@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> a: array<f32>;
@group(0) @binding(2) var<storage, read> b: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;

fn index(wg: vec3<u32>, lane: u32) -> u32 {
    return (wg.x + wg.y * params[0].y) * 64u + lane;
}

fn sigmoid(x: f32) -> f32 {
    // Keep exp finite even for large negative logits.
    let tail = exp(-abs(x));
    return select(tail, 1.0, x >= 0.0) / (1.0 + tail);
}

fn saturated_tanh(x: f32) -> f32 {
    // tanh has already rounded to +/-1 in f32 at +/-10. Bound its input
    // so implementations that lower tanh to exponentials cannot overflow.
    return tanh(clamp(x, -10.0, 10.0));
}

@compute @workgroup_size(64)
fn activation(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = index(wg, lane);
    if (i >= params[0].x) {
        return;
    }
    let x = input[i];
    switch params[0].z {
        case 1u: { output[i] = select(0.0, x, x > 0.0); }
        case 2u: { output[i] = saturated_tanh(x); }
        case 3u: { output[i] = sigmoid(x); }
        case 4u: { output[i] = x * sigmoid(x); }
        case 5u: { output[i] = select(bitcast<f32>(params[0].w), 1.0, x > 0.0) * x; }
        default: { output[i] = x; }
    }
}

@compute @workgroup_size(64)
fn channel_affine(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = index(wg, lane);
    if (i < params[0].x) {
        let c = i % params[0].z;
        output[i] = input[i] * a[c] + b[c];
    }
}

fn reduce(v: array<f32, 8>) -> f32 {
    return ((v[7] + v[3]) + (v[5] + v[1])) + ((v[6] + v[2]) + (v[4] + v[0]));
}

@compute @workgroup_size(64)
fn layer_norm(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let frame = index(wg, lane);
    let channels = params[0].z;
    if (frame >= params[0].x / channels) {
        return;
    }
    let base = frame * channels;
    var sums: array<f32, 8>;
    var squares: array<f32, 8>;
    let grouped = channels & ~7u;
    for (var first = 0u; first < grouped; first += 8u) {
        for (var i = 0u; i < 8u; i += 1u) {
            let x = input[base + first + i];
            sums[i] += x;
            squares[i] += x * x;
        }
    }
    var sum = reduce(sums);
    var square_sum = reduce(squares);
    for (var i = grouped; i < channels; i += 1u) {
        let x = input[base + i];
        sum += x;
        square_sum += x * x;
    }
    let divisor = 1.0 / f32(channels);
    let mean = sum * divisor;
    let deviation = sqrt((square_sum * divisor + bitcast<f32>(params[0].w)) - mean * mean);
    let inverse = 1.0 / deviation;
    let vector_end = channels & ~3u;
    for (var i = 0u; i < channels; i += 1u) {
        let centered = input[base + i] - mean;
        var gain = 1.0;
        if (params[1].x != 0u) {
            gain = a[i];
        }
        var normalized = centered * inverse;
        if (i < vector_end && params[1].x != 0u) {
            normalized = (gain * inverse) * centered;
        } else if (i == vector_end && (channels & 1u) != 0u && (params[1].x | params[1].y) != 0u) {
            normalized = (centered * gain) / deviation;
        } else if (params[1].x != 0u) {
            normalized = (centered * gain) * inverse;
        }
        if (params[1].y != 0u) {
            normalized += b[i];
        }
        output[base + i] = normalized;
    }
}

@compute @workgroup_size(64)
fn gated_activation(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = index(wg, lane);
    if (i >= params[0].x) {
        return;
    }
    let channels = params[0].z;
    let row = i / channels * channels * 2u + i % channels;
    var signal = input[row];
    var gate = input[row + channels];
    if (params[0].w != 0u) {
        signal += a[row];
        gate += a[row + channels];
    }
    output[i] = saturated_tanh(signal) * sigmoid(gate);
}

@compute @workgroup_size(64)
fn embedding(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = index(wg, lane);
    if (i < params[0].x) {
        let channels = params[0].z;
        let last = params[0].w;
        let x = clamp(input[i / channels], 0.0, f32(last));
        let whole = floor(x);
        let token = u32(whole) + u32(x - whole >= 0.5);
        output[i] = a[min(last, token) * channels + i % channels];
    }
}
