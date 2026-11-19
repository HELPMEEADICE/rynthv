@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> bias: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;
@group(0) @binding(5) var<storage, read> previous: array<f32>;

var<workgroup> input_tile: array<array<f32, 8>, 8>;
var<workgroup> weight_tile: array<array<f32, 8>, 8>;
var<workgroup> row_valid: array<array<u32, 8>, 8>;

@compute @workgroup_size(8, 8)
fn convolution(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lane: vec3<u32>) {
    let inputs = params[0].x;
    let outputs = params[0].y;
    let reduction = params[0].z * inputs;
    let tile = wg.x + wg.y * params[1].w;
    let block = tile % params[2].x;
    let frame = (tile / params[2].x) * 8u + lane.y;
    let channel = block * 8u + lane.x;
    var sum = 0.0;
    for (var base = 0u; base < reduction; base += 8u) {
        let input_k = base + lane.x;
        let weight_k = base + lane.y;
        var sample = 0.0;
        var coefficient = 0.0;
        var valid = false;
        if (input_k < reduction && frame < params[1].z) {
            let row = frame * params[0].w + (input_k / inputs) * params[1].x;
            if (row < params[1].y) {
                sample = input[row * inputs + input_k % inputs];
                valid = true;
            }
        }
        if (weight_k < reduction) {
            coefficient = weights[(block * reduction + weight_k) * 8u + lane.x];
        }
        input_tile[lane.y][lane.x] = sample;
        weight_tile[lane.y][lane.x] = coefficient;
        row_valid[lane.y][lane.x] = u32(valid);
        workgroupBarrier();
        for (var k = 0u; k < 8u; k += 1u) {
            if (row_valid[lane.y][k] != 0u) {
                sum += input_tile[lane.y][k] * weight_tile[k][lane.x];
            }
        }
        workgroupBarrier();
    }
    if (frame < params[1].z && channel < outputs) {
        if (params[2].z != 0u) {
            sum += bias[channel];
        }
        output[frame * outputs + channel] = sum;
    }
}

@compute @workgroup_size(64)
fn transposed_convolution(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let index = (wg.x + wg.y * params[1].w) * 64u + lane;
    let inputs = params[0].x;
    let outputs = params[0].y;
    let kernel = params[0].z;
    let stride = params[0].w;
    let dilation = params[1].x;
    let frame = index / outputs;
    if (frame >= params[1].z) {
        return;
    }
    let channel = index % outputs;
    var sum = 0.0;
    if (index < params[2].y) {
        sum = previous[index];
    } else if (params[2].z != 0u) {
        sum = bias[channel];
    }
    if (frame >= params[2].x) {
        let relative = frame - params[2].x;
        let last_tap = (kernel - 1u) * dilation;
        var first = 0u;
        if (relative > last_tap) {
            first = (relative - last_tap - 1u) / stride + 1u;
        }
        let end = min(relative / stride + 1u, params[1].y);
        for (var source = first; source < end; source += 1u) {
            let distance = relative - source * stride;
            if (distance % dilation == 0u) {
                let tap = distance / dilation;
                let base = ((channel / 8u * kernel + tap) * inputs) * 8u + channel % 8u;
                for (var i = 0u; i < inputs; i += 1u) {
                    sum += weights[base + i * 8u] * input[source * inputs + i];
                }
            }
        }
    }
    output[index] = sum;
}
