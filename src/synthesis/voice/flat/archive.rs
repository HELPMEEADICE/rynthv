use crate::platform::fs::File;
use std::{
    collections::BTreeMap,
    io::{BufReader, Cursor, Read, Seek, SeekFrom},
    path::Path,
};

use serde::Serialize;
use zip::{CompressionMethod, ZipArchive};

use super::{FlatManifest, FlatPackage, PackageConfig};
use crate::synthesis::voice::{Result, VoiceError};

pub(super) const MAX_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_BATCH_CONTENT: u64 = 4 * 1024 * 1024 * 1024;
const MAX_VOICE_CONTENT: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_CHILD_BYTES: usize = 256 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKind {
    Sfpk,
    Sfpks,
}

pub struct FlatArchive<R = BufReader<File>> {
    kind: PackageKind,
    zip: ZipArchive<R>,
    files: BTreeMap<String, u64>,
}

impl FlatArchive {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
            return Err(VoiceError::new("Invalid Flat package file type or size"));
        }
        Self::read_zip(BufReader::new(file))
            .map_err(|error| VoiceError::new(format!("Flat package {}: {error}", path.display())))
    }
}

impl<R: Read + Seek> FlatArchive<R> {
    fn read_zip(mut reader: R) -> Result<Self> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        if magic != *b"PK\x03\x04" {
            return Err(VoiceError::new(format!(
                "Unsupported Flat package magic {:02x?}; expected a plaintext ZIP",
                magic
            )));
        }
        reader.seek(SeekFrom::Start(0))?;
        let mut zip = ZipArchive::new(reader).map_err(zip_error)?;
        if zip.len() > MAX_ENTRIES || zip.has_duplicate_names() {
            return Err(VoiceError::new(
                "Flat ZIP contains too many entries or duplicate names",
            ));
        }
        if zip.has_overlapping_entries().map_err(zip_error)? {
            return Err(VoiceError::new("Flat ZIP contains overlapping entries"));
        }
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let file = zip.by_index_raw(index).map_err(zip_error)?;
            let name = std::str::from_utf8(file.name_raw())
                .map_err(|_| VoiceError::new("Flat ZIP entry name is not UTF-8"))?;
            validate_path(name.trim_end_matches('/'))?;
            if file.is_symlink() || file.encrypted() {
                return Err(VoiceError::new(format!(
                    "Flat ZIP entry {name:?} is encrypted or a symbolic link"
                )));
            }
            if file.is_dir() {
                continue;
            }
            if !matches!(
                file.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ) {
                return Err(VoiceError::new(format!(
                    "Unsupported ZIP compression for {name:?}"
                )));
            }
            total = total
                .checked_add(file.size())
                .filter(|&sum| sum <= MAX_BATCH_CONTENT)
                .ok_or_else(|| VoiceError::new("Flat ZIP exceeds expanded size limit"))?;
            if files.insert(name.to_owned(), file.size()).is_some() {
                return Err(VoiceError::new(format!(
                    "Duplicate Flat ZIP entry {name:?}"
                )));
            }
        }
        let has_voice = files.contains_key("info.nofs") || files.contains_key("voice.nofs");
        let has_children = files.keys().any(|name| is_sfpk(Path::new(name)));
        let kind = match (has_voice, has_children) {
            (true, false) if total <= MAX_VOICE_CONTENT => PackageKind::Sfpk,
            (true, false) => return Err(VoiceError::new("Expanded SFPK exceeds 1 GiB")),
            (false, true) => PackageKind::Sfpks,
            _ => {
                return Err(VoiceError::new(
                    "Flat ZIP does not identify one voice or an SFPK batch",
                ));
            }
        };
        Ok(Self { kind, zip, files })
    }

    pub fn kind(&self) -> PackageKind {
        self.kind
    }

    pub fn contains(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }

    pub fn read(&mut self, name: &str, limit: usize) -> Result<Vec<u8>> {
        let length = self
            .files
            .get(name)
            .copied()
            .ok_or_else(|| VoiceError::new(format!("Missing Flat ZIP entry {name:?}")))?;
        if length > limit as u64 {
            return Err(VoiceError::new(format!(
                "Flat ZIP entry {name:?} exceeds size limit"
            )));
        }
        let mut bytes = Vec::new();
        self.zip
            .by_name(name)
            .map_err(zip_error)?
            .take(length + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                VoiceError::new(format!("Reading Flat ZIP entry {name:?}: {error}"))
            })?;
        if bytes.len() as u64 != length {
            return Err(VoiceError::new(format!(
                "Flat ZIP entry {name:?} has an invalid length"
            )));
        }
        Ok(bytes)
    }

    pub fn read_batch_member(&mut self, name: &str) -> Result<Vec<u8>> {
        if self.kind != PackageKind::Sfpks || !is_sfpk(Path::new(name)) {
            return Err(VoiceError::new("Expected an SFPK member of a Flat batch"));
        }
        self.read(name, MAX_CHILD_BYTES)
    }

    pub fn describe(&mut self) -> Result<FlatPackage> {
        let manifest_name = match (self.contains("info.nofs"), self.contains("voice.nofs")) {
            (true, false) => "info.nofs",
            (false, true) => "voice.nofs",
            _ => {
                return Err(VoiceError::new(
                    "Expected exactly one Flat NOFS-JSON manifest",
                ));
            }
        };
        let manifest = FlatManifest::parse(&self.read(manifest_name, MAX_METADATA_BYTES)?)?;
        let config = self
            .contains("config.json")
            .then(|| {
                let bytes = self.read("config.json", MAX_METADATA_BYTES)?;
                let config: PackageConfig =
                    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
                        .map_err(|error| {
                        VoiceError::new(format!("Invalid Flat config.json: {error}"))
                    })?;
                if config.name != manifest.name
                    || config.version.parse::<u32>().ok() != Some(manifest.revision()?)
                {
                    return Err(VoiceError::new(
                        "Flat package and voice name or version disagree",
                    ));
                }
                Ok(config)
            })
            .transpose()?;
        Ok(FlatPackage {
            manifest,
            config,
            resources: self.files.keys().cloned().collect(),
        })
    }
}

