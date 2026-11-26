use super::{Controls, NATIVE_SAMPLE_RATE};
use crate::{
    model::Track,
    synthesis::{audio::AudioLayer, instrumental::AudioAsset},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
    sync::Arc,
};
use crate::{trf};

#[derive(Default)]
pub(crate) struct RecordingMix {
    pub layers: Vec<AudioLayer>,
    pub pending: BTreeMap<u64, Range<usize>>,
    pub failures: BTreeMap<u64, String>,
    pub end: usize,
}

pub(crate) fn recordings<'a, 'track>(
    tracks: impl IntoIterator<Item = &'track Track>,
    solo: &BTreeSet<u64>,
    asset: impl Fn(&Path) -> Option<Result<&'a AudioAsset, &'a str>>,
) -> Result<RecordingMix, String> {
    let mut mix = RecordingMix::default();
    let sample_rate = f64::from(NATIVE_SAMPLE_RATE);
    for track in tracks {
        let Some(clip) = track.audio_clip() else {
            continue;
        };
        if !clip.valid() {
            return Err(trf!("伴奏音轨 {} 的片段时间无效", track.name));
        }
        let controls = Controls::read(&track.mixer)?;
        if controls.mute || (!solo.is_empty() && !solo.contains(&track.id)) {
            continue;
        }
        let start = (clip.onset_seconds * sample_rate).round() as usize;
        let requested_end =
            start.saturating_add((clip.duration_seconds * sample_rate).round() as usize);
        let available = asset(&clip.path);
        if let Some(Ok(asset)) = available {
            let source_start = (clip.source_offset_seconds * sample_rate).round() as usize;
            let frames = asset
                .audio
                .frames()
                .saturating_sub(source_start)
                .min((clip.duration_seconds * sample_rate).round() as usize);
            if frames == 0 {
                continue;
            }
            let (left, right) = controls.gains(asset.audio.layout());
            let gains = [left as f32, right as f32];
            if gains.iter().any(|gain| !gain.is_finite()) {
                return Err(trf!("音轨 {} 的音量超出有效范围", track.name));
            }
            mix.end = mix.end.max(start.saturating_add(frames));
            mix.layers.push(AudioLayer {
                track_id: track.id,
                audio: Arc::clone(&asset.audio),
                start_frame: start,
                source_start,
                frames,
                gains,
            });
        } else if let Some(Err(error)) = available {
            mix.failures.insert(
                track.id,
                trf!("伴奏音轨 {} 加载失败：{}", track.name, error),
            );
        } else {
            mix.end = mix.end.max(requested_end);
            mix.pending.insert(track.id, start..requested_end);
        }
    }
    Ok(mix)
}
