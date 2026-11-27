use super::{
    SharedSource,
    controls::{Controls, DeviceState},
    sampler::MixSampler,
};
use crate::synthesis::audio::{PianoControl, live::LIVE_SAMPLE_RATE, piano::PianoSynth};
use cpal::{FromSample, SizedSample, traits::DeviceTrait};
use std::sync::{Arc, atomic::Ordering};

pub(super) fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    source: Arc<SharedSource>,
    state: Arc<DeviceState>,
    piano: PianoControl,
) -> Result<cpal::Stream, cpal::Error> {
    let failed = state.clone();
    let channels = usize::from(config.channels);
    let step = f64::from(LIVE_SAMPLE_RATE) / f64::from(config.sample_rate);
    let mut piano = PianoSynth::new(config.sample_rate, piano);
    let mut loop_range = None;
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            #[cfg(target_os = "android")]
            if !crate::platform::android::lifecycle::audio_allowed() {
                output.fill(T::EQUILIBRIUM);
                return;
            }
            if state.failure.load(Ordering::Acquire) != 0 {
                output.fill(T::EQUILIBRIUM);
                return;
            }
            piano.begin_buffer();
            let audition_active = piano.is_active();
            // The control thread installs/reclaims complete source revisions.
            // This guard avoids Arc disposal and blocking on source replacement.
            let guard = source.try_read().ok();
            let session = guard
                .as_ref()
                .and_then(|guard| guard.as_ref())
                .filter(|session| {
                    let playing = session.controls.playing.load(Ordering::Acquire);
                    if !playing {
                        session.controls.buffering.store(false, Ordering::Release);
                    }
                    playing
                });
            if session.is_none() && !audition_active {
                output.fill(T::EQUILIBRIUM);
                return;
            }
            let controls = session.map(|session| &session.controls);
            let start = controls.map_or(0, |controls| controls.position.load(Ordering::Acquire));
            let mut cursor = Controls::frame(start);
            if let Some(controls) = controls {
                controls
                    .looping
                    .update_frames(config.sample_rate, &mut loop_range);
                if let Some(range) = loop_range {
                    cursor = range.wrap(cursor);
                }
            }
            let gain = controls.map_or(1.0, |controls| {
                f32::from_bits(controls.gain.load(Ordering::Relaxed))
            });
            let mut buffering = false;
            let mut natural_end = false;
            let mut sampler = session.map(|session| MixSampler::new(&session.audio));
            let mut project_active = session.is_some();
            let mut frames = output.chunks_exact_mut(channels);
            for frame in frames.by_ref() {
                let mut stereo = [0.0; 2];
                if project_active
                    && let (Some(controls), Some(sampler)) = (controls, sampler.as_mut())
                {
                    let reached_end = loop_range.is_none() && cursor >= controls.device_frames;
                    if reached_end
                        || !controls.playing.load(Ordering::Acquire)
                        || controls.position.load(Ordering::Acquire) != start
                    {
                        if reached_end {
                            natural_end = true;
                        }
                        project_active = false;
                    } else if let Some(sample) = sampler.sample(cursor as f64 * step) {
                        stereo = sample;
                        cursor += 1;
                        if let Some(range) = loop_range {
                            cursor = range.wrap(cursor);
                        }
                    } else {
                        // A PCM hole freezes only project playback. Audition
                        // continues independently while its next block loads.
                        buffering = true;
                        project_active = false;
                    }
                }
                let audition = if audition_active { piano.sample() } else { 0.0 };
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let value = match (channels, channel) {
                        (1, _) => (stereo[0] * 0.5 + stereo[1] * 0.5) * gain + audition,
                        (_, 0 | 1) => stereo[channel] * gain + audition,
                        _ => 0.0,
                    };
                    *sample = T::from_sample(value.clamp(-1.0, 1.0));
                }
            }
            frames.into_remainder().fill(T::EQUILIBRIUM);
            // A simultaneous seek wins over this old callback's progress.
            if let Some(controls) = controls
                && controls
                    .position
                    .compare_exchange(
                        start,
                        Controls::advanced(start, cursor),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                if natural_end {
                    controls.playing.store(false, Ordering::Release);
                }
                controls.buffering.store(
                    !natural_end && buffering && controls.playing.load(Ordering::Acquire),
                    Ordering::Release,
                );
            }
        },
        move |error| {
            use cpal::ErrorKind;
            let code = match error.kind() {
                ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => return,
                ErrorKind::Xrun => {
                    failed.underruns.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable => 1,
                ErrorKind::StreamInvalidated => 2,
                ErrorKind::UnsupportedConfig | ErrorKind::UnsupportedOperation => 3,
                ErrorKind::ResourceExhausted => 4,
                _ => 5,
            };
            failed.failure.store(code, Ordering::Release);
        },
        Some(std::time::Duration::from_secs(3)),
    )
}
