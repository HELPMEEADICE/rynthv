use super::{
    batch::Kernel,
    convolution::encode_convolution,
    elementwise::{self, Element},
};
use crate::synthesis::{
    inference::{Sequence, compute, convolution::Convolution},
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use wgpu::Buffer;

struct Layer<'a> {
    main: &'a Convolution,
    condition: Option<&'a Convolution>,
    residual: &'a Convolution,
    skip: &'a Convolution,
}

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn conditional(
    tree: &PreparedTree,
    input: &Sequence,
    condition: Option<&Sequence>,
    gates: &[usize],
    skips: &[usize],
    residuals: &[usize],
    direct: Option<usize>,
    channels: usize,
) -> Result<Option<Sequence>> {
    let frames = input.frames;
    if frames == 0 || !compute::has_gpu() {
        return Ok(None);
    }
    let projection = |index| -> Result<&Convolution> {
        match &tree.node(index)?.module {
            PreparedModule::Convolution(op) => Ok(op.operator()),
            _ => Err(VoiceError::new(
                "Conditional projection is not a convolution",
            )),
        }
    };
    let direct = direct.map(projection).transpose()?;
    let mut layers = Vec::with_capacity(gates.len());
    let mut work = 0usize;
    let mut lengths = vec![input.values.len(), condition.map_or(0, |s| s.values.len())];
    for ((&gate, &skip), &residual) in gates.iter().zip(skips).zip(residuals) {
        let children = &tree.node(gate)?.children;
        layers.push(Layer {
            main: projection(children[0])?,
            condition: condition.map(|_| projection(children[1])).transpose()?,
            residual: projection(residual)?,
            skip: projection(skip)?,
        });
    }
    let mut capacity = 0;
    for op in direct.into_iter().chain(layers.iter().flat_map(|layer| {
        [
            Some(layer.main),
            layer.condition,
            Some(layer.residual),
            Some(layer.skip),
        ]
        .into_iter()
        .flatten()
    })) {
        let config = op.config();
        // The streaming program pairs outputs by row and allows general rate
        // changes. This resident graph requires every projection to preserve
        // the complete input length and both branches to have the same extent.
        if config.stride != 1
            || config.padding * 2
                != config
                    .receptive_frames()
                    .map_err(|e| VoiceError::new(e.to_string()))?
                    - 1
            || u32::try_from(frames.saturating_add(config.padding * 2)).is_err()
            || u32::try_from(config.dilation).is_err()
        {
            return Ok(None);
        }
        let size = frames
            .checked_mul(op.output_channels())
            .ok_or_else(|| VoiceError::new("Conditional GPU dimensions overflow"))?;
        capacity = capacity.max(size);
        lengths.push(op.shared_weights().len());
        work = work.saturating_add(
            size.saturating_mul(op.input_channels())
                .saturating_mul(config.kernel),
        );
    }
    if capacity > 8 * 1024 * 1024 {
        return Ok(None);
    }
    lengths.push(capacity);
    let mut output = Sequence::zeros(frames, channels)?;
    let ran = compute::accelerate(None, work, 1_048_576, &lengths, |batch| {
        let source = batch.input(&input.values);
        let cond = condition.map(|s| batch.input(&s.values));
        let [projected, projected_condition, residual, gated, skip, state] =
            std::array::from_fn(|_| batch.output(capacity));
        let outputs = [
            batch.output(output.values.len()),
            batch.output(output.values.len()),
        ];
        let project =
            |batch: &mut super::Batch<'_>, op: &Convolution, from: &Buffer, to: &Buffer| {
                encode_convolution(batch, op, from, to, frames, frames, op.config().padding)
            };
        if let Some(op) = direct {
            project(batch, op, cond.as_ref().unwrap(), &outputs[0])?;
        }
        for (index, layer) in layers.iter().enumerate() {
            let activation = if index == 0 { &source } else { &state };
            project(batch, layer.main, activation, &projected)?;
            project(batch, layer.residual, activation, &residual)?;
            let auxiliary = if let Some(op) = layer.condition {
                project(batch, op, cond.as_ref().unwrap(), &projected_condition)?;
                Some(&projected_condition)
            } else {
                None
            };
            let width = layer.residual.output_channels();
            elementwise::encode(
                batch,
                Element::Gate {
                    channels: width,
                    condition: None,
                },
                &projected,
                &gated,
                frames * width,
                auxiliary,
            )?;
            project(batch, layer.skip, &gated, &skip)?;
            let add = |batch: &mut super::Batch<'_>,
                       a: &Buffer,
                       b: &Buffer,
                       to: &Buffer,
                       length: usize,
                       both| {
                let grid = batch.grid((length as u32).div_ceil(64))?;
                let mut params = [0; 16];
                params[..3].copy_from_slice(&[length as u32, grid[0], u32::from(both)]);
                batch.dispatch(
                    Kernel {
                        entry: "conditional_sum",
                        source: include_str!("shaders/conditional.wgsl"),
                    },
                    grid,
                    params,
                    [Some(a), Some(b), None, Some(to), None],
                )
            };
            add(batch, &gated, &residual, &state, frames * width, true)?;
            add(
                batch,
                &outputs[index % 2],
                &skip,
                &outputs[(index + 1) % 2],
                output.values.len(),
                index != 0 || direct.is_some(),
            )?;
        }
        batch.read(&outputs[layers.len() % 2], &mut output.values)
    })
    .map_err(|e| VoiceError::new(e.to_string()))?;
    Ok(ran.then_some(output))
}
