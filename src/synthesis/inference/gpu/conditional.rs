use super::{
    Batch,
    batch::Kernel,
    chain::Chain,
    convolution::{Prefix, encode_convolution},
    elementwise::{self, Element},
};
use crate::synthesis::{
    inference::{Sequence, compute, convolution::Convolution, matrix::MatrixError},
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};
use std::ops::Range;
use wgpu::Buffer;

const MAX_SCRATCH_FLOATS: usize = 8 * 1024 * 1024;

struct Layer<'a> {
    main: &'a Convolution,
    condition: Option<&'a Convolution>,
    residual: &'a Convolution,
    skip: &'a Convolution,
}

pub(in super::super) struct ConditionalGraph<'a> {
    direct: Option<&'a Convolution>,
    layers: Vec<Layer<'a>>,
    channels: usize,
    radii: [usize; 2],
}

impl<'a> ConditionalGraph<'a> {
    pub(in super::super) fn prepare(
        tree: &'a PreparedTree,
        [gates, skips, residuals]: [&[usize]; 3],
        direct: Option<usize>,
        channels: usize,
    ) -> Result<Option<Self>> {
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
        for ((&gate, &skip), &residual) in gates.iter().zip(skips).zip(residuals) {
            let children = &tree.node(gate)?.children;
            layers.push(Layer {
                main: projection(children[0])?,
                condition: direct.map(|_| projection(children[1])).transpose()?,
                residual: projection(residual)?,
                skip: projection(skip)?,
            });
        }
        let mut graph = Self {
            direct,
            layers,
            channels,
            radii: [0; 2],
        };
        for op in graph.projections() {
            let config = op.config();
            // The streaming program pairs outputs by row and allows general rate
            // changes. This resident graph requires every projection to preserve
            // the complete input length and both branches to have the same extent.
            if !config
                .preserves_frames()
                .map_err(|e| VoiceError::new(e.to_string()))?
                || u32::try_from(config.dilation).is_err()
            {
                return Ok(None);
            }
        }
        // Compute the maximum dependency radius of each raw stream separately.
        // Pairing prefixes in the CPU queues is equivalent to taking the minimum
        // of each input's available length after subtracting its radius.
        let mut state = [Some(0), None];
        let mut output = direct.map_or([Some(0), None], |op| [None, Some(op.config().padding)]);
        for layer in &graph.layers {
            let mut gated = advance(state, layer.main);
            if let Some(op) = layer.condition {
                gated[1] = gated[1].max(Some(op.config().padding));
            }
            let residual = advance(state, layer.residual);
            state = merge(gated, residual);
            output = merge(output, advance(gated, layer.skip));
        }
        graph.radii = output.map(|radius| radius.unwrap_or(0));
        Ok(Some(graph))
    }

    pub(in super::super) fn radii(&self) -> [usize; 2] {
        self.radii
    }

    fn projections(&self) -> impl Iterator<Item = &Convolution> {
        self.direct
            .into_iter()
            .chain(self.layers.iter().flat_map(|layer| {
                [
                    Some(layer.main),
                    layer.condition,
                    Some(layer.residual),
                    Some(layer.skip),
                ]
                .into_iter()
                .flatten()
            }))
    }

