//! Bounded, ordered acoustic-to-PCM handoff.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod worker;

#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::Pipeline;
#[cfg(target_arch = "wasm32")]
pub(super) use worker::Pipeline;

use crate::synthesis::{
    execution,
    inference::Sequence,
    stages::vocoder::{VocoderOutput, VocoderStream, VoiceControl},
    voice::{Result, VoiceError},
};
use web_time::Instant;

struct Input {
    #[cfg(not(target_arch = "wasm32"))]
    sequence: usize,
    physical: Sequence,
    controls: Option<Sequence>,
    voice_controls: Option<Vec<VoiceControl>>,
    natural_final: bool,
}

pub(super) struct Output {
    #[cfg(not(target_arch = "wasm32"))]
    sequence: usize,
    pub pcm: VocoderOutput,
    pub natural_final: bool,
    pub elapsed_ms: f64,
}

impl Input {
    fn new(
        ready: bool,
        #[cfg(not(target_arch = "wasm32"))] sequence: usize,
        physical: Sequence,
        controls: Option<Sequence>,
        voice_controls: Option<Vec<VoiceControl>>,
        natural_final: bool,
    ) -> Result<Self> {
        if !ready
            || physical.frames > 128
            || physical.channels != 72
            || controls.as_ref().is_some_and(|controls| {
                controls.channels != 2 || physical.frames != controls.frames
            })
            || voice_controls
                .as_ref()
                .is_some_and(|controls| controls.len() != physical.frames)
        {
            return Err(VoiceError::new("Invalid progressive vocoder submission"));
        }
        execution::checkpoint()?;
        Ok(Self {
            #[cfg(not(target_arch = "wasm32"))]
            sequence,
            physical,
            controls,
            voice_controls,
            natural_final,
        })
    }

    fn process(
        self,
        vocoder: &mut VocoderStream<'_>,
        setup_ms: f64,
        #[cfg(feature = "synth-profile")] profile: Option<
            &crate::synthesis::profiling::ParallelWork,
        >,
    ) -> Result<Output> {
        execution::checkpoint()?;
        let started = Instant::now();
        let process = || {
            vocoder.process(
                &self.physical,
                self.controls.as_ref(),
                self.voice_controls.as_deref(),
                self.natural_final,
            )
        };
        #[cfg(feature = "synth-profile")]
        let pcm = match profile {
            Some(profile) => profile.measure(process),
            None => process(),
        }?;
        #[cfg(not(feature = "synth-profile"))]
        let pcm = {
            let mut process = process;
            process()?
        };
        execution::checkpoint()?;
        Ok(Output {
            #[cfg(not(target_arch = "wasm32"))]
            sequence: self.sequence,
            pcm,
            natural_final: self.natural_final,
            elapsed_ms: setup_ms + started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}
