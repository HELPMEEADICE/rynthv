use super::{Batch, batch::Kernel};
use crate::synthesis::inference::{affine::Affine, compute, matrix::MatrixError};
use wgpu::Buffer;

const AFFINE: Kernel = Kernel {
    entry: "affine",
    source: include_str!("shaders/affine.wgsl"),
};

pub(in super::super) fn affine(
    op: &Affine,
    input: &[f32],
    output: &mut [f32],
    frames: usize,
) -> Result<bool, MatrixError> {
    let weights = op.matrix().values();
    compute::accelerate(
        "affine",
        frames.saturating_mul(weights.len()),
        1_048_576,
        &[input.len(), output.len(), weights.len()],
        |batch| {
            let input = batch.input(input);
            let result = batch.output(output.len());
            encode_affine(batch, op, frames, &input, &result)?;
            batch.read(&result, output)
        },
    )
}

pub(super) fn encode_affine(
    batch: &mut Batch<'_>,
    op: &Affine,
    frames: usize,
    input: &Buffer,
    result: &Buffer,
) -> Result<(), MatrixError> {
    let weights = batch.weight(op.matrix().values());
    let bias = match op.shared_bias() {
        Some(values) => batch.weight(values),
        None => batch.input(&[]),
    };
    let columns = (op.output_channels() as u32).div_ceil(8);
    let groups = (frames as u32).div_ceil(8) * columns;
    let grid = batch.grid(groups)?;
    let mut params = [0; 16];
    params[..6].copy_from_slice(&[
        frames as u32,
        op.input_channels() as u32,
        op.output_channels() as u32,
        u32::from(op.shared_bias().is_some()),
        grid[0],
        columns,
    ]);
    batch.dispatch(
        AFFINE,
        grid,
        params,
        [Some(input), Some(&weights), Some(&bias), Some(result), None],
    )
}
