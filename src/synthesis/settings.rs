//! Portable resource locations and the selected voice relative to its library.

use super::{
    inference::compute::{AdapterId, Backend},
    voice::{flat::FlatModelRegistry, svpk},
};
use crate::platform::fs;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use crate::{tr, trf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSelection {
    pub database_id: String,
    pub revision: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisSettings {
    pub databases: Option<PathBuf>,
    pub sv1_databases: Option<PathBuf>,
    pub flat_databases: Option<PathBuf>,
    pub flat_models: Option<PathBuf>,
    pub dictionaries: Option<PathBuf>,
    pub selected_voice: Option<VoiceSelection>,
    #[serde(default)]
    pub compute_backend: Backend,
    pub compute_adapter: Option<AdapterId>,
}

impl SynthesisSettings {
    pub fn database_roots(&self) -> impl Iterator<Item = (&'static str, &Path)> {
        [
            ("SV2", self.databases.as_deref()),
            ("SV1", self.sv1_databases.as_deref()),
        ]
        .into_iter()
        .filter_map(|(label, path)| path.map(|path| (label, path)))
    }

    pub fn has_databases(&self) -> bool {
        self.databases.is_some()
            || self.sv1_databases.is_some()
            || self.flat_databases.is_some()
            || self.flat_voice_directory().is_ok_and(fs::is_dir)
            || svpk::root().is_ok_and(fs::is_dir)
    }

    pub fn flat_voice_directory(&self) -> Result<PathBuf, String> {
        match &self.flat_databases {
            Some(path) => Ok(path.clone()),
            None => crate::platform::config_directory().map(|path| path.join("flat-voices")),
        }
    }

    pub fn flat_registry(&self) -> FlatModelRegistry {
        let voices = self
            .flat_voice_directory()
            .ok()
            .filter(|path| self.flat_databases.is_some() || fs::is_dir(path));
        FlatModelRegistry::scan(voices.as_deref(), self.flat_models.as_deref())
    }

    pub fn path() -> Result<PathBuf, String> {
        crate::platform::config_directory().map(|path| path.join("synthesis.json"))
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::path()?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| trf!("无法读取合成设置 {}：{}", path.display(), error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(trf!("无法读取合成设置 {}：{}", path.display(), error)),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path()?;
        // A malformed existing file needs explicit repair, not replacement by
        // defaults from a startup that could not deserialize it.
        Self::load()?;
        let parent = path.parent().expect("settings path has a parent");
        fs::create_dir_all(parent)
            .map_err(|error| trf!("无法创建合成设置目录 {}：{}", parent.display(), error))?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| trf!("无法序列化合成设置：{}", error))?;
        let temporary = parent.join(format!("synthesis-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|error| trf!("无法保存合成设置 {}：{}", path.display(), error))
    }

    pub fn selected_directory(&self) -> Result<PathBuf, String> {
        if !self.has_databases() {
            return Err(tr!("尚未设置声库目录。").to_owned());
        }
        let selected = self
            .selected_voice
            .as_ref()
            .ok_or_else(|| tr!("尚未选择声库。").to_owned())?;
        let scan = super::library::scan_roots(self);
        let mut directories = scan
            .catalog
            .entries
            .into_iter()
            .filter_map(|entry| (entry.selection() == *selected).then_some(entry.directory));
        match (directories.next(), directories.next()) {
            (Some(directory), None) => Ok(directory),
            (None, _) => {
                let mut message = trf!("声库 {}/{} 不在已配置的声库目录中。", selected.database_id, selected.revision);
                if let Some(error) = scan.error {
                    message.push('\n');
                    message.push_str(&error);
                }
                Err(message)
            }
            (Some(_), Some(_)) => Err(trf!("声库 {}/{} 对应多个文件，请检查声库目录配置。", selected.database_id, selected.revision)),
        }
    }
}
