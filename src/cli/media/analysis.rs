use crate::synthesis::audio::RenderedAudio;
use serde_json::{Value, json};
use std::ops::Range;

pub(super) fn fundamental(
    audio: &RenderedAudio,
    cancelled: &std::sync::atomic::AtomicBool,
) -> super::Result<Value> {
    use std::sync::atomic::Ordering;
    // Average blocks before decimation, then use a normalized difference
    // function on one decoded channel to estimate the dominant period.
    let stride = (audio.sample_rate() as usize / 8_000).max(1);
    let rate = f64::from(audio.sample_rate()) / stride as f64;
    let channels = audio.layout().channels();
    let window = (rate * 0.04).round() as usize;
    let step = (rate * 0.02).round().max(1.0) as usize;
    let first = (rate / 1500.0).ceil().max(2.0) as usize;
    let last = (rate / 60.0).floor() as usize + 1;
    if window < 2 || first >= last {
        return Err(super::invalid(
            "Sample rate is too low for the F0 search range",
        ));
    }
    let mut frames = Vec::new();
    let mut decimated = vec![0.0; window + last + 1];
    let mut difference = vec![1.0; last + 1];
    let block = decimated.len() * stride;
    if audio.frames() >= block {
        for frame in (0..=audio.frames() - block).step_by(step * stride) {
            if cancelled.load(Ordering::Acquire) {
                return Err(super::Failure::new("CANCELLED", "Audio analysis cancelled"));
            }
            let start = frame * channels;
            let stop = (frame + block) * channels;
            for (sample, input) in decimated
                .iter_mut()
                .zip(audio.samples()[start..stop].chunks_exact(stride * channels))
            {
                *sample = input
                    .iter()
                    .step_by(channels)
                    .map(|&value| f64::from(value))
                    .sum::<f64>()
                    / stride as f64;
            }
            let detected = dominant_period(&decimated, window, &mut difference, first, rate);
            frames.push(json!({"time_seconds":frame as f64 / f64::from(audio.sample_rate()) + window as f64/(2.0*rate),
                "frequency_hz":detected.map(|(hz,_)| hz), "periodicity":detected.map(|(_,confidence)| confidence)}));
        }
    }
    Ok(
        json!({"source":"decoded_pcm","algorithm":"normalized difference with local period interpolation",
        "channel":0,"frequency_range_hz":[60,1500],"window_seconds":window as f64/rate,
        "hop_seconds":step as f64/rate,"frames":frames}),
    )
}

fn dominant_period(
    samples: &[f64],
    window: usize,
    difference: &mut [f64],
    first: usize,
    rate: f64,
) -> Option<(f64, f64)> {
    let base = &samples[..window];
    let energy = base.iter().map(|value| value * value).sum::<f64>() / window as f64;
    if energy < 0.000_001 {
        return None;
    }
    let mut cumulative = 0.0;
    for (lag, value) in difference.iter_mut().enumerate().skip(1) {
        let delta = base
            .iter()
            .zip(&samples[lag..lag + window])
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f64>();
        cumulative += delta;
        *value = if cumulative == 0.0 {
            1.0
        } else {
            delta * lag as f64 / cumulative
        };
    }
    let lag = (first..difference.len() - 1).find(|&lag| {
        difference[lag] < 0.15
            && difference[lag] <= difference[lag - 1]
            && difference[lag] <= difference[lag + 1]
    })?;
    let [a, b, c] = [difference[lag - 1], difference[lag], difference[lag + 1]];
    let curvature = a - 2.0 * b + c;
    let correction = if curvature > 0.0 {
        (0.5 * (a - c) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let hz = rate / (lag as f64 + correction);
    (60.0..=1500.0)
        .contains(&hz)
        .then_some((hz, (1.0 - b).clamp(0.0, 1.0)))
}

pub(super) fn measure(audio: &RenderedAudio) -> Value {
    let channels = audio.layout().channels();
    let rate = f64::from(audio.sample_rate());
    let minimum = audio.sample_rate().div_ceil(20) as usize;
    const SILENCE: f64 = 0.001;
    let mut silence = Vec::<Range<usize>>::new();
    let mut start = None;
    let mut rms_square = vec![0.0; channels];
    let mut peaks = vec![0.0_f64; channels];
    let mut full_scale = vec![0; channels];
    let mut over = vec![0; channels];
    for (index, frame) in audio.samples().chunks_exact(channels).enumerate() {
        let mut peak = 0.0_f64;
        for (channel, &sample) in frame.iter().enumerate() {
            let value = f64::from(sample).abs();
            peak = peak.max(value);
            rms_square[channel] += value * value;
            peaks[channel] = peaks[channel].max(value);
            full_scale[channel] += usize::from(value >= 1.0);
            over[channel] += usize::from(value > 1.0);
        }
        if peak < SILENCE {
            start.get_or_insert(index);
        } else if let Some(start) = start.take()
            && index - start >= minimum
        {
            silence.push(start..index);
        }
    }
    if let Some(start) = start
        && audio.frames() - start >= minimum
    {
        silence.push(start..audio.frames());
    }
    let intervals: Vec<_> = silence
        .into_iter()
        .map(|range| {
            json!({
                "from_seconds":range.start as f64 / rate,"to_seconds":range.end as f64 / rate
            })
        })
        .collect();
    let rms: Vec<_> = rms_square
        .iter()
        .map(|value| (value / audio.frames() as f64).sqrt())
        .collect();
    let dbfs = |value: f64| (value > 0.0).then(|| value.log10() * 20.0);
    json!({"source":"decoded_pcm","sample_rate":audio.sample_rate(),"channels":channels,"frames":audio.frames(),
        "duration_seconds":audio.duration_seconds(),"rms":rms,"peak":peaks,"rms_dbfs":rms.into_iter().map(dbfs).collect::<Vec<_>>(),
        "peak_dbfs":peaks.into_iter().map(dbfs).collect::<Vec<_>>(),"samples_at_or_above_full_scale":full_scale,
        "samples_above_full_scale":over,"silence_threshold_dbfs":-60,"minimum_silence_seconds":0.05,"silence":intervals})
}
