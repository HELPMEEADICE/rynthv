mod members;

use std::{io, path::Path};

use crate::platform::{
    fs,
    temporary::{self, TempDir},
};
use sevenz_rust2::{ArchiveReader, Password};

use super::{
    resource_archive,
    voice::{Result, VoiceError},
};
use members::{Member, select};

const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_DECODER_KIB: usize = 128 * 1024;

pub fn prepare(input: &Path, dry_run: bool) -> Result<TempDir> {
    let extension = input.extension().and_then(|value| value.to_str());
    let extract = match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("zip") => unpack_zip,
        Some("7z") => unpack_7z,
        _ => return Err(VoiceError::new("请选择 clf-data 的 ZIP 或 7z 压缩包")),
    };
    let directory = if dry_run {
        temporary::create(None, "import-")?
    } else {
        let root = crate::platform::config_directory()
            .map_err(VoiceError::new)?
            .join("clf-data");
        fs::create_dir_all(&root)?;
        temporary::create(Some(&root), "import-")?
    };
    extract(input, directory.path())?;
    Ok(directory)
}

fn unpack_zip(input: &Path, directory: &Path) -> Result<()> {
    let reader = resource_archive::open(input, MAX_ARCHIVE_BYTES)?;
    let mut archive = resource_archive::zip(reader, 0, MAX_ENTRIES)?;
    let members = (0..archive.len())
        .map(|index| {
            let file = archive
                .by_index_raw(index)
                .map_err(resource_archive::zip_error)?;
            Ok(Member {
                index,
                name: file
                    .name()
                    .map_err(resource_archive::zip_error)?
                    .into_owned(),
                size: file.size(),
                directory: file.is_dir(),
                regular: !file.is_symlink() && !file.encrypted(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for (name, member) in select(&members)? {
        let mut file = archive
            .by_index(member.index)
            .map_err(resource_archive::zip_error)?;
        resource_archive::copy_member(&mut file, member.size, &directory.join(name))?;
    }
    Ok(())
}

fn unpack_7z(input: &Path, directory: &Path) -> Result<()> {
    let reader = resource_archive::open(input, MAX_ARCHIVE_BYTES)?;
    let mut archive =
        ArchiveReader::new_with_memory_limit(reader, Password::empty(), MAX_DECODER_KIB)
            .map_err(seven_zip_error)?;
    archive.set_thread_count(1);
    let metadata = archive.archive();
    check_size(
        metadata.files.len(),
        metadata.files.iter().map(|entry| entry.size),
    )?;
    let members: Vec<_> = metadata
        .files
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let unix_type = (entry.windows_attributes >> 16) & 0xf000;
            Member {
                index,
                name: entry.name.clone(),
                size: entry.size,
                directory: entry.is_directory,
                regular: !entry.is_anti_item
                    && entry.windows_attributes & 0x400 == 0
                    && matches!(unix_type, 0 | 0x8000)
                    && !metadata.is_file_encrypted(index),
            }
        })
        .collect();
    let mut files: std::collections::HashMap<_, _> = select(&members)?
        .into_iter()
        .map(|(name, member)| (member.name.as_str(), (name, member.size)))
        .collect();
    archive
        .for_each_entries_with_decision_allowing_surplus(|entry, reader| {
            if !entry.is_directory
                && let Some((name, size)) = files.remove(entry.name())
            {
                resource_archive::copy_member(reader, size, &directory.join(name))
                    .map_err(io::Error::other)?;
            }
            Ok(sevenz_rust2::EntryDecision::Continue)
        })
        .map_err(seven_zip_error)?;
    if !files.is_empty() {
        return Err(VoiceError::new("7z 压缩包缺少声明的词典文件"));
    }
    Ok(())
}

fn check_size(entries: usize, mut sizes: impl Iterator<Item = u64>) -> Result<()> {
    if entries > MAX_ENTRIES
        || sizes
            .try_fold(0u64, |total, size| {
                total
                    .checked_add(size)
                    .filter(|&total| total <= MAX_ARCHIVE_BYTES)
            })
            .is_none()
    {
        return Err(VoiceError::new("词典压缩包解包后超出大小或条目数限制"));
    }
    Ok(())
}

fn seven_zip_error(error: sevenz_rust2::Error) -> VoiceError {
    VoiceError::new(format!("7z 读取失败：{error}"))
}
