use super::{BANDS, TAPS};
use std::arch::wasm32::{f32x4_add, f32x4_mul, f32x4_splat, v128, v128_load, v128_store};

pub(super) fn analysis(samples: &[f32], weights: &[[f32; BANDS]], output: &mut [f32; BANDS]) {
    let mut low = f32x4_splat(0.0);
    let mut high = low;
    for (&sample, weights) in samples.iter().zip(weights) {
        let sample = f32x4_splat(sample);
        let (low_weights, high_weights) = weights.split_at(4);
        low = f32x4_add(low, f32x4_mul(sample, load(low_weights)));
        high = f32x4_add(high, f32x4_mul(sample, load(high_weights)));
    }
    let (low_output, high_output) = output.split_at_mut(4);
    store(low_output, low);
    store(high_output, high);
}

pub(super) fn synthesis(
    samples: &[f32; BANDS],
    weights: &[[f32; TAPS]; BANDS],
    output: &mut [f32],
) {
    // Lanes are adjacent output taps. Every tap sees bands in the original order.
    let vectors = samples.map(|sample| f32x4_splat(sample));
    let (body, tail) = output.as_chunks_mut::<4>();
    for (index, out) in body.iter_mut().enumerate() {
        let tap = index * 4;
        let mut sum = load(out);
        for (&sample, weights) in vectors.iter().zip(weights) {
            sum = f32x4_add(sum, f32x4_mul(sample, load(&weights[tap..tap + 4])));
        }
        store(out, sum);
    }
    for (&sample, weights) in samples.iter().zip(weights) {
        tail[0] += sample * weights[TAPS - 1];
    }
}

#[inline]
fn load(values: &[f32]) -> v128 {
    debug_assert_eq!(values.len(), 4);
    // SAFETY: callers supply four readable floats and v128_load permits unaligned addresses.
    unsafe { v128_load(values.as_ptr().cast()) }
}

#[inline]
fn store(output: &mut [f32], value: v128) {
    debug_assert_eq!(output.len(), 4);
    // SAFETY: callers supply four writable floats and v128_store permits unaligned addresses.
    unsafe { v128_store(output.as_mut_ptr().cast(), value) }
}
