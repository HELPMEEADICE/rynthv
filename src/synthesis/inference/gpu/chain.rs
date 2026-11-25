mod stream;

use super::{
    Batch,
    batch::Readback,
    convolution::{Prefix, encode_convolution},
    elementwise::{self, Element},
    linear,
    timestamps::Timestamps,
};
use crate::synthesis::{
    inference::{
        Sequence, UnaryOperator,
        affine::Affine,
        compute,
        convolution::{Convolution, ConvolutionStream},
        matrix::MatrixError,
    },
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use std::collections::{HashMap, HashSet};
use stream::StreamStep;
use wgpu::Buffer;

const MIN_AUTO_WORK: usize = 1_048_576;

enum Operation<'a> {
    Affine(&'a Affine),
    Convolution(&'a Convolution, StreamStep),
    Element(Element<'a>),
}

struct Step<'a> {
    operation: Operation<'a>,
    frames: usize,
    len: usize,
}

pub(super) struct Chain<'a> {
    steps: Vec<Step<'a>>,
    consumed: usize,
    pub(super) frames: usize,
    input_frames: usize,
    pub(super) channels: usize,
    pub(super) capacity: usize,
    lengths: Vec<usize>,
    work: usize,
    pub(super) history_len: usize,
}

pub(in super::super) fn sequence(
    tree: &PreparedTree,
    children: &[usize],
    input: &Sequence,
    streams: &mut HashMap<usize, ConvolutionStream>,
    flush: bool,
) -> Result<Option<(usize, Sequence)>> {
    if children.len() < 2 || !compute::has_gpu() {
        return Ok(None);
    }
    let mut chain = Chain::prefix(
        tree,
        children,
        input.frames,
        input.channels,
        streams,
        flush,
        false,
    )?;
    if !chain.is_batch() {
        return Ok(None);
    }
    let validate = chain.validates_intermediate();
    let output_len = sequence_len(chain.frames, chain.channels)?;
    chain.include_readback(validate)?;
    let mut result = None;
    compute::accelerate(None, chain.work, MIN_AUTO_WORK, &chain.lengths, |batch| {
        let mut output = vec![0.0; output_len];
        let mut history = vec![0.0; chain.history_len];
        let initial = batch.input(&input.values);
        let buffers = [batch.output(chain.capacity), batch.output(chain.capacity)];
        let validation = validate.then(|| batch.validation_index());
        chain.read(
            batch,
            &initial,
            [&buffers[0], &buffers[1]],
            &mut output,
            &mut history,
            validation
                .as_ref()
                .map(|buffer| (buffer, "in a sequential convolution")),
        )?;
        result = Some((output, history));
        Ok(())
    })
    .map_err(|e| VoiceError::new(e.to_string()))?;
    let Some((output, history)) = result else {
        return Ok(None);
    };
    let output = Sequence::new(chain.frames, chain.channels, output)?;
    let consumed = chain.consumed;
    chain.commit(streams, &input.values, &history);
    Ok(Some((consumed, output)))
}

impl<'a> Chain<'a> {
    fn prefix(
        tree: &'a PreparedTree,
        children: &[usize],
        input_frames: usize,
        mut channels: usize,
        streams: &HashMap<usize, ConvolutionStream>,
        flush: bool,
        device_input: bool,
    ) -> Result<Self> {
        let mut frames = input_frames;
        let mut lengths = vec![sequence_len(frames, channels)?];
        let mut work = 0usize;
        let mut steps = Vec::new();
        let mut capacity = 0;
        let mut consumed = 0;
        let mut history_len = 0;
        let mut convolutions = HashSet::new();
        for &child in children {
            let operation = match &tree.node(child)?.module {
                PreparedModule::Affine(op) if op.input_features == channels => {
                    channels = op.output_features;
                    let weights = op.operator().matrix().values().len();
                    work = work.saturating_add(frames.saturating_mul(weights));
                    lengths.push(weights);
                    Operation::Affine(op.operator())
                }
                PreparedModule::Convolution(op)
                    if op.input_features == channels
                        && op
                            .config
                            .preserves_frames()
                            .map_err(|e| VoiceError::new(e.to_string()))?
                        && u32::try_from(op.config.dilation).is_ok() =>
                {
                    let operator = op.operator();
                    // A repeated node consumes a second chunk of the same state.
                    if !convolutions.insert(child) {
                        break;
                    }
                    let convolution = StreamStep::plan(
                        child,
                        operator,
                        streams.get(&child),
                        sequence_len(frames, channels)?,
                        flush,
                        (device_input || !steps.is_empty()).then_some(history_len),
                    )?;
                    if u32::try_from(
                        convolution
                            .chunk
                            .frames
                            .saturating_add(op.config.padding * 2),
                    )
                    .is_err()
                    {
                        break;
                    }
                    if let Some(history) = &convolution.history {
                        history_len = history.end;
                    }
                    lengths.push(convolution.state.pending().len());
                    frames = convolution.chunk.frames;
                    channels = op.output_features;
                    let weights = operator.shared_weights().len();
                    work = work.saturating_add(frames.saturating_mul(weights));
                    lengths.push(weights);
                    Operation::Convolution(operator, convolution)
                }
                PreparedModule::Unary(UnaryOperator::Identity) => {
                    consumed += 1;
                    continue;
                }
                PreparedModule::Unary(op) => Operation::Element(Element::Unary(*op)),
                PreparedModule::LeakyRelu(op) => Operation::Element(Element::LeakyRelu(op.slope())),
                PreparedModule::ChannelAffine(op) if op.channels() == channels => {
                    Operation::Element(op.gpu_element())
                }
                PreparedModule::LayerNorm(op) if op.channels() == channels => {
                    Operation::Element(op.gpu_element())
                }
                _ => break,
            };
            if let Operation::Element(op) = operation {
                lengths.extend(op.lengths());
            }
            let len = sequence_len(frames, channels)?;
            capacity = capacity.max(len);
            steps.push(Step {
                operation,
                frames,
                len,
            });
            consumed += 1;
        }
        lengths.push(capacity);
        Ok(Self {
            steps,
            consumed,
            frames,
            input_frames,
            channels,
            capacity,
            lengths,
            work,
            history_len,
        })
    }

    fn is_batch(&self) -> bool {
        self.steps.len() >= 2 && self.work != 0 && self.frames != 0
    }

    pub(super) fn projection(
        tree: &'a PreparedTree,
        index: usize,
        frames: usize,
        channels: usize,
        streams: &HashMap<usize, ConvolutionStream>,
        flush: bool,
    ) -> Result<Option<Self>> {
        let mut nodes = Vec::new();
        projection_nodes(tree, index, &mut nodes)?;
        let mut chain = Self::prefix(tree, &nodes, frames, channels, streams, flush, true)?;
        if chain.consumed != nodes.len() || chain.steps.is_empty() {
            return Ok(None);
        }
        chain.include_readback(true)?;
        Ok(compute::can_append_gpu(&chain.lengths).then_some(chain))
    }

    fn include_readback(&mut self, validate: bool) -> Result<()> {
        if validate || self.history_len != 0 {
            self.lengths.push(
                sequence_len(self.frames, self.channels)?
                    .checked_add(self.history_len)
                    .and_then(|len| len.checked_add(2 + (Timestamps::MAX_BYTES / 4) as usize))
                    .ok_or_else(|| VoiceError::new("GPU sequence readback size overflows"))?,
            );
        }
        Ok(())
    }

    pub(super) fn validates_intermediate(&self) -> bool {
        self.steps.iter().rev().skip(1).any(Step::validate)
    }

    pub(super) fn read(
        &self,
        batch: &mut Batch<'_>,
        source: &Buffer,
        buffers: [&Buffer; 2],
        output: &mut [f32],
        history: &mut [f32],
        validation: Option<(&Buffer, &'static str)>,
    ) -> std::result::Result<(), MatrixError> {
        let staging = batch.stage_readback(
            output.len(),
            history.len(),
            validation.map(|(_, label)| label),
            self.steps.len().saturating_mul(2),
        );
        let validation = validation.map(|(buffer, _)| buffer);
        let source = self.encode_steps(batch, source, buffers, &staging, validation)?;
        batch.read_staged(staging, source, 0, output, history, validation)
    }

    pub(super) fn commit(
        self,
        streams: &mut HashMap<usize, ConvolutionStream>,
        input: &[f32],
        history: &[f32],
    ) {
        for step in self.steps {
            if let Operation::Convolution(_, stream) = step.operation {
                stream.commit(streams, input, history);
            }
        }
    }

    fn encode_steps<'buf>(
        &self,
        batch: &mut Batch<'_>,
        mut source: &'buf Buffer,
        buffers: [&'buf Buffer; 2],
        staging: &Readback,
        validation: Option<&Buffer>,
    ) -> std::result::Result<&'buf Buffer, MatrixError> {
        let mut input_frames = self.input_frames;
        for (index, step) in self.steps.iter().enumerate() {
            let target = buffers[index % 2];
            match &step.operation {
                Operation::Affine(op) if step.frames != 0 => {
                    linear::encode_affine(batch, op, step.frames, source, target)?;
                }
                Operation::Convolution(op, stream) => {
                    if let Some(history) = &stream.history {
                        staging.copy_history(
                            batch,
                            source,
                            stream.chunk.input_tail.clone(),
                            history.start,
                        );
                    }
                    if step.frames != 0 {
                        let pending = (!stream.state.pending().is_empty())
                            .then(|| batch.input(stream.state.pending()));
                        let prefix = match pending.as_ref() {
                            Some(buffer) => Prefix::History {
                                buffer,
                                frames: stream.state.pending_frames(),
                            },
                            None => Prefix::Zeros(0),
                        };
                        encode_convolution(
                            batch,
                            op,
                            source,
                            target,
                            input_frames,
                            step.frames,
                            prefix,
                        )?;
                        if index + 1 < self.steps.len()
                            && step.validate()
                            && let Some(validation) = validation
                        {
                            elementwise::validate_finite(batch, target, step.len, validation)?;
                        }
                    }
                }
                Operation::Element(op) if step.frames != 0 => {
                    elementwise::encode(batch, *op, source, target, step.len, None)?;
                }
                _ => {}
            }
            source = target;
            input_frames = step.frames;
        }
        Ok(source)
    }
}

impl Step<'_> {
    fn validate(&self) -> bool {
        self.frames != 0
            && matches!(self.operation, Operation::Convolution(op, _) if op.config().padding != 0)
    }
}

fn projection_nodes(tree: &PreparedTree, index: usize, nodes: &mut Vec<usize>) -> Result<()> {
    let node = tree.node(index)?;
    if matches!(node.module, PreparedModule::Sequential) {
        for &child in &node.children {
            projection_nodes(tree, child, nodes)?;
        }
    } else {
        nodes.push(index);
    }
    Ok(())
}

fn sequence_len(frames: usize, channels: usize) -> Result<usize> {
    frames
        .checked_mul(channels)
        .ok_or_else(|| VoiceError::new("GPU sequence dimensions overflow"))
}
