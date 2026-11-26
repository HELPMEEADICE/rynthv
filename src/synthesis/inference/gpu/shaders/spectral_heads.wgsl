@group(0) @binding(0) var<storage, read> band: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<storage, read_write> spectra: array<f32>;
@group(0) @binding(4) var<uniform> params: array<vec4<u32>, 4>;

@compute @workgroup_size(64)
fn merge_spectral_band(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let index = (group.x + group.y * params[0].y) * 64u + lane;
    if (index >= params[0].x) {
        return;
    }
    let first_bin = params[0].z;
    let band_width = params[0].w;
    let bins = params[1].x;
    let heads = params[1].y;
    let frames = params[1].z;
    let frame = params[1].w + index / (band_width * heads);
    let head = (index / band_width) % heads;
    let bin = index % band_width;
    let target_index = (head * frames + frame) * bins + first_bin + bin;
    spectra[target_index] += band[index] * weights[bin];
}
