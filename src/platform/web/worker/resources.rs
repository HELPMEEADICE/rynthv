use super::{Worker, fs, json};
use crate::{
    platform::{
        config_directory,
        web::protocol::{Inventory, Library},
    },
    synthesis::{
        dictionary_import,
        settings::{SynthesisSettings, VoiceSelection},
        voice::{flat, session_identity, svpk},
    },
};
use std::path::{Path, PathBuf};
use web_time::Instant;
use zeroize::Zeroizing;

impl Worker {
    pub(super) fn library(&mut self, message: Option<String>) -> Result<String, String> {
        let settings = self.settings().clone();
        let scan = self.offline.library(&settings);
        json(&Library {
            settings,
            catalog: scan.catalog.clone(),
            warning: scan.error.clone(),
            message,
        })
    }

    pub(super) fn update_settings(
        &mut self,
        settings: SynthesisSettings,
    ) -> Result<String, String> {
        settings.save()?;
        if self.settings().compute_backend != settings.compute_backend
            || self.settings().compute_adapter != settings.compute_adapter
        {
            self.offline.reset_compute();
        }
        self.settings = Some(settings);
        self.offline.refresh();
        self.library(None)
    }

    pub(super) fn import_voices(&mut self, paths: Vec<PathBuf>) -> Result<String, String> {
        let root = self.settings().flat_voice_directory()?;
        let mut imported = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            let result = if svpk::is_package(&path) {
                svpk::import(&path, false).map(|voice| vec![voice.name])
            } else {
                flat::import(&path, &root)
                    .map(|voices| voices.into_iter().map(|voice| voice.name).collect())
            };
            self.remove_staged(&path);
            match result {
                Ok(voices) => imported.extend(voices),
                Err(error) => failures.push(format!(
                    "{}：{error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        if imported.is_empty() && !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        self.offline.refresh();
        let mut message = format!(
            "已导入 {} 个声库（{}）",
            imported.len(),
            imported.join("、")
        );
        for failure in failures {
            message.push('\n');
            message.push_str(&failure);
        }
        self.library(Some(message))
    }

    pub(super) fn import_dictionaries(&mut self, path: &Path) -> Result<String, String> {
        let result = dictionary_import::prepare(path, false);
        self.remove_staged(path);
        let directory = result.map_err(|error| error.to_string())?;
        let previous = self.settings().dictionaries.clone();
        let mut settings = self.settings().clone();
        settings.dictionaries = Some(directory.path().to_owned());
        settings.save()?;
        directory.keep();
        self.settings = Some(settings);
        self.offline.refresh();
        let managed = config_directory()?.join("clf-data");
        if let Some(previous) = previous.filter(|path| path.parent() == Some(managed.as_path())) {
            let _ = fs::remove_dir_all(previous);
        }
        self.library(Some("发音词典已导入".into()))
    }

    pub(super) fn remove_voice(&mut self, selection: &VoiceSelection) -> Result<String, String> {
        let settings = self.settings().clone();
        let path = self
            .offline
            .library(&settings)
            .catalog
            .entry(selection)
            .ok_or("所选声库不在浏览器存储中")?
            .directory
            .clone();
        self.offline.refresh();
        if selection.database_id.starts_with("sv1:") {
            svpk::remove_import(&path)
        } else {
            flat::remove_import(&settings.flat_voice_directory()?, &path)
        }
        .map_err(|error| error.to_string())?;
        self.library(Some("声库已删除".into()))
    }

    pub(super) fn inventory(&mut self, selection: &VoiceSelection) -> Result<String, String> {
        let start = Instant::now();
        let settings = self.settings().clone();
        let voice = self.offline.inventory(&settings, selection)?;
        let needs_context = voice.configuration.as_ref().is_some_and(|configuration| {
            configuration.family.requires_identity()
                && session_identity::saved_identity(
                    &configuration.name,
                    configuration.model_version,
                )
                .is_err()
        });
        json(&Inventory {
            configuration: voice.configuration.clone(),
            flat_warnings: voice.flat_warnings.clone(),
            content_hash: voice.content_hash.clone(),
            file_count: voice.files.len(),
            resource_count: voice.resource_count,
            graph_count: voice.graph_count,
            tensor_count: voice.tensor_count,
            needs_context,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    pub(super) fn import_context(
        &mut self,
        name: &str,
        version: u8,
        file: Option<&Path>,
        identity: Option<String>,
    ) -> Result<String, String> {
        let result = if let Some(path) = file {
            let result = session_identity::import_context_file(name, version, path);
            self.remove_staged(path);
            result
        } else {
            let identity = Zeroizing::new(identity.ok_or("请选择模型上下文文件或输入 identity")?);
            session_identity::parse_identity(&identity).and_then(|identity| {
                session_identity::save_manual_identity(name, version, identity)
            })
        };
        result.map_err(|error| error.to_string())?;
        self.offline.refresh();
        self.library(Some("模型上下文已保存".into()))
    }

    pub(super) fn remove_staged(&self, path: &Path) {
        if let Some(parent) = path
            .parent()
            .filter(|parent| parent.parent() == Some(Path::new("/RynthV/imports")))
        {
            let _ = fs::remove_file(path);
            let _ = fs::remove_dir(parent);
        }
    }
}
