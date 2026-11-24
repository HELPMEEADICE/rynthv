//! Complete export mixing and sparse live preview publication. Neither path
//! synthesizes, resamples, normalizes, or limits PCM. Pan uses a conventional
//! equal-power mono law / unity-center stereo balance.

#[cfg(feature = "gui")]
mod live;
mod recordings;
#[cfg(feature = "gui")]
pub(crate) use live::LiveMixer;
#[cfg(feature = "gui")]
pub(crate) use recordings::RecordingMix;
pub(crate) use recordings::recordings;

use crate::{
    model::{Extra, Project},
    synthesis::{
        audio::{AudioLayer, ChannelLayout, RenderedAudio},
        render_cache::PublishedOwner,
    },
};
use std::{collections::BTreeSet, ops::Range, sync::Arc};

const NATIVE_SAMPLE_RATE: u32 = 48_000;

pub(crate) fn solo_tracks(project: &Project) -> Result<BTreeSet<u64>, String> {
    super::mixing_options::validate_project_mixer(project.extra.get("projectMixer"))?;
    project
        .tracks
        .iter()
        .try_fold(BTreeSet::new(), |mut solo, track| {
            let controls = Controls::read(&track.mixer)
                .map_err(|error| format!("音轨 {}：{error}", track.name))?;
            if controls.solo {
                solo.insert(track.id);
            }
            Ok(solo)
        })
}

struct Controls {
    mute: bool,
    solo: bool,
    gain: f64,
    pan: f64,
}

struct Span<'a> {
    samples: &'a [f32],
    channels: usize,
    output_start: usize,
    frames: usize,
    left: f64,
    right: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VocalStem {
    Combined,
    Primary,
    Aspiration,
}

/// Validate owners and compute the mixed duration in seconds without
/// allocating or mixing PCM samples.
#[cfg(feature = "gui")]
pub(crate) fn mixed_duration_seconds(
    owners: &[PublishedOwner],
    solo_tracks: &BTreeSet<u64>,
) -> Result<Option<f64>, String> {
    Ok(
        visit_audible_owners(owners, solo_tracks, None, VocalStem::Combined, |_| {})?
            .map(|frames| frames as f64 / f64::from(NATIVE_SAMPLE_RATE)),
    )
}

#[cfg(feature = "gui")]
pub(crate) fn can_mix_with_recordings(
    owners: &[PublishedOwner],
    solo_tracks: &BTreeSet<u64>,
    recordings: &[AudioLayer],
    track_id: Option<u64>,
    stem: VocalStem,
) -> Result<bool, String> {
    let owner_frames = visit_audible_owners(owners, solo_tracks, track_id, stem, |_| {})?;
    let mut recording_frames = false;
    for layer in recordings {
        if track_id.is_some_and(|track| layer.track_id != track) {
            continue;
        }
        let _ = layer
            .start_frame
            .checked_add(layer.frames)
            .ok_or("伴奏混音结束位置溢出")?;
        recording_frames = true;
    }
    Ok(owner_frames.is_some() || recording_frames)
}

