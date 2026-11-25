use super::AudioAsset;
use crate::platform::fs::File;
use crate::synthesis::{
    audio::{ChannelLayout, RenderedAudio, conversion},
    rendered::Waveform,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    formats::{TrackType, probe::Hint},
    io::MediaSourceStream,
};

const MAX_SAMPLES: usize = 512 * 1024 * 1024;

#[cfg(target_arch = "wasm32")]
impl symphonia::core::io::MediaSource for File {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.metadata().ok().map(|metadata| metadata.len())
    }
}

pub(super) fn decode(path: &Path, cancelled: &AtomicBool) -> Result<RenderedAudio, String> {
    let file = File::open(path).map_err(|e| format!("无法打开音频文件：{e}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, stream, Default::default(), Default::default())
        .map_err(|e| e.to_string())?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("文件不包含可解码的音频")?;
    let id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("音频编码信息缺失")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut spec = None;
    let mut samples = Vec::<f32>::new();
    while let Some(packet) = format.next_packet().map_err(|e| e.to_string())? {
        if cancelled.load(Ordering::Relaxed) {
            return Err("音频加载已取消".into());
        }
        if packet.track_id != id {
            continue;
        }
        let audio = decoder
            .decode(&packet)
            .map_err(|e| format!("解码失败：{e}"))?;
        if audio.samples_interleaved() == 0 {
            continue;
        }
        if spec.as_ref().is_some_and(|s| s != audio.spec()) {
            return Err("不支持采样率或声道数在文件中途变化的音频".into());
        }
        let channels = audio.spec().channels().count();
        if !(1..=2).contains(&channels) {
            return Err("目前支持单声道和立体声音频".into());
        }
        if !(1..=384_000).contains(&audio.spec().rate()) {
            return Err("音频采样率不在 1..=384000 Hz 范围内".into());
        }
        spec = Some(audio.spec().clone());
        let count = audio.samples_interleaved();
        let start = samples.len();
        let end = start
            .checked_add(count)
            .filter(|n| *n <= MAX_SAMPLES)
            .filter(|n| {
                let source_frames = (*n / channels) as u128;
                let resampled_frames = source_frames
                    .saturating_mul(48_000)
                    .div_ceil(u128::from(audio.spec().rate()));
                resampled_frames.saturating_mul(channels as u128) <= MAX_SAMPLES as u128
            })
            .ok_or("音频解码后超过 2 GiB，请拆分文件后导入")?;
        samples
            .try_reserve(count)
            .map_err(|e| format!("音频内存不足：{e}"))?;
        samples.resize(end, 0.0);
        audio.copy_to_slice_interleaved(&mut samples[start..]);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err("音频加载已取消".into());
    }
    let spec = spec.ok_or("音频文件为空")?;
    let layout = if spec.channels().count() == 1 {
        ChannelLayout::Mono
    } else {
        ChannelLayout::Stereo
    };
    RenderedAudio::new(spec.rate(), 0.0, layout, samples).map_err(|e| e.to_string())
}

pub(super) fn load(
    path: &Path,
    cancelled: &AtomicBool,
    waveform: bool,
) -> Result<AudioAsset, String> {
    let source = decode(path, cancelled)?;
    let audio = Arc::new(if source.sample_rate() == 48_000 {
        source
    } else {
        conversion::convert(&source, 48_000, source.layout()).map_err(|e| e.to_string())?
    });
    let waveforms = (0..if waveform {
        audio.layout().channels()
    } else {
        0
    })
        .map(|c| Waveform::new_channel(&audio, c))
        .collect();
    Ok(AudioAsset { audio, waveforms })
}
