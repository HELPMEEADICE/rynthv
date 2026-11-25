use super::{Batch, batch::Kernel};
use crate::synthesis::inference::{compute, convolution::Convolution, matrix::MatrixError};
use wgpu::Buffer;

const SOURCE: &str = include_str!("shaders/convolution.wgsl");

pub(super) enum Prefix<'a> {
    Zeros(usize),
    History { buffer: &'a Buffer, frames: usize },
}

pub(in super::super) fn convolution(
    op: &Convolution,
    input: &[f32],
    output: &mut [f32],
    frames: usize,
) -> Result<bool, MatrixError> {
    let config = op.config();
    let length = (frames - 1) * config.stride + (config.kernel - 1) * config.dilation;
    if u32::try_from(length).is_err() || u32::try_from(config.dilation).is_err() {
        compute::record_cpu("convolution", "frame address exceeds WebGPU index range");
        return Ok(false);
    }
    compute::accelerate(
        "convolution",
        work(op, output.len()),
        1_048_576,
        &[input.len(), op.shared_weights().len(), output.len()],
        |batch| {
            let source = batch.input(input);
            let result = batch.output(output.len());
            encode_convolution(
                batch,
                op,
                &source,
                &result,
                input.len() / op.input_channels(),
                frames,
                Prefix::Zeros(0),
            )?;
            batch.read(&result, output)
        },
    )
}

pub(super) fn encode_convolution(
    batch: &mut Batch<'_>,
    op: &Convolution,
    source: &Buffer,
    result: &Buffer,
    input_frames: usize,
    output_frames: usize,
    prefix: Prefix<'_>,
) -> Result<(), MatrixError> {
    let columns = (op.output_channels() as u32).div_ceil(8);
    let grid = batch.grid((output_frames as u32).div_ceil(8) * columns)?;
    let mut params = parameters(
        op,
        input_frames * op.input_channels(),
        output_frames * op.output_channels(),
    );
    params[7] = grid[0];
    params[8] = columns;
    let previous = match prefix {
        Prefix::Zeros(frames) => {
            params[11] = frames as u32;
            source
        }
        Prefix::History { buffer, frames } => {
            params[9] = 1;
            params[11] = frames as u32;
            buffer
        }
    };
    encode(
        batch,
        "convolution",
        op,
        grid,
        params,
        source,
        result,
        previous,
    )
}

pub(in super::super) fn transpose(
    op: &Convolution,
    input: &[f32],
    previous: &[f32],
    first_input: usize,
    output_len: usize,
) -> Result<Option<Vec<f32>>, MatrixError> {
    let config = op.config();
    if [first_input, config.stride, config.dilation]
        .iter()
        .any(|&n| u32::try_from(n).is_err())
        || u32::try_from((config.kernel - 1) * config.dilation).is_err()
    {
        compute::record_cpu(
            "transposed_convolution",
            "frame address exceeds WebGPU index range",
        );
        return Ok(None);
    }
    let mut output = Vec::new();
    compute::accelerate(
        "transposed_convolution",
        work(
            op,
            (input.len() / op.input_channels()).saturating_mul(op.output_channels()),
        ),
        1_048_576,
        &[
            input.len(),
            op.shared_weights().len(),
            output_len,
            previous.len(),
        ],
        |batch| {
            let source = batch.input(input);
            let pending = batch.input(previous);
            let result = batch.output(output_len);
            let grid = batch.grid((output_len as u32).div_ceil(64))?;
            let mut params = parameters(op, input.len(), output_len);
            params[7] = grid[0];
            params[8] = first_input as u32;
            params[9] = previous.len() as u32;
            encode(
                batch,
                "transposed_convolution",
                op,
                grid,
                params,
                &source,
                &result,
                &pending,
            )?;
            output.resize(output_len, 0.0);
            batch.read(&result, &mut output)
        },
    )
    .map(|ran| ran.then_some(output))
}

fn work(op: &Convolution, output_values: usize) -> usize {
    output_values
        .saturating_mul(op.input_channels())
        .saturating_mul(op.config().kernel)
}

fn parameters(op: &Convolution, input_len: usize, output_len: usize) -> [u32; 16] {
    let config = op.config();
    let mut params = [0; 16];
    params[..7].copy_from_slice(&[
        op.input_channels() as u32,
        op.output_channels() as u32,
        config.kernel as u32,
        config.stride as u32,
        config.dilation as u32,
        (input_len / op.input_channels()) as u32,
        (output_len / op.output_channels()) as u32,
    ]);
    params[10] = u32::from(op.has_bias());
    params
}

#[allow(clippy::too_many_arguments)]
fn encode(
    batch: &mut Batch<'_>,
    entry: &'static str,
    op: &Convolution,
    grid: [u32; 2],
    params: [u32; 16],
    source: &Buffer,
    result: &Buffer,
    previous: &Buffer,
) -> Result<(), MatrixError> {
    let weights = batch.weight(op.shared_weights());
    let bias = match op.shared_bias() {
        Some(values) => batch.weight(values),
        None => batch.input(&[]),
    };
    batch.dispatch(
        Kernel {
            source: SOURCE,
            entry,
        },
        grid,
        params,
        [
            Some(source),
            Some(&weights),
            Some(&bias),
            Some(result),
            Some(previous),
        ],
    )
}