impl FlatArchive {
    pub(super) fn visit_packages<T>(
        &mut self,
        mut visit: impl FnMut(Option<(&str, &[u8])>, FlatPackage) -> Result<T>,
    ) -> Result<Vec<T>> {
        if self.kind == PackageKind::Sfpk {
            return Ok(vec![visit(None, self.describe()?)?]);
        }
        let names = self
            .files
            .keys()
            .filter(|name| is_sfpk(Path::new(name)))
            .cloned()
            .collect::<Vec<_>>();
        names
            .into_iter()
            .map(|name| {
                let result = (|| {
                    let bytes = self.read_batch_member(&name)?;
                    let mut child = FlatArchive::read_zip(Cursor::new(bytes.as_slice()))?;
                    if child.kind != PackageKind::Sfpk {
                        return Err(VoiceError::new("Nested Flat batches are unsupported"));
                    }
                    visit(Some((&name, &bytes)), child.describe()?)
                })();
                result.map_err(|error| {
                    VoiceError::new(format!("Flat batch member {name:?}: {error}"))
                })
            })
            .collect()
    }
}

pub(super) fn is_sfpk(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sfpk"))
}

fn validate_path(name: &str) -> Result<()> {
    if name.len() > 1024
        || name.contains(['\\', ':', '\0'])
        || name.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(VoiceError::new(format!("Invalid Flat ZIP path {name:?}")));
    }
    Ok(())
}

fn zip_error(error: zip::result::ZipError) -> VoiceError {
    VoiceError::new(format!("Invalid Flat ZIP: {error}"))
}
