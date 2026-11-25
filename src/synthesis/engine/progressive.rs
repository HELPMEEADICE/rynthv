//! Demand-driven PCM production over one immutable prepared owner revision.
//!
//! Neural execution cursors may be replaced after a seek. Prepared duration,
//! pitch/features and already published PCM remain owned by this revision.
mod cursor;
mod network;
mod pcm;
mod pipeline;
mod prepare;
mod render;

pub use cursor::Cursor;
pub use network::Network;
pub use prepare::prepare;
pub use render::synthesize;

use super::{OwnerSynthesis, PreparedRenderAcoustics, RenderResult, StageTimings};
use crate::synthesis::{
    audio::RenderedAudio,
    rendered::{PitchTrack, RenderVisuals, RenderedPhonemeTrack, Waveform},
    voice::Result,
};
use network::PreparedFrames;
use pcm::PcmStore;
use std::{ops::Range, sync::OnceLock};
use web_time::Instant;

pub struct PreparedRender {
    acoustic: PreparedRenderAcoustics,
    pcm: PcmStore,
    pitch: PitchTrack,
    phonemes: RenderedPhonemeTrack,
    timbre_frames: OnceLock<PreparedFrames>,
}

pub(crate) struct CompletedRender {
    pub(crate) state: OwnerSynthesis,
    pub(crate) pitch: PitchTrack,
    pub(crate) phonemes: RenderedPhonemeTrack,
    pub(crate) timings: StageTimings,
}

impl PreparedRender {
    /// Immutable pitch geometry for progressive display. The worker retains
    /// this once and shares its values across ready PCM interval snapshots.
    pub fn pitch_track(&self) -> Result<PitchTrack> {
        Ok(self.pitch.clone())
    }

    pub fn phoneme_track(&self) -> RenderedPhonemeTrack {
        self.phonemes.clone()
    }

    /// Existing unchanged regions are available before any new cursor runs.
    /// Publication still applies the owner instance's visible coverage.
    pub fn ready_audio(&self) -> Result<Vec<RenderedAudio>> {
        self.pcm.ready_audio()
    }

    /// Exact native model clock. Playback intersects its own integer coverage
    /// with this range to distinguish outside silence from pending PCM.
    pub fn model_samples(&self) -> Range<i64> {
        self.pcm.model_samples()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn visible_samples(&self) -> Range<i64> {
        use crate::synthesis::score::SAMPLE_RATE;
        let model = self.pcm.model_samples();
        let phrase = &self.acoustic.prepared.phrase;
        let offsets = super::components::visible_offsets(
            model.start as f64 / f64::from(SAMPLE_RATE),
            (model.end - model.start) as usize,
            phrase.visible_begin_seconds,
            phrase.visible_end_seconds,
        );
        model.start + offsets.start as i64..model.start + offsets.end as i64
    }

    pub(crate) fn suppress_pcm_publication(&mut self) {
        self.pcm.suppress_publication();
    }

    pub fn needs_audio(&self, seconds: f64) -> bool {
        self.pcm.needs(seconds)
    }

    pub fn is_complete(&self) -> bool {
        self.pcm.is_complete()
    }

    /// Only a complete owner creates a history snapshot. Progressive display
    /// patches never become synthesis baselines for a later source revision.
    pub fn finish(self) -> Result<RenderResult> {
        let completed = self.into_completed()?;
        let phrase = &completed.state.prepared.phrase;
        let audio = completed
            .state
            .components
            .publish(phrase.visible_begin_seconds, phrase.visible_end_seconds)?;
        let started = Instant::now();
        let visuals = RenderVisuals {
            pitch: completed.pitch,
            phonemes: completed.phonemes,
            waveform: Waveform::new(&audio),
        };
        let mut timings = completed.timings;
        timings.visuals_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(RenderResult {
            audio,
            visuals,
            timings,
            state: completed.state,
        })
    }

    pub(crate) fn into_completed(self) -> Result<CompletedRender> {
        Ok(CompletedRender {
            pitch: self.pitch,
            phonemes: self.phonemes,
            timings: self.acoustic.timings,
            state: OwnerSynthesis {
                prepared: self.acoustic.prepared,
                duration: self.acoustic.duration,
                pitch: self.acoustic.pitch,
                components: self.pcm.into_components()?,
            },
        })
    }
}
