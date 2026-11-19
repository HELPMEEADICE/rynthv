@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> relative_key: array<f32>;
@group(0) @binding(2) var<storage, read> relative_value: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;
@group(0) @binding(5) var<storage, read> scores: array<f32>;

var<workgroup> reduction: array<f32, 64>;

fn distance(query: u32, key: u32) -> i32 {
    // Subtract the frame indices first. The host bounds the result after the offset.
    return (i32(query) - i32(key)) + bitcast<i32>(params[2].w);
}

fn position(delta: i32) -> u32 {
    let entries = params[2].z;
    return u32(clamp(delta + i32(entries / 2u), 0, i32(entries - 1u)));
}

@compute @workgroup_size(64)
fn attention_scores(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let index = (wg.x + wg.y * params[1].w) * 64u + lane;
    let keys = params[0].y;
    let heads = params[0].z;
    let channels = params[0].w;
    let query = index / keys / heads;
    if (query >= params[0].x) {
        return;
    }
    let head = (index / keys) % heads;
    let key = index % keys;
    let q = (query * heads + head) * channels;
    let k = (key * heads + head) * channels + params[1].y;
    var sum = 0.0;
    for (var i = 0u; i < channels; i += 1u) {
        sum += input[q + i] * input[k + i];
    }
    sum *= 1.0 / sqrt(f32(channels));
    let delta = distance(query, key);
    if ((params[2].y != 0u && input[params[2].x + key] > 0.5) ||
        (params[3].x != 0u && u32(abs(delta)) > params[3].x)) {
        sum = bitcast<f32>(0xd01502f9u);
    }
    if (params[2].z != 0u) {
        let base = position(delta) * channels;
        var sums: array<f32, 8>;
        let grouped = channels & ~7u;
        for (var first = 0u; first < grouped; first += 8u) {
            for (var i = 0u; i < 8u; i += 1u) {
                sums[i] += input[q + first + i] * relative_key[base + first + i];
            }
        }
        var term = ((sums[7] + sums[3]) + (sums[5] + sums[1])) +
            ((sums[6] + sums[2]) + (sums[4] + sums[0]));
        for (var i = grouped; i < channels; i += 1u) {
            term += input[q + i] * relative_key[base + i];
        }
        sum += term;
    }
    output[index] = sum;
}

@compute @workgroup_size(64)
fn attention_softmax(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let row = wg.x + wg.y * params[1].w;
    if (row >= params[0].x * params[0].z) {
        return;
    }
    let keys = params[0].y;
    let base = row * keys;
    var largest = -3.402823466e+38;
    for (var i = lane; i < keys; i += 64u) {
        largest = max(largest, output[base + i]);
    }
    reduction[lane] = largest;
    workgroupBarrier();
    for (var width = 32u; width > 0u; width /= 2u) {
        if (lane < width) {
            reduction[lane] = max(reduction[lane], reduction[lane + width]);
        }
        workgroupBarrier();
    }
    let maximum = workgroupUniformLoad(&reduction[0]);
    var sum = 0.0;
    for (var i = lane; i < keys; i += 64u) {
        sum += exp(output[base + i] - maximum);
    }
    reduction[lane] = sum;
    workgroupBarrier();
    for (var width = 32u; width > 0u; width /= 2u) {
        if (lane < width) {
            reduction[lane] += reduction[lane + width];
        }
        workgroupBarrier();
    }
    let normalizer = maximum + log(reduction[0]);
    for (var i = lane; i < keys; i += 64u) {
        output[base + i] = exp(output[base + i] - normalizer);
    }
}

@compute @workgroup_size(64)
fn attention_values(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let index = (wg.x + wg.y * params[1].w) * 64u + lane;
    let channels = params[1].x;
    let keys = params[0].y;
    let heads = params[0].z;
    let query = index / channels / heads;
    if (query >= params[0].x) {
        return;
    }
    let head = (index / channels) % heads;
    let channel = index % channels;
    let score = (query * heads + head) * keys;
    var sum = 0.0;
    for (var key = 0u; key < keys; key += 1u) {
        let i = (key * heads + head) * channels + channel + params[1].z;
        sum += scores[score + key] * input[i];
    }
    if (params[2].z != 0u) {
        for (var key = 0u; key < keys; key += 1u) {
            let i = position(distance(query, key)) * channels + channel;
            sum += scores[score + key] * relative_value[i];
        }
    }
    output[index] = sum;
}
