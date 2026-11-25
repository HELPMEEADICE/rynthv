use super::{Batch, batch::Kernel, linear::encode_affine};
use crate::synthesis::inference::{
    affine::Affine,
    compute,
    matrix::MatrixError,
    ops::attention_relative::{AttentionOptions, RelativePositions},
};
use wgpu::Buffer;

const SOURCE: &str = include_str!("shaders/attention.wgsl");

pub(in super::super) fn attention(
    [query, key, value]: [&[f32]; 3],
    relative: Option<&RelativePositions>,
    output: &mut [f32],
    dims: [usize; 5],
    options: AttentionOptions<'_>,
) -> Result<bool, MatrixError> {
    let [query_frames, key_frames, heads, keys, values] = dims;
    let name = if relative.is_some() {
        "relative_attention"
    } else {
        "attention"
    };
    let rows = query_frames * heads;
    let score_len = rows.saturating_mul(key_frames);
    let tile = query_tile(query_frames, heads, key_frames);
    if tile == 0 || !positions_fit(relative, query_frames, key_frames, options) {
        compute::record_cpu(name, "attention window or score matrix exceeds GPU budget");
        return Ok(false);
    }
    let mask = options.key_padding_mask.unwrap_or_default();
    let input_len = query
        .len()
        .saturating_add(key.len())
        .saturating_add(value.len())
        .saturating_add(mask.len());
    let q_len = query.len();
    let k_len = key.len();
    let v_len = value.len();
    compute::accelerate(
        name,
        score_len.saturating_mul(keys.saturating_add(values)),
        1_048_576,
        &[
            input_len,
            tile * heads * key_frames,
            output.len(),
            relative.map_or(0, |r| r.key.len()),
            relative.map_or(0, |r| r.value.len()),
        ],
        |batch| {
            let input = batch.staging_input(input_len);
            batch.write_slice(&input, 0, query);
            batch.write_slice(&input, q_len, key);
            batch.write_slice(&input, q_len + k_len, value);
            batch.write_slice(&input, q_len + k_len + v_len, mask);
            let result = batch.output(output.len());
            encode_attention_passes(
                batch,
                &input,
                relative,
                &result,
                dims,
                [q_len, k_len, v_len, mask.len()],
                options,
            )?;
            batch.read(&result, output)
        },
    )
}

pub(in super::super) fn projected_attention(
    ops: [&Affine; 4],
    inputs: [&[f32]; 3],
    relative: Option<&RelativePositions>,
    output_len: usize,
    dims: [usize; 5],
    options: AttentionOptions<'_>,
) -> Result<Option<Vec<f32>>, MatrixError> {
    let [query_frames, key_frames, heads, keys, values] = dims;
    let [query_op, key_op, value_op, output_op] = ops;
    let [raw_query, raw_key, raw_value] = inputs;
    if query_frames == 0 || key_frames == 0 {
        return Ok(None);
    }
    let score_len = query_frames
        .saturating_mul(heads)
        .saturating_mul(key_frames);
    let tile = query_tile(query_frames, heads, key_frames);
    if tile == 0 || !positions_fit(relative, query_frames, key_frames, options) {
        return Ok(None);
    }
    let mask = options.key_padding_mask.unwrap_or_default();
    let q_len = query_frames.saturating_mul(query_op.output_channels());
    let k_len = key_frames.saturating_mul(key_op.output_channels());
    let v_len = key_frames.saturating_mul(value_op.output_channels());
    let context_len = query_frames.saturating_mul(output_op.input_channels());
    let qkv_mask_len = q_len
        .saturating_add(k_len)
        .saturating_add(v_len)
        .saturating_add(mask.len());
    if u32::try_from(qkv_mask_len).is_err() {
        return Ok(None);
    }
    let work = query_frames
        .saturating_mul(query_op.matrix().values().len())
        .saturating_add(key_frames.saturating_mul(key_op.matrix().values().len()))
        .saturating_add(key_frames.saturating_mul(value_op.matrix().values().len()))
        .saturating_add(score_len.saturating_mul(keys.saturating_add(values)))
        .saturating_add(query_frames.saturating_mul(output_op.matrix().values().len()));
    let mut projected = None;
    compute::accelerate(
        None,
        work,
        1_048_576,
        &[
            raw_query.len(),
            raw_key.len(),
            raw_value.len(),
            q_len,
            k_len,
            v_len,
            qkv_mask_len,
            tile * heads * key_frames,
            context_len,
            output_len,
            query_op.matrix().values().len(),
            key_op.matrix().values().len(),
            value_op.matrix().values().len(),
            output_op.matrix().values().len(),
            relative.map_or(0, |r| r.key.len()),
            relative.map_or(0, |r| r.value.len()),
        ],
        |batch| {
            let raw_q_buf = batch.input(raw_query);
            let raw_k_buf = batch.input(raw_key);
            let raw_v_buf = batch.input(raw_value);
            let q_buf = batch.output(q_len);
            let k_buf = batch.output(k_len);
            let v_buf = batch.output(v_len);
            encode_affine(batch, query_op, query_frames, &raw_q_buf, &q_buf)?;
            encode_affine(batch, key_op, key_frames, &raw_k_buf, &k_buf)?;
            encode_affine(batch, value_op, key_frames, &raw_v_buf, &v_buf)?;
            let qkv_mask = batch.staging_input(qkv_mask_len);
            batch.copy_floats(&q_buf, &qkv_mask, 0, q_len);
            batch.copy_floats(&k_buf, &qkv_mask, q_len, k_len);
            batch.copy_floats(&v_buf, &qkv_mask, q_len + k_len, v_len);
            batch.write_slice(&qkv_mask, q_len + k_len + v_len, mask);
            let context_buf = batch.output(context_len);
            encode_attention_passes(
                batch,
                &qkv_mask,
                relative,
                &context_buf,
                dims,
                [q_len, k_len, v_len, mask.len()],
                options,
            )?;
            let final_buf = batch.output(output_len);
            encode_affine(batch, output_op, query_frames, &context_buf, &final_buf)?;
            let mut output = vec![0.0; output_len];
            batch.read(&final_buf, &mut output)?;
            projected = Some(output);
            Ok(())
        },
    )?;
    Ok(projected)
}

