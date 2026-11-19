use super::{Batch, batch::Kernel};
use crate::synthesis::inference::{UnaryOperator, compute, matrix::MatrixError};
use std::sync::Arc;
use wgpu::Buffer;

const SOURCE: &str = include_str!("shaders/elementwise.wgsl");

#[derive(Clone, Copy)]
pub(in super::super) enum Element<'a> {
    Unary(UnaryOperator),
    LeakyRelu(f32),
    ChannelAffine {
        gain: &'a Arc<[f32]>,
        bias: &'a Arc<[f32]>,
    },
    LayerNorm {
        channels: usize,
        epsilon: f32,
        gain: Option<&'a Arc<[f32]>>,
        bias: Option<&'a Arc<[f32]>>,
    },
    Gate {
        channels: usize,
        condition: Option<&'a [f32]>,
    },
    Embedding {
        table: &'a Arc<[f32]>,
        channels: usize,
    },
}

impl Element<'_> {
    fn kernel(self) -> Kernel {
        let entry = match self {
            Self::Unary(_) | Self::LeakyRelu(_) => "activation",
            Self::ChannelAffine { .. } => "channel_affine",
            Self::LayerNorm { .. } => "layer_norm",
            Self::Gate { .. } => "gated_activation",
            Self::Embedding { .. } => "embedding",
        };
        Kernel {
            entry,
            source: SOURCE,
        }
    }

    pub(super) fn lengths(self) -> [usize; 2] {
        let length = |weight: Option<&Arc<[f32]>>| weight.map_or(0, |w| w.len());
        match self {
            Self::LayerNorm { gain, bias, .. } => [length(gain), length(bias)],
            Self::ChannelAffine { gain, bias } => [gain.len(), bias.len()],
            Self::Gate { condition, .. } => [condition.map_or(0, <[f32]>::len), 0],
            Self::Embedding { table, .. } => [table.len(), 0],
            _ => [0; 2],
        }
    }
}

pub(in super::super) fn elementwise(
    op: Element<'_>,
    input: &[f32],
    output: &mut [f32],
) -> Result<bool, MatrixError> {
    if output.is_empty() {
        return Ok(false);
    }
    let [a, b] = op.lengths();
    compute::accelerate(
        op.kernel().entry,
        output.len(),
        131_072,
        &[input.len(), output.len(), a, b],
        |batch| {
            let input = batch.input(input);
            let result = batch.output(output.len());
            encode(batch, op, &input, &result, output.len(), None)?;
            batch.read(&result, output)
        },
    )
}

pub(super) fn encode(
    batch: &mut Batch<'_>,
    op: Element<'_>,
    input: &Buffer,
    result: &Buffer,
    len: usize,
    auxiliary: Option<&Buffer>,
) -> Result<(), MatrixError> {
    let mut params = [0; 16];
    params[0] = len as u32;
    let mut a = None;
    let mut b = None;
    let mut work = len;
    match op {
        Element::Unary(op) => {
            params[2] = match op {
                UnaryOperator::Identity => 0,
                UnaryOperator::Relu => 1,
                UnaryOperator::Tanh => 2,
                UnaryOperator::Sigmoid => 3,
                UnaryOperator::Silu => 4,
            }
        }
        Element::LeakyRelu(slope) => {
            params[2] = 5;
            params[3] = slope.to_bits();
        }
        Element::ChannelAffine { gain, bias } => {
            params[2] = gain.len() as u32;
            a = Some(batch.weight(gain));
            b = Some(batch.weight(bias));
        }
        Element::LayerNorm {
            channels,
            epsilon,
            gain,
            bias,
        } => {
            params[2] = channels as u32;
            params[3] = epsilon.to_bits();
            params[4] = u32::from(gain.is_some());
            params[5] = u32::from(bias.is_some());
            a = Some(match gain {
                Some(weight) => batch.weight(weight),
                None => batch.input(&[]),
            });
            b = Some(match bias {
                Some(weight) => batch.weight(weight),
                None => batch.input(&[]),
            });
            work /= channels;
        }
        Element::Gate {
            channels,
            condition,
        } => {
            params[2] = channels as u32;
            params[3] = u32::from(condition.is_some() || auxiliary.is_some());
            if auxiliary.is_none() {
                a = Some(batch.input(condition.unwrap_or_default()));
            }
        }
        Element::Embedding { table, channels } => {
            params[2] = channels as u32;
            params[3] = (table.len() / channels - 1) as u32;
            a = Some(batch.weight(table));
        }
    }
    let grid = batch.grid((work as u32).div_ceil(64))?;
    params[1] = grid[0];
    batch.dispatch(
        op.kernel(),
        grid,
        params,
        [
            Some(input),
            auxiliary.or(a.as_ref()),
            b.as_ref(),
            Some(result),
            None,
        ],
    )
}
