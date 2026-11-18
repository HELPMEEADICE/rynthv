//! Portable packed convolution with the same independent lane reductions.

use super::OUTPUT_BLOCK;

#[derive(Clone, Copy)]
pub(super) struct Output([f32; OUTPUT_BLOCK]);

impl Output {
    pub(super) fn zero() -> Self {
        Self([0.0; OUTPUT_BLOCK])
    }

    pub(super) fn load(values: &[f32]) -> Self {
        let mut output = Self::zero();
        output.0[..values.len()].copy_from_slice(values);
        output
    }

    pub(super) fn store(self, values: &mut [f32]) {
        values.copy_from_slice(&self.0[..values.len()]);
    }

    pub(super) fn add(&mut self, value: Self) {
        for (sum, value) in self.0.iter_mut().zip(value.0) {
            *sum += value;
        }
    }

    fn accumulate(&mut self, weights: &[f32; OUTPUT_BLOCK], sample: f32) {
        for (sum, &weight) in self.0.iter_mut().zip(weights) {
            *sum += weight * sample;
        }
    }
}

pub(super) fn accumulate_one(weights: &[f32], source: &[f32], sum: &mut Output) {
    let (weights, tail) = weights.as_chunks::<OUTPUT_BLOCK>();
    debug_assert!(tail.is_empty());
    debug_assert_eq!(weights.len(), source.len());
    for (weights, &sample) in weights.iter().zip(source) {
        sum.accumulate(weights, sample);
    }
}

pub(super) fn accumulate_four(weights: &[f32], source: [&[f32]; 4], sums: &mut [Output; 4]) {
    for (row, sum) in source.into_iter().zip(sums) {
        accumulate_one(weights, row, sum);
    }
}
