use super::{
    convolution::encode_convolution,
    elementwise::{self, Element},
    linear,
};
use crate::synthesis::{
    inference::{Sequence, UnaryOperator, affine::Affine, compute, convolution::Convolution},
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};

enum Step<'a> {
    Affine(&'a Affine),
    Convolution(&'a Convolution),
    Element(Element<'a>),
}

pub(in super::super) fn sequence(
    tree: &PreparedTree,
    children: &[usize],
    input: &Sequence,
) -> Result<Option<(usize, Sequence)>> {
    if input.frames == 0 || children.len() < 2 || !compute::has_gpu() {
        return Ok(None);
    }
    let mut channels = input.channels;
    let mut work = 0usize;
    let mut lengths = vec![input.values.len()];
    let mut steps = Vec::new();
    let mut capacity = 0;
    let mut consumed = 0;
    for &child in children {
        let step = match &tree.node(child)?.module {
            PreparedModule::Affine(op) if op.input_features == channels => {
                channels = op.output_features;
                work = work.saturating_add(
                    input
                        .frames
                        .saturating_mul(op.operator().matrix().values().len()),
                );
                lengths.push(op.operator().matrix().values().len());
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
                work = work.saturating_add(input.frames.saturating_mul(weights));
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
        let size = input
            .frames
            .checked_mul(channels)
            .ok_or_else(|| VoiceError::new("GPU sequence dimensions overflow"))?;
        capacity = capacity.max(size);
        steps.push((step, size));
        consumed += 1;
    }
    if steps.len() < 2 || work == 0 {
        return Ok(None);
    }
    lengths.push(capacity);
    let mut output = Sequence::zeros(input.frames, channels)?;
    let completed = compute::accelerate(None, work, 1_048_576, &lengths, |batch| {
        let initial = batch.input(&input.values);
        let buffers = [batch.output(capacity), batch.output(capacity)];
        let mut source = &initial;
        for (index, &(ref step, size)) in steps.iter().enumerate() {
            let target = &buffers[index % 2];
            match step {
                Step::Affine(op) => linear::encode_affine(batch, op, input.frames, source, target)?,
                Step::Convolution(op) => {
                    encode_convolution(batch, op, source, target, input.frames, input.frames, 0)?
                }
                Step::Element(op) => elementwise::encode(batch, *op, source, target, size, None)?,
            }
            source = target;
        }
        batch.read(source, &mut output.values)
    })
    .map_err(|e| VoiceError::new(e.to_string()))?;
    Ok(completed.then_some((consumed, output)))
}
