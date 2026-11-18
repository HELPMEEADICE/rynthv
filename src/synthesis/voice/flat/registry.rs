use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{FlatArchive, FlatPackage, ModelRole, archive::is_sfpk, manifest::validate_id};
use crate::synthesis::voice::Result;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelSource {
    File { path: PathBuf },
    Package { path: PathBuf, member: String },
}

#[derive(Default)]
pub struct FlatModelRegistry {
    entries: BTreeMap<(Option<ModelRole>, String), BTreeSet<ModelSource>>,
    pub packages: BTreeMap<PathBuf, FlatPackage>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct RegisteredModel<'a> {
    pub role: Option<ModelRole>,
    pub id: &'a str,
    pub sources: &'a BTreeSet<ModelSource>,
}

impl FlatModelRegistry {
    /// Index references and ZIP entry names without reading model weights.
    pub fn scan(voice_directory: Option<&Path>, model_directory: Option<&Path>) -> Self {
        let mut registry = Self::default();
        if let Some(path) = voice_directory {
            registry.scan_voices(path);
        }
        if let Some(path) = model_directory {
            registry.scan_model_folder(path, None, false);
            for role in [
                ModelRole::Acoustic,
                ModelRole::Vocoder,
                ModelRole::Duration,
                ModelRole::F0,
                ModelRole::SingingAssistant,
            ] {
                registry.scan_model_folder(&path.join(role.directory()), Some(role), true);
            }
        }
        registry
    }

    pub fn models(&self) -> impl Iterator<Item = RegisteredModel<'_>> {
        self.entries.iter().map(|((role, id), sources)| RegisteredModel {
            role: *role,
            id,
            sources,
        })
    }

    pub(super) fn candidates(&self, role: ModelRole, id: &str) -> BTreeSet<ModelSource> {
        [Some(role), None]
            .into_iter()
            .filter_map(|key| self.entries.get(&(key, id.to_owned())))
            .flatten()
            .cloned()
            .collect()
    }

    fn scan_voices(&mut self, directory: &Path) {
        self.scan_package_folder(directory, false);
    }

    fn scan_package_folder(&mut self, directory: &Path, optional: bool) {
        for path in self.files(directory, optional).into_iter().filter(|path| is_sfpk(path)) {
            let result = (|| -> Result<_> {
                let package = FlatArchive::open(&path)?.describe()?;
                for (role, id) in package.manifest.model_ids()? {
                    let member = model_member(id);
                    if package.resources.binary_search(&member).is_ok() {
                        self.entries
                            .entry((Some(role), id.to_owned()))
                            .or_default()
                            .insert(ModelSource::Package { path: path.clone(), member });
                    }
                }
                Ok(package)
            })();
            match result {
                Ok(package) => { self.packages.insert(path, package); }
                Err(error) => self.warnings.push(format!("{}: {error}", path.display())),
            }
        }
    }

    fn scan_model_folder(&mut self, directory: &Path, role: Option<ModelRole>, optional: bool) {
        for path in self.files(directory, optional) {
            if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dnni")) {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|id| id.to_str()) else {
                continue;
            };
            if let Err(error) = validate_id(id) {
                self.warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
            self.entries.entry((role, id.to_owned())).or_default()
                .insert(ModelSource::File { path });
        }
    }

    fn files(&mut self, directory: &Path, optional: bool) -> Vec<PathBuf> {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                if !(optional && error.kind() == std::io::ErrorKind::NotFound) {
                    self.warnings.push(format!("{}: {error}", directory.display()));
                }
                return Vec::new();
            }
        };
        entries.filter_map(|entry| {
            let result = entry.and_then(|entry| Ok((entry.file_type()?, entry.path())));
            match result {
                Ok((kind, path)) if kind.is_file() => Some(path),
                Ok(_) => None,
                Err(error) => {
                    self.warnings.push(format!("{}: {error}", directory.display()));
                    None
                }
            }
        }).collect()
    }
}

pub(super) fn model_member(id: &str) -> String {
    format!("models/{id}.dnni")
}
