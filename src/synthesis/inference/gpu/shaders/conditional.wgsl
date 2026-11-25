@group(0) @binding(0) var<storage, read> left: array<f32>;
@group(0) @binding(1) var<storage, read> right: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;
@group(0) @binding(5) var<storage, read_write> invalid_index: atomic<u32>;

fn sum(i: u32) -> f32 {
    var value = 0.0;
    if (params[0].z != 0u) {
        value = left[i];
    }
    return value + right[i];
}

@compute @workgroup_size(64)
fn conditional_sum(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = (wg.x + wg.y * params[0].y) * 64u + lane;
    if (i < params[0].x) {
        output[i] = sum(i);
    }
}

@compute @workgroup_size(64)
fn conditional_sum_projected(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let i = (wg.x + wg.y * params[0].y) * 64u + lane;
    if (i < params[0].x) {
        let index = params[0].w + i;
        let value = sum(index);
        if ((bitcast<u32>(value) & 0x7f800000u) == 0x7f800000u) {
            atomicMin(&invalid_index, index);
        }
        output[i] = value;
    }
}
