@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> initial: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;

var<workgroup> state: array<f32, 2048>;

fn sigmoid(x: f32) -> f32 {
    return 1.0 / (1.0 + exp(-x));
}

fn frame_index(step: u32) -> u32 {
    return select(step, params[0].z - 1u - step, params[1].z != 0u);
}

@compute @workgroup_size(64)
fn gru_input(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let index = (wg.x + wg.y * params[0].w) * 64u + lane;
    let inputs = params[0].x;
    let hidden = params[0].y;
    let frame = index / hidden;
    if (frame >= params[0].z) {
        return;
    }
    let channel = index % hidden;
    let first = frame * inputs;
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < inputs; i += 1u) {
        sum += weights[channel + i * hidden].xyz * input[first + i];
    }
    sum += weights[channel + inputs * hidden].xyz;
    let base = frame * 3u * hidden + channel;
    output[base] = sum.x;
    output[base + hidden] = sum.y;
    output[base + 2u * hidden] = sum.z;
}

fn recurrent(channel: u32, bank: u32, seed: vec3<f32>) -> vec3<f32> {
    let hidden = params[0].y;
    let base = (params[0].x + 1u) * hidden + channel;
    var sum = seed;
    for (var i = 0u; i < hidden; i += 1u) {
        sum += weights[base + i * hidden].xyz * state[bank + i];
    }
    return sum + weights[base + hidden * hidden].xyz;
}

@compute @workgroup_size(64)
fn gru_recurrent(@builtin(local_invocation_index) lane: u32) {
    let hidden = params[0].y;
    let start = params[2].x;
    let width = params[1].x;
    let offset = params[1].y;
    for (var channel = lane; channel < hidden; channel += 64u) {
        if (start == 0u) {
            state[channel] = initial[channel];
        } else {
            state[channel] = output[frame_index(start - 1u) * width + offset + channel];
        }
    }
    workgroupBarrier();
    for (var step = start; step < params[2].y; step += 1u) {
        let frame = frame_index(step);
        let bank = ((step - start) & 1u) * hidden;
        for (var channel = lane; channel < hidden; channel += 64u) {
            let base = frame * 3u * hidden + channel;
            let recurrent_terms = recurrent(channel, bank, vec3<f32>(input[base], input[base + hidden], 0.0));
            let reset = sigmoid(recurrent_terms.x);
            let update = sigmoid(recurrent_terms.y);
            let candidate = tanh(input[base + hidden * 2u] + reset * recurrent_terms.z);
            let value = state[bank + channel] * update + candidate * (1.0 - update);
            state[hidden - bank + channel] = value;
            output[frame * width + offset + channel] = value;
        }
        workgroupBarrier();
    }
}