fn query_tile(queries: usize, heads: usize, keys: usize) -> usize {
    let scores_per_query = heads.saturating_mul(keys);
    if scores_per_query == 0 {
        return 0;
    }
    #[cfg(target_os = "android")]
    {
        #[cfg(feature = "gui")]
        let desired = (crate::platform::android::memory_bytes() / 256).clamp(2 << 20, 16 << 20);
        #[cfg(not(feature = "gui"))]
        let desired = 8 << 20;
        let bytes = compute::gpu_storage_budget().map_or(desired, |limit| desired.min(limit));
        queries.min(bytes as usize / 4 / scores_per_query)
    }
    #[cfg(not(target_os = "android"))]
    {
        if queries.saturating_mul(scores_per_query) <= 16 * 1024 * 1024 {
            queries
        } else {
            0
        }
    }
}

fn positions_fit(
    relative: Option<&RelativePositions>,
    query_frames: usize,
    key_frames: usize,
    options: AttentionOptions<'_>,
) -> bool {
    let entries = relative.map_or(0, |positions| positions.entries);
    let max_distance = i64::from(options.position_offset) + query_frames as i64 - 1;
    let min_distance = i64::from(options.position_offset) - key_frames as i64 + 1;
    min_distance > i64::from(i32::MIN) && max_distance + (entries / 2) as i64 <= i64::from(i32::MAX)
}

fn encode_attention_passes(
    batch: &mut Batch<'_>,
    input: &Buffer,
    relative: Option<&RelativePositions>,
    result: &Buffer,
    [query_frames, key_frames, heads, keys, values]: [usize; 5],
    [q_len, k_len, v_len, mask_len]: [usize; 4],
    options: AttentionOptions<'_>,
) -> Result<(), MatrixError> {
    let entries = relative.map_or(0, |positions| positions.entries);
    let window = options.window.unwrap_or(0).min(i32::MAX as usize);
    let tile = query_tile(query_frames, heads, key_frames);
    let scores = batch.output(tile * heads * key_frames);
    let relative_key = relative.map(|r| batch.weight(&r.key));
    let relative_value = relative.map(|r| batch.weight(&r.value));
    let empty = batch.input(&[]);
    let mut params = [0; 16];
    params[..13].copy_from_slice(&[
        0,
        key_frames as u32,
        heads as u32,
        keys as u32,
        values as u32,
        q_len as u32,
        (q_len + k_len) as u32,
        0,
        (q_len + k_len + v_len) as u32,
        u32::from(mask_len != 0),
        entries as u32,
        options.position_offset as u32,
        window as u32,
    ]);
    for offset in (0..query_frames).step_by(tile) {
        let queries = tile.min(query_frames - offset);
        params[0] = queries as u32;
        params[13] = offset as u32;
        dispatch_tile(
            batch,
            [
                input,
                relative_key.as_ref().unwrap_or(&empty),
                relative_value.as_ref().unwrap_or(&empty),
                result,
                &scores,
            ],
            params,
            queries * heads,
            key_frames,
            values,
        )?;
    }
    Ok(())
}

fn dispatch_tile(
    batch: &mut Batch<'_>,
    [input, relative_key, relative_value, result, scores]: [&Buffer; 5],
    mut params: [u32; 16],
    rows: usize,
    keys: usize,
    values: usize,
) -> Result<(), MatrixError> {
    let grid = batch.grid(((rows * keys) as u32).div_ceil(64))?;
    params[7] = grid[0];
    batch.dispatch(
        Kernel {
            source: SOURCE,
            entry: "attention_scores",
        },
        grid,
        params,
        [Some(input), Some(relative_key), None, Some(scores), None],
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
        [None, None, None, Some(scores), None],
    )?;
    let grid = batch.grid(((rows * values) as u32).div_ceil(64))?;
    params[7] = grid[0];
    batch.dispatch(
        Kernel {
            source: SOURCE,
            entry: "attention_values",
        },
        grid,
        params,
        [
            Some(input),
            None,
            Some(relative_value),
            Some(result),
            Some(scores),
        ],
    )
}