    pub(in super::super) fn execute(
        &self,
        input: &Sequence,
        condition: Option<&Sequence>,
        output_rows: Range<usize>,
        projection: Option<(&PreparedTree, usize)>,
    ) -> Result<Option<(Sequence, bool)>> {
        let frames = input.frames;
        if output_rows.is_empty() {
            return Ok(Some((Sequence::zeros(0, self.channels)?, false)));
        }
        let mut work = 0usize;
        let mut lengths = vec![input.values.len(), condition.map_or(0, |s| s.values.len())];
        let mut capacity = 0;
        for op in self.projections() {
            let config = op.config();
            if u32::try_from(frames.saturating_add(config.padding * 2)).is_err() {
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
        if capacity > MAX_SCRATCH_FLOATS {
            return Ok(None);
        }
        lengths.push(capacity);
        let full_output_len = frames * self.channels;
        let stable = output_rows.start * self.channels..output_rows.end * self.channels;
        let projection = match projection {
            Some((tree, index)) => {
                Chain::projection(tree, index, output_rows.len(), self.channels)?
                    .filter(|chain| chain.capacity <= MAX_SCRATCH_FLOATS)
            }
            None => None,
        };
        let channels = projection
            .as_ref()
            .map_or(self.channels, |chain| chain.channels);
        let mut output = Sequence::zeros(output_rows.len(), channels)?;
        let ran = compute::accelerate(None, work, 1_048_576, &lengths, |batch| {
            let source = batch.input(&input.values);
            let cond = condition.map(|s| batch.input(&s.values));
            let chain_capacity = projection
                .as_ref()
                .map_or(capacity, |chain| capacity.max(chain.capacity));
            let [projected, residual] = std::array::from_fn(|_| batch.output(chain_capacity));
            let [aux_or_skip, gated, state] = std::array::from_fn(|_| batch.output(capacity));
            let outputs = [batch.output(full_output_len), batch.output(full_output_len)];
            let projection = projection
                .as_ref()
                .map(|chain| (chain, batch.validation_index()));
            let project = |batch: &mut Batch<'_>, op: &Convolution, from: &Buffer, to: &Buffer| {
                encode_convolution(
                    batch,
                    op,
                    from,
                    to,
                    frames,
                    frames,
                    Prefix::Zeros(op.config().padding),
                )
            };
            if let Some(op) = self.direct {
                project(batch, op, cond.as_ref().unwrap(), &outputs[0])?;
            }
            for (index, layer) in self.layers.iter().enumerate() {
                let activation = if index == 0 { &source } else { &state };
                project(batch, layer.main, activation, &projected)?;
                project(batch, layer.residual, activation, &residual)?;
                let auxiliary = if let Some(op) = layer.condition {
                    project(batch, op, cond.as_ref().unwrap(), &aux_or_skip)?;
                    Some(&aux_or_skip)
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
                project(batch, layer.skip, &gated, &aux_or_skip)?;
                add(
                    batch,
                    &gated,
                    &residual,
                    &state,
                    0..frames * width,
                    true,
                    None,
                )?;
                // Only published rows enter the pointwise projection. The final
                // skip sum also checks the boundary that used to be read on the CPU.
                let validation = projection
                    .as_ref()
                    .filter(|_| index + 1 == self.layers.len())
                    .map(|(_, validation)| validation);
                let values = if validation.is_some() {
                    stable.clone()
                } else {
                    0..full_output_len
                };
                add(
                    batch,
                    &outputs[index % 2],
                    &aux_or_skip,
                    &outputs[(index + 1) % 2],
                    values,
                    index != 0 || self.direct.is_some(),
                    validation,
                )?;
            }
            let result = &outputs[self.layers.len() % 2];
            if let Some((projection, validation)) = projection {
                let result = projection.encode(batch, result, [&projected, &residual])?;
                batch.read_checked(result, &mut output.values, &validation)
            } else {
                batch.read_from(result, stable.start, &mut output.values)
            }
        })
        .map_err(|e| VoiceError::new(e.to_string()))?;
        Ok(ran.then_some((output, projection.is_some())))
    }
}

fn add(
    batch: &mut Batch<'_>,
    left: &Buffer,
    right: &Buffer,
    output: &Buffer,
    values: Range<usize>,
    both: bool,
    validation_index: Option<&Buffer>,
) -> std::result::Result<(), MatrixError> {
    let grid = batch.grid((values.len() as u32).div_ceil(64))?;
    let mut params = [0; 16];
    params[..4].copy_from_slice(&[
        values.len() as u32,
        grid[0],
        u32::from(both),
        values.start as u32,
    ]);
    batch.dispatch(
        Kernel {
            entry: if validation_index.is_some() {
                "conditional_sum_projected"
            } else {
                "conditional_sum"
            },
            source: include_str!("shaders/conditional.wgsl"),
        },
        grid,
        params,
        [
            Some(left),
            Some(right),
            None,
            Some(output),
            validation_index,
        ],
    )
}

fn advance(radii: [Option<usize>; 2], op: &Convolution) -> [Option<usize>; 2] {
    radii.map(|radius| radius.map(|radius| radius.saturating_add(op.config().padding)))
}

fn merge(left: [Option<usize>; 2], right: [Option<usize>; 2]) -> [Option<usize>; 2] {
    [left[0].max(right[0]), left[1].max(right[1])]
}
