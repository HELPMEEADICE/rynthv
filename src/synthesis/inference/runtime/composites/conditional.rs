mod history;

use super::{ModelRuntime, queue::SumQueue};
use crate::synthesis::{
    inference::{
        Sequence, VocoderBranches, compute,
        convolution::ConvolutionStream,
        gpu::{ConditionalGraph, ConditionalProjection},
        ops::conditional::ConditionalConfig,
    },
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use history::History;
use std::{collections::HashMap, ops::Range};

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
        pending_projection: &mut Option<usize>,
        flush: bool,
    ) -> Result<Sequence> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("op.conditional");
        let tree = runtime.tree;
        if (self.history.is_some() || compute::has_gpu())
            && let Some(graph) = self.graph(tree, children, condition.is_some())?
        {
            let mut render = |input: &Sequence,
                              condition: Option<&Sequence>,
                              rows: Range<usize>|
             -> Result<Option<Sequence>> {
                let suffix = pending_projection.map(|index| ConditionalProjection {
                    tree,
                    index,
                    streams: &mut runtime.convolutions,
                    flush,
                });
                let result = graph.execute(input, condition, rows, suffix)?;
                Ok(result.map(|(output, projected)| {
                    if projected {
                        *pending_projection = None;
                    }
                    output
                }))
            };
            if flush && self.history.is_none() {
                if let Some(output) = render(input, condition, 0..input.frames)? {
                    return Ok(output);
                }
            } else {
                let mut history = self.history.clone().unwrap_or_default();
                let output = history.try_process(
                    input,
                    condition,
                    flush,
                    graph.radii(),
                    |input, condition, rows| {
                        if let Some(output) = render(input, condition, rows.clone())? {
                            return Ok(Some(output));
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
                        .map(Some)
                    },
                )?;
                self.history = (!flush).then_some(history);
                return Ok(output.expect("the conditional window always produces a result"));
            }
        }
        self.processed = true;
        self.process_queued(runtime, children, input, condition, flush)
    }

    fn graph<'a>(
        &self,
        tree: &'a PreparedTree,
        children: &[usize],
        conditioned: bool,
    ) -> Result<Option<ConditionalGraph<'a>>> {
        if self.processed {
            return Ok(None);
        }
        ConditionalGraph::prepare(
            tree,
            [&self.gates, &self.skips, &self.residuals],
            conditioned.then(|| children[3]),
            self.config.output_channels,
        )
    }

    pub(super) fn try_vocoder_branches(
        &mut self,
        tree: &PreparedTree,
        children: &[usize],
        streams: &mut HashMap<usize, ConvolutionStream>,
        input: (&Sequence, Option<&Sequence>),
        branches: &mut VocoderBranches<'_>,
        flush: bool,
    ) -> Result<Option<(Vec<Sequence>, Sequence)>> {
        let Some(graph) = self.graph(tree, children, input.1.is_some())? else {
            return Ok(None);
        };
        if flush && self.history.is_none() {
            return branches.execute(&graph, tree, streams, input, 0..input.0.frames, flush);
        }
        let history = self.history.get_or_insert_with(History::default);
        let output = history.try_process(
            input.0,
            input.1,
            flush,
            graph.radii(),
            |input, condition, rows| {
                branches.execute(&graph, tree, streams, (input, condition), rows, flush)
            },
        )?;
        if output.is_some() && flush {
            self.history = None;
        }
        Ok(output)
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
