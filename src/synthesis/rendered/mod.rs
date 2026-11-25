//! Immutable synthesis output and the display data prepared alongside its PCM.
//!
//! Waveform queries share the rendered samples and use a peak pyramid. Neither
//! painting nor owner-cache publication needs to scan or copy an entire clip.

pub mod patch;
mod phonemes;
mod pitch;
mod snapshot;
mod waveform;

pub use phonemes::{PhonemeSnapshot, RenderedPhoneme, RenderedPhonemeTrack, SourcePhone};
pub use pitch::PitchTrack;
pub use snapshot::{VisualFlash, VisualPatch, VisualPiece, VisualSnapshot, VisualTimeline};
pub use waveform::Waveform;

use super::{
    audio::RenderedAudio,
    engine::{OwnerSynthesis, PcmComponents, RenderResult},
};
use std::sync::Arc;

#[derive(Debug)]
pub struct RenderedClip {
    pub audio: Arc<RenderedAudio>,
    pub visuals: Arc<RenderVisuals>,
    pub state: RenderState,
}

#[derive(Debug)]
pub enum RenderState {
    Local(Arc<OwnerSynthesis>),
    #[cfg(target_arch = "wasm32")]
    CpuWorker {
        components: PcmComponents,
        slot: usize,
        serial: u64,
    },
}

impl From<RenderResult> for RenderedClip {
    fn from(result: RenderResult) -> Self {
        Self {
            audio: Arc::new(result.audio),
            visuals: Arc::new(result.visuals),
            state: RenderState::Local(Arc::new(result.state)),
        }
    }
}

impl RenderedClip {
    /// Shared PCM is counted once, even though the waveform also retains it.
    /// Prepared synthesis context is retained for the next completed edit.
    pub fn memory_bytes(&self) -> usize {
        let state_bytes = match &self.state {
            RenderState::Local(state) => state.memory_bytes(),
            #[cfg(target_arch = "wasm32")]
            RenderState::CpuWorker { components, .. } => components.memory_bytes(),
        };
        std::mem::size_of_val(self.audio.samples())
            .saturating_add(self.visuals.memory_bytes())
            .saturating_add(state_bytes)
    }

    pub(crate) fn components(&self) -> &PcmComponents {
        match &self.state {
            RenderState::Local(state) => &state.components,
            #[cfg(target_arch = "wasm32")]
            RenderState::CpuWorker { components, .. } => components,
        }
    }
}

#[derive(Debug)]
pub struct RenderVisuals {
    pub pitch: PitchTrack,
    pub phonemes: RenderedPhonemeTrack,
    pub waveform: Waveform,
}

impl RenderVisuals {
    /// Additional display storage; the clip owns the shared PCM allocation.
    pub fn memory_bytes(&self) -> usize {
        self.pitch
            .memory_bytes()
            .saturating_add(self.phonemes.memory_bytes())
            .saturating_add(self.waveform.memory_bytes())
    }
}
