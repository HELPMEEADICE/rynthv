use super::{AudioError, ChannelLayout, RenderedAudio, conversion};
use std::{io::Write, path::Path};

#[derive(Clone, Copy)]
pub struct WaveFormat {
    pub sample_rate: u32,
    pub layout: ChannelLayout,
    pub bit_depth: u16,
}

/// Native float output used by the synthesis CLI, without normalization.
pub fn save_wav(path: &Path, audio: &RenderedAudio) -> Result<(), AudioError> {
    crate::storage::atomic_write_policy(path, true, |writer| encode(writer, audio, 32))
        .map_err(AudioError::Wave)
}

pub fn save_wav_as(
    path: &Path,
    audio: &RenderedAudio,
    format: WaveFormat,
) -> Result<(), AudioError> {
    save_wav_policy(path, audio, format, true)
}

pub fn save_wav_policy(
    path: &Path,
    audio: &RenderedAudio,
    format: WaveFormat,
    overwrite: bool,
) -> Result<(), AudioError> {
    if ![44100, 48000, 96000].contains(&format.sample_rate) {
        return Err(AudioError::Wave("不支持的导出格式".into()));
    }
    let converted = convert_format(audio, format)?;
    let audio = converted.as_ref().unwrap_or(audio);
    crate::storage::atomic_write_policy(path, overwrite, |writer| {
        encode(writer, audio, format.bit_depth)
    })
    .map_err(AudioError::Wave)
}

pub fn write_wav(
    mut writer: impl Write,
    audio: &RenderedAudio,
    format: WaveFormat,
) -> Result<(), AudioError> {
    let converted = convert_format(audio, format)?;
    encode(
        &mut writer,
        converted.as_ref().unwrap_or(audio),
        format.bit_depth,
    )
    .map_err(AudioError::Wave)
}

fn convert_format(
    audio: &RenderedAudio,
    format: WaveFormat,
) -> Result<Option<RenderedAudio>, AudioError> {
    if ![16, 24, 32].contains(&format.bit_depth) {
        return Err(AudioError::Wave("不支持的导出格式".into()));
    }
    if audio.sample_rate() == format.sample_rate && audio.layout() == format.layout {
        return Ok(None);
    }
    conversion::convert(audio, format.sample_rate, format.layout).map(Some)
}

fn encode(writer: &mut impl Write, audio: &RenderedAudio, depth: u16) -> Result<(), String> {
    let float = depth == 32;
    let header_bytes = if float { 58 } else { 44 };
    let data_bytes = audio
        .samples()
        .len()
        .checked_mul(usize::from(depth / 8))
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or("PCM exceeds RIFF's 4 GiB limit")?;
    let padding = data_bytes % 2;
    let riff_bytes = data_bytes
        .checked_add(header_bytes - 8 + padding)
        .ok_or("PCM exceeds RIFF's 4 GiB limit")?;
    let channels = audio.layout().channels() as u16;
    let block_align = channels * (depth / 8);
    let mut write = |bytes: &[u8]| writer.write_all(bytes).map_err(|e| e.to_string());
    write(b"RIFF")?;
    write(&riff_bytes.to_le_bytes())?;
    write(b"WAVEfmt ")?;
    write(&(if float { 18u32 } else { 16u32 }).to_le_bytes())?;
    write(&(if float { 3u16 } else { 1u16 }).to_le_bytes())?;
    write(&channels.to_le_bytes())?;
    write(&audio.sample_rate().to_le_bytes())?;
    write(&(audio.sample_rate() * u32::from(block_align)).to_le_bytes())?;
    write(&block_align.to_le_bytes())?;
    write(&depth.to_le_bytes())?;
    if float {
        write(&0u16.to_le_bytes())?;
        write(b"fact")?;
        write(&4u32.to_le_bytes())?;
        write(&(audio.frames() as u32).to_le_bytes())?;
    }
    write(b"data")?;
    write(&data_bytes.to_le_bytes())?;
    match depth {
        16 => write_converted(audio.samples(), &mut write, |sample| {
            ((f64::from(sample) * 32768.0)
                .round()
                .clamp(-32768.0, 32767.0) as i16)
                .to_le_bytes()
        })?,
        24 => write_converted(audio.samples(), &mut write, |sample| {
            let [low, mid, high, _] = ((f64::from(sample) * 8388608.0)
                .round()
                .clamp(-8388608.0, 8388607.0) as i32)
                .to_le_bytes();
            [low, mid, high]
        })?,
        #[cfg(target_endian = "little")]
        32 => {
            for samples in audio.samples().chunks(16_384) {
                crate::synthesis::execution::checkpoint().map_err(|error| error.to_string())?;
                write(bytemuck::cast_slice(samples))?;
            }
        }
        #[cfg(target_endian = "big")]
        32 => write_converted(audio.samples(), &mut write, f32::to_le_bytes)?,
        _ => return Err("不支持的导出位深".into()),
    }
    if padding != 0 {
        write(&[0])?;
    }
    crate::synthesis::execution::checkpoint().map_err(|error| error.to_string())
}

fn write_converted<const N: usize>(
    samples: &[f32],
    write: &mut impl FnMut(&[u8]) -> Result<(), String>,
    convert: impl Fn(f32) -> [u8; N],
) -> Result<(), String> {
    let mut buffer = [0_u8; 65_536];
    let (frames, _) = buffer.as_chunks_mut::<N>();
    for samples in samples.chunks(frames.len()) {
        crate::synthesis::execution::checkpoint().map_err(|error| error.to_string())?;
        for (target, &sample) in frames.iter_mut().zip(samples) {
            *target = convert(sample);
        }
        write(bytemuck::cast_slice(&frames[..samples.len()]))?;
    }
    Ok(())
}
