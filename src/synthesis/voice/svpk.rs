use crate::platform::fs;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Result, VoiceCatalog, VoiceError, hex, nofs};
use crate::{
    platform::{config_directory, temporary},
    synthesis::resource_archive,
};

const ZIP_OFFSET: u64 = 264;
const MIB: u64 = 1024 * 1024;
const MAX_NOFS_BYTES: u64 = 2048 * MIB;
const VOICE: &str = "voice.nofs";

#[derive(Serialize)]
pub struct ImportedVoice {
    pub database_id: String,
    pub revision: u32,
    pub name: String,
    pub path: PathBuf,
}

pub(crate) fn root() -> std::result::Result<PathBuf, String> {
    config_directory().map(|path| path.join("sv1-voices"))
}

pub fn is_package(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svpk"))
}

pub fn import(input: &Path, dry_run: bool) -> Result<ImportedVoice> {
    let root = root().map_err(VoiceError::new)?;
    let staging = if dry_run {
        temporary::create(None, ".svpk-")?
    } else {
        let config = root.parent().expect("voice root has a parent");
        fs::create_dir_all(config)?;
        temporary::create(Some(config), ".svpk-")?
    };
    unpack(input, staging.path())?;
    let mut archive = nofs::NofsArchive::open(&staging.path().join(VOICE))?;
    let voice = nofs::describe(&mut archive)?.configuration;
    drop(archive);
    let revision = voice
        .version
        .parse::<u32>()
        .expect("validated NOFS version");
    let mut digest = Sha256::new();
    digest.update((voice.vendor.len() as u64).to_le_bytes());
    digest.update(voice.vendor.as_bytes());
    digest.update(voice.name.as_bytes());
    let id = hex(&digest.finalize()[..16]);
    let destination = root.join(revision.to_string()).join(&id);
    if fs::try_exists(&destination)? {
        return Err(VoiceError::new(format!(
            "声库 {} 的 {revision} 版已导入",
            voice.name
        )));
    }
    if !dry_run {
        fs::create_dir_all(
            destination
                .parent()
                .expect("voice has a revision directory"),
        )?;
        fs::rename(staging.path(), &destination)?;
    }
    Ok(ImportedVoice {
        database_id: format!("sv1:{id}"),
        revision,
        name: voice.name,
        path: destination.join(VOICE),
    })
}

pub(crate) fn scan(root: &Path) -> Result<VoiceCatalog> {
    let mut catalog = VoiceCatalog::default();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<u32>().is_ok())
        {
            let found = VoiceCatalog::scan(&entry.path())?;
            catalog.entries.extend(found.entries);
            catalog.warnings.extend(found.warnings);
        }
    }
    Ok(catalog)
}

fn unpack(input: &Path, destination: &Path) -> Result<()> {
    let mut reader = resource_archive::open(input, MAX_NOFS_BYTES + 64 * MIB)?;
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    reader.seek(SeekFrom::Start(ZIP_OFFSET))?;
    let mut signature = [0; 4];
    reader.read_exact(&mut signature)?;
    if header != *b"SVPK\0\x01\0\0" || signature != *b"PK\x03\x04" {
        return Err(VoiceError::new("无法识别的 SVPK 文件头"));
    }
    let mut archive = resource_archive::zip(reader, ZIP_OFFSET, 128)?;
    if archive.index_for_name(VOICE).is_none() {
        return Err(VoiceError::new(
            "SVPK 缺少 voice.nofs，仅支持原生 SV1 AI 声库",
        ));
    }
    for (name, max_bytes) in [
        (VOICE, MAX_NOFS_BYTES),
        ("profile.png", 16 * MIB),
        ("background.png", 16 * MIB),
        ("license.txt", MIB),
        ("package-config", MIB),
    ] {
        if archive.index_for_name(name).is_some() {
            let mut file = archive.by_name(name).map_err(resource_archive::zip_error)?;
            let size = file.size();
            if !file.is_file() || file.is_symlink() || size > max_bytes {
                return Err(VoiceError::new(format!("SVPK 资源条目无效：{name}")));
            }
            resource_archive::copy_member(&mut file, size, &destination.join(name))?;
        }
    }
    Ok(())
}