pub(crate) fn mix_with_recordings(
    owners: &[PublishedOwner],
    solo_tracks: &BTreeSet<u64>,
    recordings: &[AudioLayer],
    track_id: Option<u64>,
    stem: VocalStem,
    range: Option<Range<usize>>,
) -> Result<Option<Arc<RenderedAudio>>, String> {
    let mut spans = Vec::new();
    spans
        .try_reserve_exact(owners.len())
        .map_err(|error| format!("无法分配混音片段：{error}"))?;
    let mut output_frames =
        visit_audible_owners(owners, solo_tracks, track_id, stem, |span| spans.push(span))?
            .unwrap_or(0);
    for layer in recordings {
        if track_id.is_some_and(|track| layer.track_id != track) {
            continue;
        }
        let end = layer
            .start_frame
            .checked_add(layer.frames)
            .ok_or("伴奏混音结束位置溢出")?;
        output_frames = output_frames.max(end);
        let channels = layer.audio.layout().channels();
        spans.push(Span {
            samples: &layer.audio.samples()
                [layer.source_start * channels..(layer.source_start + layer.frames) * channels],
            channels,
            output_start: layer.start_frame,
            frames: layer.frames,
            left: f64::from(layer.gains[0]),
            right: f64::from(layer.gains[1]),
        });
    }
    let range = range.unwrap_or(0..output_frames);
    output_frames = range.end.saturating_sub(range.start);
    if output_frames == 0
        || !spans.iter().any(|span| {
            span.output_start < range.end && span.output_start + span.frames > range.start
        })
    {
        return Ok(None);
    }
    let sample_count = output_frames.checked_mul(2).ok_or("混音样本数量溢出")?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(sample_count)
        .map_err(|error| format!("无法分配立体声混音缓冲区：{error}"))?;
    samples.resize(sample_count, 0.0_f32);
    for span in spans {
        super::execution::checkpoint().map_err(|error| error.to_string())?;
        let start = span.output_start.max(range.start);
        let end = (span.output_start + span.frames).min(range.end);
        if start >= end {
            continue;
        }
        let destination = &mut samples[(start - range.start) * 2..(end - range.start) * 2];
        let first = (start - span.output_start) * span.channels;
        let last = (end - span.output_start) * span.channels;
        for (index, (input, output)) in span.samples[first..last]
            .chunks_exact(span.channels)
            .zip(destination.as_chunks_mut::<2>().0)
            .enumerate()
        {
            if index.is_multiple_of(65_536) {
                super::execution::checkpoint().map_err(|error| error.to_string())?;
            }
            let right = input[span.channels - 1];
            output[0] = (f64::from(output[0]) + f64::from(input[0]) * span.left) as f32;
            output[1] = (f64::from(output[1]) + f64::from(right) * span.right) as f32;
        }
    }
    RenderedAudio::new(NATIVE_SAMPLE_RATE, 0.0, ChannelLayout::Stereo, samples)
        .map(Arc::new)
        .map(Some)
        .map_err(|error| format!("混音输出无效：{error}"))
}

fn visit_audible_owners<'a>(
    owners: &'a [PublishedOwner],
    solo_tracks: &BTreeSet<u64>,
    track_id: Option<u64>,
    stem: VocalStem,
    mut visit: impl FnMut(Span<'a>),
) -> Result<Option<usize>, String> {
    let mut output_frames: Option<usize> = None;
    for owner in owners {
        if track_id.is_some_and(|track| owner.controls.track_id != track) {
            continue;
        }
        if owner.audio.sample_rate() != NATIVE_SAMPLE_RATE {
            return Err(format!(
                "音符组 {} 的 PCM 采样率为 {} Hz，混音要求所有音符组使用 48000 Hz",
                owner.owner.0,
                owner.audio.sample_rate()
            ));
        }
        let controls = Controls::read(&owner.controls.mixer)
            .map_err(|error| format!("音轨 {}：{error}", owner.controls.track_id))?;
        if controls.solo != solo_tracks.contains(&owner.controls.track_id) {
            return Err(format!(
                "音轨 {} 的独奏状态与项目混音快照不一致",
                owner.controls.track_id
            ));
        }
        if owner.controls.reference_muted
            || controls.mute
            || (!solo_tracks.is_empty() && !controls.solo)
        {
            continue;
        }
        let Some(span) = prepare_span(owner, controls, stem)? else {
            continue;
        };
        let end = span
            .output_start
            .checked_add(span.frames)
            .ok_or("混音结束位置溢出")?;
        output_frames = Some(output_frames.map_or(end, |prev| prev.max(end)));
        visit(span);
    }
    Ok(output_frames)
}

