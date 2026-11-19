@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> bias: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;

var<workgroup> input_tile: array<array<f32, 8>, 8>;
var<workgroup> weight_tile: array<array<f32, 8>, 8>;

// Each tile computes eight frames by eight output channels. The second
// dimension of each tile is contiguous in its source tensor.
@compute @workgroup_size(8, 8)
fn affine(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lane: vec3<u32>) {
    let frames = params[0].x;
    let inputs = params[0].y;
    let outputs = params[0].z;
    let tile = wg.x + wg.y * params[1].x;
    let frame = (tile / params[1].y) * 8u + lane.y;
    let channel = (tile % params[1].y) * 8u + lane.x;
    let weight_channel = (tile % params[1].y) * 8u + lane.y;
    var sum = 0.0;
    for (var base = 0u; base < inputs; base += 8u) {
        let k = base + lane.x;
        var x = 0.0;
        var w = 0.0;
        if (k < inputs && frame < frames) {
            x = input[frame * inputs + k];
        }
        if (k < inputs && weight_channel < outputs) {
            w = weights[weight_channel * inputs + k];
        }
        input_tile[lane.y][lane.x] = x;
        weight_tile[lane.y][lane.x] = w;
        workgroupBarrier();
        let count = min(8u, inputs - base);
        let grouped = count & ~3u;
        for (var i = 0u; i < grouped; i += 4u) {
            let p0 = input_tile[lane.y][i] * weight_tile[lane.x][i];
            let p1 = input_tile[lane.y][i + 1u] * weight_tile[lane.x][i + 1u];
            let p2 = input_tile[lane.y][i + 2u] * weight_tile[lane.x][i + 2u];
            let p3 = input_tile[lane.y][i + 3u] * weight_tile[lane.x][i + 3u];
            sum = p3 + ((p2 + p1) + (p0 + sum));
        }
        for (var i = grouped; i < count; i += 1u) {
            sum += input_tile[lane.y][i] * weight_tile[lane.x][i];
        }
        workgroupBarrier();
    }
    if (frame < frames && channel < outputs) {
        if (params[0].w != 0u) {
            sum += bias[channel];
        }
        output[frame * outputs + channel] = sum;
    }
}
