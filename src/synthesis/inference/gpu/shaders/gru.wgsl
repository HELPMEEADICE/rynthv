@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> initial: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;

var<workgroup> state: array<array<f32, 1024>, 2>;

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
    let frame = index / (3u * hidden);
    if (frame >= params[0].z) {
        return;
    }
    let gate = (index / hidden) % 3u;
    let channel = index % hidden;
    let base = gate * (inputs + 1u) * hidden + channel;
    let first = frame * inputs;
    var sum = 0.0;
    let grouped = inputs & ~3u;
    for (var i = 0u; i < grouped; i += 4u) {
        let p0 = weights[base + i * hidden] * input[first + i];
        let p1 = weights[base + (i + 1u) * hidden] * input[first + i + 1u];
        let p2 = weights[base + (i + 2u) * hidden] * input[first + i + 2u];
        let p3 = weights[base + (i + 3u) * hidden] * input[first + i + 3u];
        sum = p3 + ((p2 + p1) + (p0 + sum));
    }
    for (var i = grouped; i < inputs; i += 1u) {
        sum += weights[base + i * hidden] * input[first + i];
    }
    output[index] = sum + weights[base + inputs * hidden];
}

fn recurrent(gate: u32, channel: u32, bank: u32, seed: f32) -> f32 {
    let hidden = params[0].y;
    let base = (3u * (params[0].x + 1u) + gate * (hidden + 1u)) * hidden + channel;
    var sum = seed;
    let grouped = hidden & ~3u;
    for (var i = 0u; i < grouped; i += 4u) {
        let p0 = weights[base + i * hidden] * state[bank][i];
        let p1 = weights[base + (i + 1u) * hidden] * state[bank][i + 1u];
        let p2 = weights[base + (i + 2u) * hidden] * state[bank][i + 2u];
        let p3 = weights[base + (i + 3u) * hidden] * state[bank][i + 3u];
        sum = p3 + ((p2 + p1) + (p0 + sum));
    }
    for (var i = grouped; i < hidden; i += 1u) {
        sum += weights[base + i * hidden] * state[bank][i];
    }
    return sum + weights[base + hidden * hidden];
}

@compute @workgroup_size(64)
fn gru_recurrent(@builtin(local_invocation_index) lane: u32) {
    let hidden = params[0].y;
    let start = params[2].x;
    let width = params[1].x;
    let offset = params[1].y;
    for (var channel = lane; channel < hidden; channel += 64u) {
        if (start == 0u) {
            state[0][channel] = initial[channel];
        } else {
            state[0][channel] = output[frame_index(start - 1u) * width + offset + channel];
        }
    }
    workgroupBarrier();
    for (var step = start; step < params[2].y; step += 1u) {
        let frame = frame_index(step);
        let bank = (step - start) & 1u;
        for (var channel = lane; channel < hidden; channel += 64u) {
            let base = frame * 3u * hidden + channel;
            let reset = sigmoid(recurrent(0u, channel, bank, input[base]));
            let update = sigmoid(recurrent(1u, channel, bank, input[base + hidden]));
            let candidate = tanh(input[base + hidden * 2u] + reset * recurrent(2u, channel, bank, 0.0));
            let value = state[bank][channel] * update + candidate * (1.0 - update);
            state[1u - bank][channel] = value;
            output[frame * width + offset + channel] = value;
        }
        workgroupBarrier();
    }
}