fn prepare_span(
    owner: &PublishedOwner,
    controls: Controls,
    stem: VocalStem,
) -> Result<Option<Span<'_>>, String> {
    let coverage = owner.coverage;
    if coverage.sample_rate == 0 || coverage.sample_end.0 < coverage.sample_start.0 {
        return Err(format!("音符组 {} 的音频覆盖范围无效", owner.owner.0));
    }
    // Select native sample timestamps inside the half-open project interval.
    // Integer arithmetic avoids drift when a project uses, for example, 44100 Hz.
    let first = native_bound(coverage.sample_start.0, coverage.sample_rate)?;
    let last = native_bound(coverage.sample_end.0, coverage.sample_rate)?;
    let origin = (owner.audio.start_seconds() * f64::from(NATIVE_SAMPLE_RATE)).round();
    // `as usize` saturates; check before conversion rather than accepting a
    // silently truncated placement. Vec's signed address limit is stricter.
    if !origin.is_finite() || origin < 0.0 || origin >= isize::MAX as f64 {
        return Err(format!(
            "音符组 {} 的 PCM 开始位置超出地址范围",
            owner.owner.0
        ));
    }
    let origin = origin as usize;
    let end = origin
        .checked_add(owner.audio.frames())
        .ok_or("音符组 PCM 结束位置溢出")?;
    let output_start = first.max(origin);
    let output_end = last.min(end);
    if output_end <= output_start {
        return Ok(None);
    }
    let layout = owner.audio.layout();
    let frames = output_end - output_start;
    let channels = layout.channels();
    let samples = match stem {
        VocalStem::Combined => &owner.audio.samples()
            [(output_start - origin) * channels..(output_end - origin) * channels],
        VocalStem::Primary | VocalStem::Aspiration => {
            let source = if stem == VocalStem::Primary {
                &owner.components.primary
            } else {
                &owner.components.secondary
            };
            let component_origin =
                (owner.components.start_seconds * f64::from(NATIVE_SAMPLE_RATE)).round();
            let from = usize::try_from(output_start as i128 - component_origin as i128)
                .ok()
                .and_then(|start| start.checked_add(frames).map(|end| start..end))
                .and_then(|range| source.get(range));
            from.ok_or("分离气声与已发布人声的时间范围不一致")?
        }
    };
    let (left, right) = controls.gains(owner.audio.layout());
    Ok(Some(Span {
        samples,
        channels,
        output_start,
        frames,
        left,
        right,
    }))
}

impl Controls {
    fn gains(&self, layout: ChannelLayout) -> (f64, f64) {
        let (left, right) = match layout {
            ChannelLayout::Mono => {
                let angle = (self.pan + 1.0) * std::f64::consts::FRAC_PI_4;
                (angle.cos(), angle.sin())
            }
            ChannelLayout::Stereo if self.pan < 0.0 => {
                (1.0, (self.pan * std::f64::consts::FRAC_PI_2).cos())
            }
            ChannelLayout::Stereo => ((self.pan * std::f64::consts::FRAC_PI_2).cos(), 1.0),
        };
        (self.gain * left, self.gain * right)
    }
}

fn native_bound(project_sample: i64, project_rate: u32) -> Result<usize, String> {
    let numerator = i128::from(project_sample.max(0)) * i128::from(NATIVE_SAMPLE_RATE);
    let denominator = i128::from(project_rate);
    let frame = (numerator + denominator - 1) / denominator;
    usize::try_from(frame).map_err(|_| "音符组覆盖范围超出地址范围".into())
}

impl Controls {
    fn read(extra: &Extra) -> Result<Self, String> {
        crate::synthesis::mixing_options::validate_mixer(extra)?;
        let mute = boolean(extra, "mute")?;
        let solo = boolean(extra, "solo")?;
        let decibels = number(extra, "gainDecibel")?;
        let pan = number(extra, "pan")?;
        if !(-1.0..=1.0).contains(&pan) {
            return Err("声像 pan 必须在 -1 到 1 之间".into());
        }
        let gain = 10.0_f64.powf(decibels / 20.0);
        if !(gain as f32).is_finite() {
            return Err("gainDecibel 产生了非有限增益".into());
        }
        Ok(Self {
            mute,
            solo,
            gain,
            pan,
        })
    }
}

fn boolean(extra: &Extra, field: &str) -> Result<bool, String> {
    extra
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format!("混音字段 {field} 缺失或不是布尔值"))
}

fn number(extra: &Extra, field: &str) -> Result<f64, String> {
    extra
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("混音字段 {field} 缺失或不是有限数值"))
}
