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
    Convolution(&'a Convolution, Option<StreamStep>),
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
    frames: usize,
    input_frames: usize,
    pub(super) channels: usize,
    pub(super) capacity: usize,
    lengths: Vec<usize>,
    work: usize,
    history_len: usize,
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
        Some((streams, flush)),
    )?;
    if !chain.is_batch() {
        return Ok(None);
    }
    let validate = chain.steps[..chain.steps.len() - 1]
        .iter()
        .any(Step::validate);
    let output_len = sequence_len(chain.frames, chain.channels)?;
    if validate || chain.history_len != 0 {
        chain.lengths.push(
            output_len
                .checked_add(chain.history_len)
                .and_then(|len| len.checked_add(2 + (Timestamps::MAX_BYTES / 4) as usize))
                .ok_or_else(|| VoiceError::new("GPU sequence readback size overflows"))?,
        );
    }
    let mut result = None;
    compute::accelerate(None, chain.work, MIN_AUTO_WORK, &chain.lengths, |batch| {
        let mut output = vec![0.0; output_len];
        let mut history = vec![0.0; chain.history_len];
        let initial = batch.input(&input.values);
        let buffers = [batch.output(chain.capacity), batch.output(chain.capacity)];
        let validation = validate.then(|| batch.validation_index());
        let staging = batch.stage_readback(
            output_len,
            history.len(),
            validate.then_some("in a sequential convolution"),
            chain.steps.len().saturating_mul(2),
        );
        let source = chain.encode_steps(
            batch,
            &initial,
            [&buffers[0], &buffers[1]],
            Some((&staging, validation.as_ref())),
        )?;
        batch.read_staged(
            staging,
            source,
            0,
            &mut output,
            &mut history,
            validation.as_ref(),
        )?;
        result = Some((output, history));
        Ok(())
    })
    .map_err(|e| VoiceError::new(e.to_string()))?;
    let Some((output, history)) = result else {
        return Ok(None);
    };
    let output = Sequence::new(chain.frames, chain.channels, output)?;
    for step in chain.steps {
        if let Operation::Convolution(_, Some(stream)) = step.operation {
            stream.commit(streams, &input.values, &history);
        }
    }
    Ok(Some((chain.consumed, output)))
}

impl<'a> Chain<'a> {
    fn prefix(
        tree: &'a PreparedTree,
        children: &[usize],
        input_frames: usize,
        mut channels: usize,
        stream: Option<(&HashMap<usize, ConvolutionStream>, bool)>,
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
                        && (stream.is_some() || op.config.padding == 0)
                        && op
                            .config
                            .preserves_frames()
                            .map_err(|e| VoiceError::new(e.to_string()))?
                        && u32::try_from(op.config.dilation).is_ok() =>
                {
                    let operator = op.operator();
                    let convolution = if let Some((streams, flush)) = stream {
                        // A repeated node consumes a second chunk of the same state.
                        if !convolutions.insert(child) {
                            break;
                        }
                        let state = StreamStep::plan(
                            child,
                            operator,
                            streams.get(&child),
                            sequence_len(frames, channels)?,
                            flush,
                            (!steps.is_empty()).then_some(history_len),
                        )?;
                        if u32::try_from(state.chunk.frames.saturating_add(op.config.padding * 2))
                            .is_err()
                        {
                            break;
                        }
                        if let Some(history) = &state.history {
                            history_len = history.end;
                        }
                        lengths.push(state.state.pending().len());
                        frames = state.chunk.frames;
                        Some(state)
                    } else {
                        None
                    };
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
    ) -> Result<Option<Self>> {
        let node = tree.node(index)?;
        if frames == 0 || !matches!(node.module, PreparedModule::Sequential) {
            return Ok(None);
        }
        let chain = Self::prefix(tree, &node.children, frames, channels, None)?;
        if chain.consumed != node.children.len() || !chain.is_batch() {
            return Ok(None);
        }
        Ok(compute::can_append_gpu(chain.work, MIN_AUTO_WORK, &chain.lengths).then_some(chain))
    }

    pub(super) fn encode<'buf>(
        &self,
        batch: &mut Batch<'_>,
        source: &'buf Buffer,
        buffers: [&'buf Buffer; 2],
    ) -> std::result::Result<&'buf Buffer, MatrixError> {
        self.encode_steps(batch, source, buffers, None)
    }

    fn encode_steps<'buf>(
        &self,
        batch: &mut Batch<'_>,
        mut source: &'buf Buffer,
        buffers: [&'buf Buffer; 2],
        readback: Option<(&Readback, Option<&Buffer>)>,
    ) -> std::result::Result<&'buf Buffer, MatrixError> {
        let mut input_frames = self.input_frames;
        for (index, step) in self.steps.iter().enumerate() {
            let target = buffers[index % 2];
            match &step.operation {
                Operation::Affine(op) if step.frames != 0 => {
                    linear::encode_affine(batch, op, step.frames, source, target)?;
                }
                Operation::Convolution(op, stream) => {
                    if let Some(stream) = stream
                        && let Some((staging, _)) = readback
                        && let Some(history) = &stream.history
                    {
                        staging.copy_history(
                            batch,
                            source,
                            stream.chunk.input_tail.clone(),
                            history.start,
                        );
                    }
                    if step.frames != 0 {
                        let pending = stream
                            .as_ref()
                            .filter(|stream| !stream.state.pending().is_empty())
                            .map(|stream| {
                                (
                                    batch.input(stream.state.pending()),
                                    stream.state.pending_frames(),
                                )
                            });
                        let prefix = match &pending {
                            Some((buffer, frames)) => Prefix::History {
                                buffer,
                                frames: *frames,
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
                            && let Some((_, Some(validation))) = readback
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
            && matches!(self.operation, Operation::Convolution(op, Some(_)) if op.config().padding != 0)
    }
}

fn sequence_len(frames: usize, channels: usize) -> Result<usize> {
    frames
        .checked_mul(channels)
        .ok_or_else(|| VoiceError::new("GPU sequence dimensions overflow"))
}
