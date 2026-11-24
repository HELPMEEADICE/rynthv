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
    encode(path, audio, 32, true)
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
    if ![44100, 48000, 96000].contains(&format.sample_rate)
        || ![16, 24, 32].contains(&format.bit_depth)
    {
        return Err(AudioError::Wave("不支持的导出格式".into()));
    }
    if audio.sample_rate() == format.sample_rate && audio.layout() == format.layout {
        encode(path, audio, format.bit_depth, overwrite)
    } else {
        encode(
            path,
            &conversion::convert(audio, format.sample_rate, format.layout)?,
            format.bit_depth,
            overwrite,
        )
    }
}

fn encode(
    path: &Path,
    audio: &RenderedAudio,
    depth: u16,
    overwrite: bool,
) -> Result<(), AudioError> {
    let float = depth == 32;
    let header_bytes = if float { 58 } else { 44 };
    let data_bytes = audio
        .samples()
        .len()
        .checked_mul(usize::from(depth / 8))
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(|| AudioError::Wave("PCM exceeds RIFF's 4 GiB limit".into()))?;
    let padding = data_bytes % 2;
    let riff_bytes = data_bytes
        .checked_add(header_bytes - 8 + padding)
        .ok_or_else(|| AudioError::Wave("PCM exceeds RIFF's 4 GiB limit".into()))?;
    let channels = audio.layout().channels() as u16;
    let block_align = channels * (depth / 8);
    crate::storage::atomic_write_policy(path, overwrite, |writer| {
        let write = |writer: &mut std::io::BufWriter<std::fs::File>, bytes: &[u8]| {
            writer.write_all(bytes).map_err(|e| e.to_string())
        };
        write(writer, b"RIFF")?;
        write(writer, &riff_bytes.to_le_bytes())?;
        write(writer, b"WAVEfmt ")?;
        write(writer, &(if float { 18u32 } else { 16u32 }).to_le_bytes())?;
        write(writer, &(if float { 3u16 } else { 1u16 }).to_le_bytes())?;
        write(writer, &channels.to_le_bytes())?;
        write(writer, &audio.sample_rate().to_le_bytes())?;
        write(
            writer,
            &(audio.sample_rate() * u32::from(block_align)).to_le_bytes(),
        )?;
        write(writer, &block_align.to_le_bytes())?;
        write(writer, &depth.to_le_bytes())?;
        if float {
            write(writer, &0u16.to_le_bytes())?;
            write(writer, b"fact")?;
            write(writer, &4u32.to_le_bytes())?;
            write(writer, &(audio.frames() as u32).to_le_bytes())?;
        }
        write(writer, b"data")?;
        write(writer, &data_bytes.to_le_bytes())?;
        let mut chunk = [0u8; 4096];
        let mut cursor = 0usize;
        let sample_bytes = usize::from(depth / 8);
        for &sample in audio.samples() {
            if cursor + sample_bytes > chunk.len() {
                crate::synthesis::execution::checkpoint().map_err(|error| error.to_string())?;
                write(writer, &chunk[..cursor])?;
                cursor = 0;
            }
            match depth {
                16 => {
                    let encoded = ((f64::from(sample) * 32768.0)
                        .round()
                        .clamp(-32768.0, 32767.0) as i16)
                        .to_le_bytes();
                    chunk[cursor..cursor + 2].copy_from_slice(&encoded);
                    cursor += 2;
                }
                24 => {
                    let encoded = ((f64::from(sample) * 8388608.0)
                        .round()
                        .clamp(-8388608.0, 8388607.0) as i32)
                        .to_le_bytes();
                    chunk[cursor..cursor + 3].copy_from_slice(&encoded[..3]);
                    cursor += 3;
                }
                _ => {
                    chunk[cursor..cursor + 4].copy_from_slice(&sample.to_le_bytes());
                    cursor += 4;
                }
            }
        }
        if cursor > 0 {
            write(writer, &chunk[..cursor])?;
        }
        if padding != 0 {
            write(writer, &[0])?;
        }
        crate::synthesis::execution::checkpoint().map_err(|error| error.to_string())
    })
    .map_err(AudioError::Wave)
}
