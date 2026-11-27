//! The two native PCM components are retained independently for phase matching.
//! Playback sums the primary and secondary components after publication/cropping.

use crate::synthesis::{
    audio::{ChannelLayout, RenderedAudio},
    score::SAMPLE_RATE,
    voice::{Result, VoiceError},
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PcmComponents {
    pub start_seconds: f64,
    samples: Arc<[Box<[f32]>; 2]>,
}

impl PcmComponents {
    pub fn new(start_seconds: f64, samples: [Vec<f32>; 2]) -> Result<Self> {
        if !start_seconds.is_finite()
            || samples[0].len() != samples[1].len()
            || samples.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(VoiceError::new("Invalid vocoder component timeline"));
        }
        Ok(Self::from_ready(start_seconds, samples))
    }

    /// Adopt the progressive store's two completed buffers.
    pub(super) fn from_ready(start_seconds: f64, samples: [Vec<f32>; 2]) -> Self {
        Self {
            start_seconds,
            samples: Arc::new(samples.map(Vec::into_boxed_slice)),
        }
    }

    pub fn primary(&self) -> &[f32] {
        &self.samples[0]
    }

    pub fn secondary(&self) -> &[f32] {
        &self.samples[1]
    }

    pub fn publish(&self, begin_seconds: f64, end_seconds: Option<f64>) -> Result<RenderedAudio> {
        let rate = f64::from(SAMPLE_RATE);
        let range = visible_offsets(
            self.start_seconds,
            self.primary().len(),
            begin_seconds,
            end_seconds,
        );
        let (first, end) = (range.start, range.end);
        if first >= end {
            return Err(VoiceError::new(
                "Rendered components have no visible samples",
            ));
        }
        let samples = self.primary()[first..end]
            .iter()
            .zip(&self.secondary()[first..end])
            .map(|(&a, &b)| a + b)
            .collect();
        RenderedAudio::new(
            SAMPLE_RATE,
            self.start_seconds + first as f64 / rate,
            ChannelLayout::Mono,
            samples,
        )
        .map_err(|error| VoiceError::new(error.to_string()))
    }

    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of_val(self.primary())
            .saturating_add(std::mem::size_of_val(self.secondary()))
    }
}

pub(super) fn visible_offsets(
    origin: f64,
    samples: usize,
    begin: f64,
    end: Option<f64>,
) -> std::ops::Range<usize> {
    let rate = f64::from(SAMPLE_RATE);
    let first = ((begin.max(0.0) - origin) * rate)
        .ceil()
        .clamp(0.0, samples as f64) as usize;
    let end = end.map_or(samples, |end| {
        ((end - origin) * rate).floor().clamp(0.0, samples as f64) as usize
    });
    first..end
}
