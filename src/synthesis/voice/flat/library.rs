use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::Serialize;
use uuid::Uuid;

use super::{FlatArchive, FlatModelRegistry, FlatPackage, PackageKind, archive::MAX_ARCHIVE_BYTES};
use crate::synthesis::voice::{Result, VoiceError};

const IMPORTS: &str = ".rynthv-imports";
const MAX_PROFILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct ImportedFlatVoice {
    pub database_id: String,
    pub revision: u32,
    pub name: String,
    pub path: PathBuf,
}

pub fn inspect_import(input: &Path) -> Result<Vec<super::FlatManifest>> {
    let mut archive = FlatArchive::open(input)?;
    archive.visit_packages(|_, package| Ok(package.manifest))
}

/// A batch becomes visible to library scans with one directory rename.
pub fn import(input: &Path, root: &Path) -> Result<Vec<ImportedFlatVoice>> {
    let mut archive = FlatArchive::open(input)?;
    fs::create_dir_all(root)?;
    let root = fs::canonicalize(root)?;
    let imports = imports_root(&root);
    fs::create_dir_all(&imports)?;
    let _lock = lock(&imports)?;
    let registry = FlatModelRegistry::scan(Some(&root), None);
    let mut keys = registry
        .packages
        .values()
        .map(|package| Ok((package.manifest.database_id(), package.manifest.revision()?)))
        .collect::<Result<BTreeSet<_>>>()?;
    let batch_id = Uuid::new_v4().to_string();
    let destination = imports.join(&batch_id);
    let mut staging = Staging::new(imports.join(format!(".{batch_id}")))?;
    let mut add = |staging: &mut Staging,
                   package: FlatPackage,
                   staged: StagedPackage<'_>|
     -> Result<ImportedFlatVoice> {
        let database_id = package.manifest.database_id();
        let revision = package.manifest.revision()?;
        if !keys.insert((database_id.clone(), revision)) {
            return Err(VoiceError::new(format!(
                "Flat voice {:?} version {revision} is already present",
                package.manifest.name
            )));
        }
        let name = format!(
            "{}-v{revision}.sfpk",
            database_id.trim_start_matches("flat:")
        );
        let path = staging.path.join(&name);
        match staged {
            StagedPackage::Member(bytes) => {
                let mut file = staging.create(&path)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            StagedPackage::File(input) => {
                fs::rename(input, &path)?;
                staging.files.push(path.clone());
            }
        }
        save_profile(staging, &path, &package);
        Ok(ImportedFlatVoice {
            database_id,
            revision,
            name: package.manifest.name,
            path: destination.join(name),
        })
    };
    let imported = if archive.kind() == PackageKind::Sfpk {
        let input_copy = staging.path.join("input.tmp");
        copy_package(input, &mut staging.create(&input_copy)?)?;
        let package = FlatArchive::open(&input_copy)?.describe()?;
        vec![add(
            &mut staging,
            package,
            StagedPackage::File(&input_copy),
        )?]
    } else {
        archive.visit_packages(|member, package| {
            let (_, bytes) = member.expect("batch member");
            add(&mut staging, package, StagedPackage::Member(bytes))
        })?
    };
    fs::rename(&staging.path, &destination)?;
    Ok(imported)
}

enum StagedPackage<'a> {
    Member(&'a [u8]),
    File(&'a Path),
}

pub fn remove_import(root: &Path, path: &Path) -> Result<()> {
    let imports = imports_root(&fs::canonicalize(root)?);
    let _lock = lock(&imports)?;
    let resolved = fs::canonicalize(path)?;
    if !is_managed(&imports, &resolved) || fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(VoiceError::new(
            "Flat voice is outside the managed import directory",
        ));
    }
    fs::remove_file(&resolved)?;
    let _ = fs::remove_file(resolved.with_extension("png"));
    if let Some(directory) = resolved.parent() {
        let _ = fs::remove_dir(directory);
    }
    Ok(())
}

pub(super) fn import_directories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let imports = imports_root(root);
    let entries = match fs::read_dir(imports) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.file_name().to_str().is_some_and(is_uuid) {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

pub(super) fn is_managed(imports: &Path, voice: &Path) -> bool {
    let Some(parent) = voice.parent() else {
        return false;
    };
    parent.parent() == Some(imports)
        && parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_uuid)
        && super::is_voice_path(voice)
}

pub(super) fn imports_root(root: &Path) -> PathBuf {
    root.join(IMPORTS)
}

fn is_uuid(name: &str) -> bool {
    Uuid::parse_str(name).is_ok()
}

fn lock(imports: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(imports)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(VoiceError::new("Invalid managed Flat import directory"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(imports.join(".lock"))?;
    file.lock()?;
    Ok(file)
}

fn copy_package(input: &Path, output: &mut File) -> Result<()> {
    let file = File::open(input)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(VoiceError::new("Invalid Flat package file type or size"));
    }
    let written = io::copy(&mut file.take(MAX_ARCHIVE_BYTES + 1), output)?;
    if written != metadata.len() {
        return Err(VoiceError::new("Flat package changed while copying"));
    }
    output.sync_all()?;
    Ok(())
}

fn save_profile(staging: &mut Staging, path: &Path, package: &FlatPackage) {
    if package
        .resources
        .binary_search_by(|name| name.as_str().cmp("profile.png"))
        .is_err()
    {
        return;
    }
    let result = (|| -> Result<()> {
        let png = FlatArchive::open(path)?.read("profile.png", MAX_PROFILE_BYTES)?;
        let mut reader =
            image::ImageReader::with_format(io::Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|error| VoiceError::new(error.to_string()))?;
        let icon = path.with_extension("png");
        let mut file = staging.create(&icon)?;
        let result = image
            .thumbnail(192, 192)
            .write_to(&mut file, image::ImageFormat::Png);
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(icon);
        }
        Ok(())
    })();
    let _ = result;
}

struct Staging {
    path: PathBuf,
    files: Vec<PathBuf>,
}

impl Staging {
    fn new(path: PathBuf) -> Result<Self> {
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            files: Vec::new(),
        })
    }

    fn create(&mut self, path: &Path) -> Result<File> {
        let file = File::create_new(path)?;
        self.files.push(path.to_owned());
        Ok(file)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        for path in &self.files {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&self.path);
    }
}
