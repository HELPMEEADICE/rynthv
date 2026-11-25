//! SIMD128 output lanes keep the scalar tap and input reduction order.

use std::arch::wasm32::{f32x4, f32x4_add, f32x4_mul, f32x4_splat, v128, v128_load, v128_store};

#[derive(Clone, Copy)]
pub(super) struct Output([v128; 2]);

impl Output {
    #[inline]
    pub(super) fn zero() -> Self {
        Self([f32x4(0.0, 0.0, 0.0, 0.0); 2])
    }

    #[inline]
    pub(super) fn load(values: &[f32]) -> Self {
        let mut padded = [0.0; 8];
        let block = values.try_into().unwrap_or_else(|_| {
            padded[..values.len()].copy_from_slice(values);
            &padded
        });
        Self(load_block(block))
    }

    #[inline]
    pub(super) fn store(self, values: &mut [f32]) {
        let mut padded = [0.0; 8];
        let target = if values.len() == 8 {
            values.as_mut_ptr()
        } else {
            padded.as_mut_ptr()
        };
        // SAFETY: either target has eight writable f32 values. SIMD permits unaligned access.
        unsafe {
            v128_store(target.cast(), self.0[0]);
            v128_store(target.add(4).cast(), self.0[1]);
        }
        if values.len() != 8 {
            values.copy_from_slice(&padded[..values.len()]);
        }
    }

    #[inline]
    pub(super) fn add(&mut self, value: Self) {
        for (sum, value) in self.0.iter_mut().zip(value.0) {
            *sum = f32x4_add(*sum, value);
        }
    }

    #[inline]
    fn multiply_add(&mut self, weights: [v128; 2], sample: f32) {
        let input = f32x4_splat(sample);
        self.add(Self(weights.map(|weight| f32x4_mul(weight, input))));
    }
}

#[inline]
fn load_block(weights: &[f32; 8]) -> [v128; 2] {
    let base = weights.as_ptr();
    // SAFETY: the array contains two readable four-float vectors.
    unsafe { [v128_load(base.cast()), v128_load(base.add(4).cast())] }
}

#[inline]
pub(super) fn accumulate_one(weights: &[f32], source: &[f32], sum: &mut Output) {
    for (block, &sample) in weights.as_chunks::<8>().0.iter().zip(source) {
        sum.multiply_add(load_block(block), sample);
    }
}

#[inline]
pub(super) fn accumulate_four(weights: &[f32], source: [&[f32]; 4], sums: &mut [Output; 4]) {
    for (index, block) in weights.as_chunks::<8>().0.iter().enumerate() {
        let weights = load_block(block);
        for (sum, src) in sums.iter_mut().zip(source) {
            sum.multiply_add(weights, src[index]);
        }
    }
}
