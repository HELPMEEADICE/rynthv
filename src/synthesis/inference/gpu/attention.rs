use super::batch::Kernel;
use crate::synthesis::inference::{
    compute,
    matrix::MatrixError,
    ops::attention_relative::{AttentionOptions, RelativePositions},
};

const SOURCE: &str = include_str!("shaders/attention.wgsl");

pub(in super::super) fn attention(
    [query, key, value]: [&[f32]; 3],
    relative: Option<&RelativePositions>,
    output: &mut [f32],
    [query_frames, key_frames, heads, keys, values]: [usize; 5],
    options: AttentionOptions<'_>,
) -> Result<bool, MatrixError> {
    let name = if relative.is_some() {
        "relative_attention"
    } else {
        "attention"
    };
    let entries = relative.map_or(0, |positions| positions.entries);
    let max_distance = i64::from(options.position_offset) + query_frames as i64 - 1;
    let min_distance = i64::from(options.position_offset) - key_frames as i64 + 1;
    let window = options.window.unwrap_or(0).min(i32::MAX as usize);
    let rows = query_frames * heads;
    let score_len = rows.saturating_mul(key_frames);
    if min_distance <= i64::from(i32::MIN)
        || max_distance + (entries / 2) as i64 > i64::from(i32::MAX)
        || score_len > 16 * 1024 * 1024
    {
        compute::record_cpu(name, "attention window or score matrix exceeds GPU budget");
        return Ok(false);
    }
    let mask = options.key_padding_mask.unwrap_or_default();
    let input_len = query
        .len()
        .saturating_add(key.len())
        .saturating_add(value.len())
        .saturating_add(mask.len());
    compute::accelerate(
        name,
        score_len.saturating_mul(keys.saturating_add(values)),
        1_048_576,
        &[
            input_len,
            score_len,
            output.len(),
            relative.map_or(0, |r| r.key.len()),
            relative.map_or(0, |r| r.value.len()),
        ],
        |batch| {
            let source = [query, key, value, mask].concat();
            let input = batch.input(&source);
            let scores = batch.output(score_len);
            let result = batch.output(output.len());
            let relative_key = relative.map(|r| batch.weight(&r.key));
            let relative_value = relative.map(|r| batch.weight(&r.value));
            let empty = batch.input(&[]);
            let mut params = [0; 16];
            params[..13].copy_from_slice(&[
                query_frames as u32,
                key_frames as u32,
                heads as u32,
                keys as u32,
                values as u32,
                query.len() as u32,
                (query.len() + key.len()) as u32,
                0,
                (query.len() + key.len() + value.len()) as u32,
                u32::from(!mask.is_empty()),
                entries as u32,
                options.position_offset as u32,
                window as u32,
            ]);
            let grid = batch.grid((score_len as u32).div_ceil(64))?;
            params[7] = grid[0];
            batch.dispatch(
                Kernel {
                    source: SOURCE,
                    entry: "attention_scores",
                },
                grid,
                params,
                [
                    Some(&input),
                    Some(relative_key.as_ref().unwrap_or(&empty)),
                    None,
                    Some(&scores),
                    None,
                ],
            )?;
            let grid = batch.grid(rows as u32)?;
            params[7] = grid[0];
            batch.dispatch(
                Kernel {
                    source: SOURCE,
                    entry: "attention_softmax",
                },
                grid,
                params,
                [None, None, None, Some(&scores), None],
            )?;
            let grid = batch.grid((output.len() as u32).div_ceil(64))?;
            params[7] = grid[0];
            batch.dispatch(
                Kernel {
                    source: SOURCE,
                    entry: "attention_values",
                },
                grid,
                params,
                [
                    Some(&input),
                    None,
                    Some(relative_value.as_ref().unwrap_or(&empty)),
                    Some(&result),
                    Some(&scores),
                ],
            )?;
            batch.read(&result, output)
        },
    )
}
