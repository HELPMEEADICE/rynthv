mod history;

use super::{ModelRuntime, queue::SumQueue};
use crate::synthesis::{
    inference::{Sequence, compute, gpu::ConditionalGraph, ops::conditional::ConditionalConfig},
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use history::History;

#[derive(Clone, Default)]
struct LayerState {
    residual: SumQueue,
    skip: SumQueue,
}

#[derive(Clone)]
pub(super) struct ConditionalState {
    config: ConditionalConfig,
    gates: Vec<usize>,
    skips: Vec<usize>,
    residuals: Vec<usize>,
    layers: Vec<LayerState>,
    processed: bool,
    history: Option<History>,
}

impl ConditionalState {
    pub(super) fn new(
        tree: &PreparedTree,
        children: &[usize],
        config: ConditionalConfig,
    ) -> Result<Self> {
        let expected = if config.condition_channels == 0 { 3 } else { 4 };
        if children.len() != expected {
            return Err(VoiceError::new(
                "Conditional stack has an invalid child count",
            ));
        }
        let mut lists = Vec::with_capacity(3);
        for &child in &children[..3] {
            let node = tree.node(child)?;
            if !matches!(node.module, PreparedModule::ModuleList) {
                return Err(VoiceError::new(
                    "Conditional branch is not an ownership list",
                ));
            }
            lists.push(node.children.clone());
        }
        let layers = lists[0].len();
        if layers == 0 || lists.iter().any(|list| list.len() != layers) {
            return Err(VoiceError::new(
                "Conditional branch lists have invalid lengths",
            ));
        }
        let mut lists = lists.into_iter();
        Ok(Self {
            config,
            gates: lists.next().expect("three lists"),
            skips: lists.next().expect("three lists"),
            residuals: lists.next().expect("three lists"),
            layers: vec![LayerState::default(); layers],
            processed: false,
            history: None,
        })
    }

    pub(super) fn process(
        &mut self,
        runtime: &mut ModelRuntime<'_>,
        children: &[usize],
        input: &Sequence,
        condition: Option<&Sequence>,
        flush: bool,
    ) -> Result<Sequence> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("op.conditional");
        let tree = runtime.tree;
        if (self.history.is_some() || !self.processed && compute::has_gpu())
            && let Some(graph) = ConditionalGraph::prepare(
                tree,
                [&self.gates, &self.skips, &self.residuals],
                condition.map(|_| children[3]),
                self.config.output_channels,
            )?
        {
            if flush && self.history.is_none() {
                if let Some(output) = graph.execute(input, condition, 0..input.frames)? {
                    return Ok(output);
                }
            } else {
                let mut history = self.history.clone().unwrap_or_default();
                let output = history.process(
                    input,
                    condition,
                    flush,
                    graph.radii(),
                    self.config.output_channels,
                    |input, condition, rows| {
                        if let Some(output) = graph.execute(input, condition, rows.clone())? {
                            return Ok(output);
                        }
                        // Replaying a bounded context is also valid on the CPU.
                        // The original stream queues are never mixed with halo state.
                        compute::record_cpu(
                            "conditional",
                            "GPU window below threshold or exceeds device budget",
                        );
                        let mut cpu = Self::new(tree, children, self.config)?;
                        let mut runtime = ModelRuntime::new(tree);
                        compute::scope(None, || {
                            cpu.process_queued(&mut runtime, children, input, condition, true)
                        })?
                        .slice_frames(rows)
                    },
                )?;
                self.history = (!flush).then_some(history);
                return Ok(output);
            }
        }
        self.processed = true;
        self.process_queued(runtime, children, input, condition, flush)
    }

    fn process_queued(
        &mut self,
        runtime: &mut ModelRuntime<'_>,
        children: &[usize],
        input: &Sequence,
        condition: Option<&Sequence>,
        flush: bool,
    ) -> Result<Sequence> {
        let mut output = match condition {
            Some(condition) => runtime.execute(children[3], condition, flush)?,
            None => Sequence::zeros(input.frames, self.config.output_channels)?,
        };
        if output.channels != self.config.output_channels {
            return Err(VoiceError::new(
                "Conditional direct projection has an invalid width",
            ));
        }
        let mut state = input.clone();
        for (index, layer) in self.layers.iter_mut().enumerate() {
            let residual = runtime.execute(self.residuals[index], &state, flush)?;
            let gated = match condition {
                Some(condition) => {
                    runtime.execute_conditioned(self.gates[index], &state, condition, flush)?
                }
                None => runtime.execute(self.gates[index], &state, flush)?,
            };
            // Skip always sees the gate before its residual addition.
            // Each sum releases only a matched prefix.
            let skip = runtime.execute(self.skips[index], &gated, flush)?;
            state = layer
                .residual
                .process(&gated, &residual, flush, "conditional residual")?;
            output = layer
                .skip
                .process(&output, &skip, flush, "conditional skip")?;
        }
        Ok(output)
    }
}
