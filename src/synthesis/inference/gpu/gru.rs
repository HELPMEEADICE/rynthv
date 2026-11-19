use super::{Batch, batch::Kernel};
use crate::synthesis::inference::{
    compute,
    matrix::MatrixError,
    recurrent::{Gru, GruState},
};
use wgpu::Buffer;

const SOURCE: &str = include_str!("shaders/gru.wgsl");
const MAX_HIDDEN: usize = 1024;

pub(in super::super) fn gru(
    op: &Gru,
    input: &[f32],
    output: &mut [f32],
    hidden: &[f32],
    frames: usize,
) -> Result<bool, MatrixError> {
    execute("gru", &[(op, hidden)], input, output, frames)
}

pub(in super::super) fn bidirectional_gru(
    operators: [&Gru; 2],
    input: &[f32],
    output: &mut [f32],
    states: [&GruState; 2],
    frames: usize,
) -> Result<bool, MatrixError> {
    let directions = operators
        .into_iter()
        .zip(states)
        .map(|(op, state)| (op, state.hidden()))
        .collect::<Vec<_>>();
    execute("bidirectional_gru", &directions, input, output, frames)
}

fn execute(
    name: &'static str,
    directions: &[(&Gru, &[f32])],
    input: &[f32],
    output: &mut [f32],
    frames: usize,
) -> Result<bool, MatrixError> {
    let mut work = 0usize;
    let mut lengths = vec![input.len(), output.len()];
    for &(op, _) in directions {
        let hidden = op.output_channels();
        if hidden > MAX_HIDDEN || frames.saturating_mul(hidden * hidden) > 256 * 1024 * 1024 {
            compute::record_cpu(name, "sequence exceeds WebGPU recurrent workgroup budget");
            return Ok(false);
        }
        let gate_work = hidden.saturating_mul(op.input_channels().saturating_add(hidden));
        work = work.saturating_add(frames.saturating_mul(gate_work).saturating_mul(3));
        lengths.extend([frames.saturating_mul(3 * hidden), op.gpu_weight_len()]);
    }
    let width = output.len() / frames;
    compute::accelerate(name, work, 1_048_576, &lengths, |batch| {
        let source = batch.input(input);
        let result = batch.output(output.len());
        let mut offset = 0;
        for (index, &(op, initial)) in directions.iter().enumerate() {
            encode(
                batch,
                &source,
                &result,
                op,
                initial,
                frames,
                width,
                offset,
                index != 0,
            )?;
            offset += op.output_channels();
        }
        batch.read(&result, output)
    })
}

#[allow(clippy::too_many_arguments)]
fn encode(
    batch: &mut Batch<'_>,
    source: &Buffer,
    result: &Buffer,
    op: &Gru,
    initial: &[f32],
    frames: usize,
    width: usize,
    offset: usize,
    reverse: bool,
) -> Result<(), MatrixError> {
    let hidden = op.output_channels();
    let weights = batch.weight(op.gpu_weights());
    let state = batch.input(initial);
    let projected = batch.output(frames * 3 * hidden);
    let grid = batch.grid(((frames * hidden) as u32).div_ceil(64))?;
    let mut params = [0; 16];
    params[..8].copy_from_slice(&[
        op.input_channels() as u32,
        hidden as u32,
        frames as u32,
        grid[0],
        width as u32,
        offset as u32,
        u32::from(reverse),
        0,
    ]);
    batch.dispatch(
        Kernel {
            entry: "gru_input",
            source: SOURCE,
        },
        grid,
        params,
        [Some(source), Some(&weights), None, Some(&projected), None],
    )?;
    // Each recurrent dispatch uses a single workgroup. Bound its serial
    // workload and seed the next dispatch from the previous output on-device.
    let grain = (4_194_304 / (hidden * hidden)).clamp(1, 128);
    for first in (0..frames).step_by(grain) {
        params[8] = first as u32;
        params[9] = (first + grain).min(frames) as u32;
        batch.dispatch(
            Kernel {
                entry: "gru_recurrent",
                source: SOURCE,
            },
            [1, 1],
            params,
            [
                Some(&projected),
                Some(&weights),
                Some(&state),
                Some(result),
                None,
            ],
        )?;
    }
    Ok(())
}
