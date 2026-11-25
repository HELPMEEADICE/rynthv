//! Lightweight discovery of installed voice revisions, without loading model weights.

use crate::platform::fs;
mod dnni;
mod flat;
mod metadata;
mod nofs;
mod resource;

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Result, VoiceFamily};
pub use flat::FlatCatalogEntry;
use metadata::MetadataIndex;

#[derive(Clone, Debug, Serialize)]
pub enum VoiceAvailability {
    /// The revision uses a supported format; full model validation happens on load.
    Loadable,
    Unsupported(String),
}

impl VoiceAvailability {
    pub fn is_loadable(&self) -> bool {
        matches!(self, Self::Loadable)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VoiceCatalogEntry {
    pub database_id: String,
    pub revision: u32,
    pub model_version: Option<u8>,
    pub family: Option<VoiceFamily>,
    pub directory: PathBuf,
    pub display_name: String,
    pub vendor: String,
    pub genres: Vec<String>,
    pub pitch_range: Option<String>,
    pub languages: Vec<String>,
    pub icon_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flat: Option<FlatCatalogEntry>,
    pub availability: VoiceAvailability,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct VoiceCatalog {
    pub entries: Vec<VoiceCatalogEntry>,
    pub warnings: Vec<String>,
}

impl VoiceCatalog {
    pub fn from_flat(registry: &super::flat::FlatModelRegistry) -> Self {
        flat::catalog(registry)
    }

    /// Discover supported DNNI revisions and original SV1 AI NOFS files without
    /// reading weights. A root can also be one SV1 voice folder or NOFS file.
    pub fn scan(root: &Path) -> Result<Self> {
        let mut catalog = Self::default();
        if super::nofs::is_file(root) {
            catalog.entries.push(nofs::entry(root)?);
            return Ok(catalog);
        }
        for path in super::nofs::paths(root)? {
            catalog.entries.push(nofs::entry(&path)?);
        }
        let metadata = MetadataIndex::load(root);
        for entry in fs::read_dir(root)? {
            let database = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    catalog
                        .warnings
                        .push(format!("无法读取声库目录 {}：{error}", root.display()));
                    continue;
                }
            };
            let is_dir = match database.file_type() {
                Ok(kind) => kind.is_dir(),
                Err(error) => {
                    catalog.warnings.push(format!(
                        "无法读取声库目录 {}：{error}",
                        database.path().display()
                    ));
                    continue;
                }
            };
            if !is_dir {
                continue;
            }
            if let Ok(database_id) = uuid::Uuid::parse_str(&database.file_name().to_string_lossy())
            {
                dnni::scan(&database.path(), database_id, &mut catalog, &metadata)?;
            }
            match super::nofs::paths(&database.path()) {
                Ok(paths) => {
                    for path in paths {
                        catalog.entries.push(nofs::entry(&path)?);
                    }
                }
                Err(error) => catalog.warnings.push(format!(
                    "无法读取声库目录 {}：{error}",
                    database.path().display()
                )),
            }
        }
        let mut counts = std::collections::BTreeMap::new();
        for entry in &catalog.entries {
            *counts
                .entry((entry.database_id.clone(), entry.revision))
                .or_insert(0usize) += 1;
        }
        for entry in &mut catalog.entries {
            if counts[&(entry.database_id.clone(), entry.revision)] > 1 {
                entry.availability = VoiceAvailability::Unsupported(
                    "相同声库版本存在多个文件，请移除重复项后刷新".to_owned(),
                );
            }
        }
        catalog.entries.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.database_id.cmp(&right.database_id))
                .then_with(|| right.revision.cmp(&left.revision))
        });
        Ok(catalog)
    }
}
