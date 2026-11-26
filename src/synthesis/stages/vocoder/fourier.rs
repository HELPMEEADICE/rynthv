//! Shared complex arithmetic for excitation chirps and spectral filtering.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use crate::synthesis::voice::{Result, VoiceError};

pub(super) type Complex = [f32; 2];

pub(super) fn multiply([ar, ai]: Complex, [br, bi]: Complex) -> Complex {
    [ar * br - ai * bi, ar * bi + ai * br]
}

/// Unnormalized radix-2 FFT on a power-of-two complex buffer.
///
/// The forward transform uses a negative exponent; inverse uses a positive
/// exponent. Callers apply their own `1/count` normalization after inversion.
#[derive(Debug)]
pub(super) struct FftPlan {
    swaps: Vec<(usize, usize)>,
    twiddles: Vec<Complex>,
}

impl FftPlan {
    pub(super) fn shared(count: usize) -> Result<Arc<Self>> {
        static PLANS: OnceLock<Mutex<HashMap<usize, Weak<FftPlan>>>> = OnceLock::new();
        let mut plans = PLANS.get_or_init(Mutex::default).lock().unwrap();
        if let Some(plan) = plans.get(&count).and_then(Weak::upgrade) {
            return Ok(plan);
        }
        plans.retain(|_, plan| plan.strong_count() != 0);
        let plan = Arc::new(Self::new(count)?);
        plans.insert(count, Arc::downgrade(&plan));
        Ok(plan)
    }

    fn new(count: usize) -> Result<Self> {
        debug_assert!(count >= 2 && count.is_power_of_two());
        let mut swaps = Vec::new();
        let mut twiddles = Vec::new();
        swaps.try_reserve_exact(count / 2).map_err(fft_allocation)?;
        twiddles
            .try_reserve_exact(count - 1)
            .map_err(fft_allocation)?;
        let shift = usize::BITS - count.trailing_zeros();
        for index in 0..count {
            let reversed = index.reverse_bits() >> shift;
            if index < reversed {
                swaps.push((index, reversed));
            }
        }
        // Consecutive butterflies within a stage read consecutive coefficients.
        twiddles.resize(count - 1, [0.0; 2]);
        let (earlier, roots) = twiddles.split_at_mut(count / 2 - 1);
        for (index, root) in roots.iter_mut().enumerate() {
            let angle = -std::f64::consts::TAU * index as f64 / count as f64;
            let (sin, cos) = angle.sin_cos();
            *root = [cos as f32, sin as f32];
        }
        let mut half = 1;
        while half < count / 2 {
            for (target, &root) in earlier[half - 1..2 * half - 1]
                .iter_mut()
                .zip(roots.iter().step_by(count / (2 * half)))
            {
                *target = root;
            }
            half *= 2;
        }
        Ok(Self { swaps, twiddles })
    }

    pub(super) fn transform<const INVERSE: bool>(&self, values: &mut [Complex]) {
        let count = values.len();
        debug_assert_eq!(self.twiddles.len() + 1, count);
        for &(index, reversed) in &self.swaps {
            values.swap(index, reversed);
        }
        let mut half = 1;
        while half < count {
            let roots = &self.twiddles[half - 1..2 * half - 1];
            for chunk in values.chunks_exact_mut(2 * half) {
                let (left, right) = chunk.split_at_mut(half);
                for ((a, b), &[real, imag]) in left.iter_mut().zip(right).zip(roots) {
                    let root = [real, if INVERSE { -imag } else { imag }];
                    let product = multiply(*b, root);
                    let previous = *a;
                    *a = [previous[0] + product[0], previous[1] + product[1]];
                    *b = [previous[0] - product[0], previous[1] - product[1]];
                }
            }
            half *= 2;
        }
    }
}

fn fft_allocation(_: std::collections::TryReserveError) -> VoiceError {
    VoiceError::new("Could not allocate vocoder FFT plan")
}
