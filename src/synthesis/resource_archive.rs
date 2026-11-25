use crate::platform::fs::File;
use std::{
    io::{self, BufReader, Read},
    path::Path,
};

use zip::{
    ZipArchive,
    read::{ArchiveOffset, Config},
};

use super::voice::{Result, VoiceError};

pub(super) fn open(path: &Path, max_bytes: u64) -> Result<BufReader<File>> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(VoiceError::new("资源压缩包不是普通文件，或超出大小限制"));
    }
    Ok(BufReader::new(file))
}

pub(super) fn zip(
    reader: BufReader<File>,
    offset: u64,
    max_entries: usize,
) -> Result<ZipArchive<BufReader<File>>> {
    let mut archive = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(offset),
        },
        reader,
    )
    .map_err(zip_error)?;
    if archive.len() > max_entries
        || archive.has_duplicate_names()
        || archive.has_overlapping_entries().map_err(zip_error)?
    {
        return Err(VoiceError::new("ZIP 条目过多、重名或相互重叠"));
    }
    Ok(archive)
}

pub(super) fn copy_member(reader: &mut dyn Read, size: u64, destination: &Path) -> Result<()> {
    let mut output = File::create_new(destination)?;
    if io::copy(&mut reader.take(size + 1), &mut output)? != size {
        return Err(VoiceError::new(format!(
            "资源条目大小与压缩包记录不一致：{}",
            destination.display()
        )));
    }
    output.sync_all()?;
    Ok(())
}

pub(super) fn zip_error(error: zip::result::ZipError) -> VoiceError {
    VoiceError::new(format!("ZIP 读取失败：{error}"))
}
