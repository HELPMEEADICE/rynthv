//! Offline, band-limited rate conversion. Preview retains native PCM.
use super::{AudioError, ChannelLayout, RenderedAudio};

pub(crate) fn convert(
    audio: &RenderedAudio,
    rate: u32,
    layout: ChannelLayout,
) -> Result<RenderedAudio, AudioError> {
    convert_samples(
        audio.sample_rate(),
        audio.start_seconds(),
        audio.layout(),
        audio.samples(),
        rate,
        layout,
    )
}

pub(crate) fn convert_samples(
    source_rate: u32,
    start_seconds: f64,
    source_layout: ChannelLayout,
    input: &[f32],
    rate: u32,
    layout: ChannelLayout,
) -> Result<RenderedAudio, AudioError> {
    let source_channels = source_layout.channels();
    let source_frames = input.len() / source_channels;
    let channels = layout.channels();
    let frames = (source_frames as u128 * u128::from(rate)).div_ceil(u128::from(source_rate));
    let count = usize::try_from(frames)
        .ok()
        .and_then(|n| n.checked_mul(channels))
        .ok_or_else(|| AudioError::Wave("导出音频长度溢出".into()))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|e| AudioError::Wave(e.to_string()))?;
    let sample = |frame: usize, channel: usize| {
        let frame_slice = &input[frame * source_channels..][..source_channels];
        if channels == 1 && source_channels == 2 {
            (f64::from(frame_slice[0]) + f64::from(frame_slice[1])) * 0.5
        } else {
            f64::from(frame_slice[channel.min(source_channels - 1)])
        }
    };
    if rate == source_rate {
        for frame in 0..source_frames {
            checkpoint(frame)?;
            for channel in 0..channels {
                samples.push(sample(frame, channel) as f32);
            }
        }
    } else {
        const RADIUS: i64 = 32;
        const TAPS: usize = (RADIUS as usize) * 2;
        const PHASES: usize = 1024;
        let cutoff = (f64::from(rate) / f64::from(source_rate)).min(1.0) * 0.96;
        let mut kernels = Vec::with_capacity((PHASES + 1) * TAPS);
        for phase in 0..=PHASES {
            let fraction = phase as f64 / PHASES as f64;
            let start = kernels.len();
            let mut sum = 0.0;
            for tap in -RADIUS + 1..=RADIUS {
                let x = tap as f64 - fraction;
                let z = std::f64::consts::PI * x * cutoff;
                let sinc = if z.abs() < 1e-12 { 1.0 } else { z.sin() / z };
                let window = 0.42
                    + 0.5 * (std::f64::consts::PI * x / RADIUS as f64).cos()
                    + 0.08 * (std::f64::consts::TAU * x / RADIUS as f64).cos();
                let weight = cutoff * sinc * window;
                sum += weight;
                kernels.push(weight);
            }
            for weight in &mut kernels[start..] {
                *weight /= sum;
            }
        }
        let ratio = f64::from(source_rate) / f64::from(rate);
        for frame in 0..count / channels {
            checkpoint(frame)?;
            let position = frame as f64 * ratio;
            let base = position.floor() as i64;
            let phase = (position.fract() * PHASES as f64).round() as usize;
            let kernel = &kernels[phase * TAPS..(phase + 1) * TAPS];
            let first = base - RADIUS + 1;
            let last = base + RADIUS;
            if first >= 0 && (last as usize) < source_frames {
                let base_frame = first as usize;
                for channel in 0..channels {
                    let mut value = 0.0;
                    for (tap, &weight) in kernel.iter().enumerate() {
                        value += sample(base_frame + tap, channel) * weight;
                    }
                    samples.push(value as f32);
                }
            } else {
                for channel in 0..channels {
                    let mut value = 0.0;
                    for (tap, &weight) in kernel.iter().enumerate() {
                        let source = first + tap as i64;
                        if source >= 0 && (source as usize) < source_frames {
                            value += sample(source as usize, channel) * weight;
                        }
                    }
                    samples.push(value as f32);
                }
            }
        }
    }
    RenderedAudio::new(rate, start_seconds, layout, samples)
}

fn checkpoint(frame: usize) -> Result<(), AudioError> {
    if frame.is_multiple_of(4096) {
        crate::synthesis::execution::checkpoint()
            .map_err(|error| AudioError::Wave(error.to_string()))?;
    }
    Ok(())
}
