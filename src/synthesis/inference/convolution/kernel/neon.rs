//! Two four-lane NEON vectors per frame. FMUL and FADD round separately,
//! preserving the ordered reduction used by the SSE2 kernel.

use std::arch::aarch64::{float32x4_t, vaddq_f32, vdupq_n_f32, vld1q_f32, vmulq_n_f32, vst1q_f32};

use super::OUTPUT_BLOCK;

#[derive(Clone, Copy)]
pub(super) struct Output {
    low: float32x4_t,
    high: float32x4_t,
}

impl Output {
    #[inline(always)]
    pub(super) fn zero() -> Self {
        // SAFETY: This module is compiled only when NEON is enabled.
        let zero = unsafe { vdupq_n_f32(0.0) };
        Self {
            low: zero,
            high: zero,
        }
    }

    #[inline(always)]
    fn load_block(values: &[f32; OUTPUT_BLOCK]) -> Self {
        // SAFETY: Both groups of four f32s are within the array. NEON loads
        // need no additional vector alignment, and NEON is enabled for this module.
        unsafe {
            Self {
                low: vld1q_f32(values.as_ptr()),
                high: vld1q_f32(values.as_ptr().add(4)),
            }
        }
    }

    #[inline(always)]
    pub(super) fn load(values: &[f32]) -> Self {
        if let Ok(block) = values.try_into() {
            return Self::load_block(block);
        }
        let mut padded = [0.0; OUTPUT_BLOCK];
        padded[..values.len()].copy_from_slice(values);
        Self::load_block(&padded)
    }

    #[inline(always)]
    fn store_block(self, values: &mut [f32; OUTPUT_BLOCK]) {
        // SAFETY: Both groups of four f32s are within the exclusive array.
        // NEON stores need no additional vector alignment.
        unsafe {
            vst1q_f32(values.as_mut_ptr(), self.low);
            vst1q_f32(values.as_mut_ptr().add(4), self.high);
        }
    }

    #[inline(always)]
    pub(super) fn store(self, values: &mut [f32]) {
        if let Ok(block) = (&mut *values).try_into() {
            self.store_block(block);
        } else {
            let mut padded = [0.0; OUTPUT_BLOCK];
            self.store_block(&mut padded);
            values.copy_from_slice(&padded[..values.len()]);
        }
    }

    #[inline(always)]
    pub(super) fn add(&mut self, value: Self) {
        // SAFETY: NEON is enabled and these operations only use registers.
        unsafe {
            self.low = vaddq_f32(self.low, value.low);
            self.high = vaddq_f32(self.high, value.high);
        }
    }

    #[inline(always)]
    fn accumulate(&mut self, weights: Self, sample: f32) {
        // SAFETY: NEON is enabled and these operations only use registers.
        unsafe {
            self.low = vaddq_f32(self.low, vmulq_n_f32(weights.low, sample));
            self.high = vaddq_f32(self.high, vmulq_n_f32(weights.high, sample));
        }
    }
}

#[inline(always)]
pub(super) fn accumulate_one(weights: &[f32], source: &[f32], sum: &mut Output) {
    let (weights, tail) = weights.as_chunks::<OUTPUT_BLOCK>();
    debug_assert!(tail.is_empty());
    debug_assert_eq!(weights.len(), source.len());
    for (weights, &sample) in weights.iter().zip(source) {
        sum.accumulate(Output::load_block(weights), sample);
    }
}

#[inline(always)]
pub(super) fn accumulate_four(weights: &[f32], source: [&[f32]; 4], sums: &mut [Output; 4]) {
    let (weights, tail) = weights.as_chunks::<OUTPUT_BLOCK>();
    debug_assert!(tail.is_empty());
    debug_assert!(source.iter().all(|row| row.len() == weights.len()));
    let [a, b, c, d] = source;
    let rows = weights.iter().zip(a).zip(b).zip(c).zip(d);
    for ((((weights, &a), &b), &c), &d) in rows {
        let weights = Output::load_block(weights);
        sums[0].accumulate(weights, a);
        sums[1].accumulate(weights, b);
        sums[2].accumulate(weights, c);
        sums[3].accumulate(weights, d);
    }
}
