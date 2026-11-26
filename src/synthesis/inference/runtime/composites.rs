//! Persistent programs for composite operators with independently ready branches.

mod conditional;
mod gated;
mod queue;
mod sampler;

use super::ModelRuntime;
use crate::synthesis::{
    inference::{Sequence, VocoderBranches, compute},
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};

#[derive(Clone)]
pub(super) struct CompositeState {
    program: Program,
    input_channels: usize,
    condition_channels: Option<usize>,
    input_frames: usize,
    condition_frames: usize,
}

#[derive(Clone)]
enum Program {
    Gated(gated::GatedState),
    Conditional(conditional::ConditionalState),
    Sampler(sampler::SamplerState),
}

impl CompositeState {
    fn new(
        tree: &PreparedTree,
        index: usize,
        input: &Sequence,
        condition: Option<&Sequence>,
    ) -> Result<Self> {
        let node = tree.node(index)?;
        let program = match node.module {
            PreparedModule::Gated(config) => {
                check_inputs(
                    input,
                    condition,
                    config.input_channels,
                    config.condition_channels,
                )?;
                Program::Gated(gated::GatedState::new(config, condition.is_some())?)
            }
            PreparedModule::Conditional(config) => {
                check_inputs(
                    input,
                    condition,
                    config.input_channels,
                    config.condition_channels,
                )?;
                Program::Conditional(conditional::ConditionalState::new(
                    tree,
                    &node.children,
                    config,
                )?)
            }
            PreparedModule::SamplerNetwork => {
                let condition = condition.ok_or_else(|| {
                    VoiceError::new("Sampler network requires a condition sequence")
                })?;
                if tree.payload(index)? != [4, 0, 0, 1, 0, 1, 0, 1, 0, 1]
                    || node.children.len() != 5
                {
                    return Err(VoiceError::new("Unsupported sampler network configuration"));
                }
                Program::Sampler(sampler::SamplerState::new(
                    input.channels,
                    condition.channels,
                )?)
            }
            _ => return Err(VoiceError::new("Expected a recovered composite operator")),
        };
        Ok(Self {
            program,
            input_channels: input.channels,
            condition_channels: condition.map(|s| s.channels),
            input_frames: 0,
            condition_frames: 0,
        })
    }

    fn accept(
        &mut self,
        input: &Sequence,
        condition: Option<&Sequence>,
        flush: bool,
    ) -> Result<()> {
        if input.channels != self.input_channels
            || condition.map(|sequence| sequence.channels) != self.condition_channels
        {
            return Err(VoiceError::new(
                "Composite input shape or condition presence changed",
            ));
        }
        self.input_frames = self
            .input_frames
            .checked_add(input.frames)
            .ok_or_else(|| VoiceError::new("Composite input frame count overflows"))?;
        self.condition_frames = self
            .condition_frames
            .checked_add(condition.map_or(0, |s| s.frames))
            .ok_or_else(|| VoiceError::new("Composite condition frame count overflows"))?;
        if flush && condition.is_some() && self.input_frames != self.condition_frames {
            return Err(VoiceError::new(
                "Final composite input and condition lengths differ",
            ));
        }
        Ok(())
    }
}

impl ModelRuntime<'_> {
    pub(super) fn composite(
        &mut self,
        index: usize,
        input: &Sequence,
        condition: Option<&Sequence>,
        mut projection: Option<usize>,
        flush: bool,
    ) -> Result<Sequence> {
        if self.closed.contains(&index) {
            return Err(VoiceError::new(
                "Composite stream is finalized; reset before reuse",
            ));
        }
        input.validate()?;
        if let Some(condition) = condition {
            condition.validate()?;
        }
        let tree = self.tree;
        let node = tree.node(index)?;
        // Taking the parent state out permits recursive child execution while all
        // child queues remain resident in this same runtime.
        let mut state = match self.composites.remove(&index) {
            Some(state) => state,
            None => CompositeState::new(tree, index, input, condition)?,
        };
        let output = (|| {
            state.accept(input, condition, flush)?;
            match &mut state.program {
                Program::Gated(program) => {
                    program.process(self, &node.children, input, condition, flush)
                }
                Program::Conditional(program) => program.process(
                    self,
                    &node.children,
                    input,
                    condition,
                    &mut projection,
                    flush,
                ),
                Program::Sampler(program) => program.process(
                    self,
                    &node.children,
                    input,
                    condition.ok_or_else(|| VoiceError::new("Missing sampler condition"))?,
                    flush,
                ),
            }
        })();
        if output.is_ok() && flush {
            self.closed.insert(index);
        } else {
            self.composites.insert(index, state);
        }
        let output = output?;
        if let Some(projection) = projection {
            self.execute(projection, &output, flush)
        } else {
            Ok(output)
        }
    }

    pub(crate) fn try_vocoder_branches(
        &mut self,
        index: usize,
        input: &Sequence,
        condition: Option<&Sequence>,
        branches: &mut VocoderBranches<'_>,
        flush: bool,
    ) -> Result<Option<(Vec<Sequence>, Sequence)>> {
        if !compute::has_gpu() || self.closed.contains(&index) {
            return Ok(None);
        }
        crate::synthesis::execution::checkpoint()?;
        input.validate()?;
        if let Some(condition) = condition {
            condition.validate()?;
        }
        let tree = self.tree;
        let node = tree.node(index)?;
        let mut state = match self.composites.get(&index) {
            Some(state) => state.clone(),
            None => CompositeState::new(tree, index, input, condition)?,
        };
        state.accept(input, condition, flush)?;
        let Program::Conditional(program) = &mut state.program else {
            return Ok(None);
        };
        let output = program.try_vocoder_branches(
            tree,
            &node.children,
            &mut self.convolutions,
            (input, condition),
            branches,
            flush,
        )?;
        if output.is_some() {
            if flush {
                self.composites.remove(&index);
                self.closed.insert(index);
            } else {
                self.composites.insert(index, state);
            }
        }
        Ok(output)
    }
}

fn check_inputs(
    input: &Sequence,
    condition: Option<&Sequence>,
    channels: usize,
    condition_channels: usize,
) -> Result<()> {
    if input.channels != channels {
        return Err(VoiceError::new(
            "Composite input has an invalid channel count",
        ));
    }
    if let Some(condition) = condition
        && (condition_channels == 0 || condition.channels != condition_channels)
    {
        return Err(VoiceError::new(
            "Composite condition has an invalid channel count",
        ));
    }
    Ok(())
}
