use super::{
    Batch,
    convolution::encode_convolution,
    elementwise::{self, Element},
    linear,
};
use crate::synthesis::{
    inference::{
        Sequence, UnaryOperator, affine::Affine, compute, convolution::Convolution,
        matrix::MatrixError,
    },
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use wgpu::Buffer;

const MIN_AUTO_WORK: usize = 1_048_576;

enum Step<'a> {
    Affine(&'a Affine),
    Convolution(&'a Convolution),
    Element(Element<'a>),
}

pub(super) struct Chain<'a> {
    steps: Vec<(Step<'a>, usize)>,
    consumed: usize,
    frames: usize,
    pub(super) channels: usize,
    pub(super) capacity: usize,
    lengths: Vec<usize>,
    work: usize,
}

pub(in super::super) fn sequence(
    tree: &PreparedTree,
    children: &[usize],
    input: &Sequence,
) -> Result<Option<(usize, Sequence)>> {
    if input.frames == 0 || children.len() < 2 || !compute::has_gpu() {
        return Ok(None);
    }
    let chain = Chain::prefix(tree, children, input.frames, input.channels)?;
    if !chain.is_batch() {
        return Ok(None);
    }
    let mut output = Sequence::zeros(input.frames, chain.channels)?;
    let completed = compute::accelerate(None, chain.work, MIN_AUTO_WORK, &chain.lengths, |batch| {
        let initial = batch.input(&input.values);
        let buffers = [batch.output(chain.capacity), batch.output(chain.capacity)];
        let source = chain.encode(batch, &initial, [&buffers[0], &buffers[1]])?;
        batch.read(source, &mut output.values)
    })
    .map_err(|e| VoiceError::new(e.to_string()))?;
    Ok(completed.then_some((chain.consumed, output)))
}

impl<'a> Chain<'a> {
    fn prefix(
        tree: &'a PreparedTree,
        children: &[usize],
        frames: usize,
        mut channels: usize,
    ) -> Result<Self> {
        let mut lengths = vec![sequence_len(frames, channels)?];
        let mut work = 0usize;
        let mut steps = Vec::new();
        let mut capacity = 0;
        let mut consumed = 0;
        for &child in children {
            let step = match &tree.node(child)?.module {
                PreparedModule::Affine(op) if op.input_features == channels => {
                    channels = op.output_features;
                    let weights = op.operator().matrix().values().len();
                    work = work.saturating_add(frames.saturating_mul(weights));
                    lengths.push(weights);
                    Step::Affine(op.operator())
                }
                PreparedModule::Convolution(op)
                    if op.input_features == channels
                        && op.config.kernel == 1
                        && op.config.stride == 1
                        && op.config.padding == 0
                        && u32::try_from(op.config.dilation).is_ok() =>
                {
                    channels = op.output_features;
                    let operator = op.operator();
                    let weights = operator.shared_weights().len();
                    work = work.saturating_add(frames.saturating_mul(weights));
                    lengths.push(weights);
                    Step::Convolution(operator)
                }
                PreparedModule::Unary(UnaryOperator::Identity) => {
                    consumed += 1;
                    continue;
                }
                PreparedModule::Unary(op) => Step::Element(Element::Unary(*op)),
                PreparedModule::LeakyRelu(op) => Step::Element(Element::LeakyRelu(op.slope())),
                PreparedModule::ChannelAffine(op) if op.channels() == channels => {
                    Step::Element(op.gpu_element())
                }
                PreparedModule::LayerNorm(op) if op.channels() == channels => {
                    Step::Element(op.gpu_element())
                }
                _ => break,
            };
            if let Step::Element(op) = step {
                lengths.extend(op.lengths());
            }
            let size = sequence_len(frames, channels)?;
            capacity = capacity.max(size);
            steps.push((step, size));
            consumed += 1;
        }
        lengths.push(capacity);
        Ok(Self {
            steps,
            consumed,
            frames,
            channels,
            capacity,
            lengths,
            work,
        })
    }

    fn is_batch(&self) -> bool {
        self.steps.len() >= 2 && self.work != 0
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
        let chain = Self::prefix(tree, &node.children, frames, channels)?;
        if chain.consumed != node.children.len() || !chain.is_batch() {
            return Ok(None);
        }
        Ok(compute::can_append_gpu(chain.work, MIN_AUTO_WORK, &chain.lengths).then_some(chain))
    }

    pub(super) fn encode<'buf>(
        &self,
        batch: &mut Batch<'_>,
        mut source: &'buf Buffer,
        buffers: [&'buf Buffer; 2],
    ) -> std::result::Result<&'buf Buffer, MatrixError> {
        for (index, (step, size)) in self.steps.iter().enumerate() {
            let target = buffers[index % 2];
            match step {
                Step::Affine(op) => linear::encode_affine(batch, op, self.frames, source, target)?,
                Step::Convolution(op) => {
                    encode_convolution(batch, op, source, target, self.frames, self.frames, 0)?
                }
                Step::Element(op) => elementwise::encode(batch, *op, source, target, *size, None)?,
            }
            source = target;
        }
        Ok(source)
    }
}

fn sequence_len(frames: usize, channels: usize) -> Result<usize> {
    frames
        .checked_mul(channels)
        .ok_or_else(|| VoiceError::new("GPU sequence dimensions overflow"))
}
