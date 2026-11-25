use super::{ContextImportPhase, Runtime, VoiceState};
use crate::{
    model::{GroupRef, Track},
    platform::web::{
        client::{Pending, submit},
        protocol::{Library, Request},
    },
    synthesis::{
        inference::compute::{AdapterId, AdapterInfo, Backend},
        notify::Notify,
        settings::{SynthesisSettings, VoiceSelection},
        voice::{VoiceCatalog, VoiceCatalogEntry},
    },
};
use std::path::{Path, PathBuf};

pub struct VoiceLibrary {
    pub settings: SynthesisSettings,
    catalog: Option<VoiceCatalog>,
    adapters: Option<Vec<AdapterInfo>>,
    error: Option<String>,
    feedback: Option<Result<String, String>>,
    pub(super) task: Option<Pending<Library>>,
}

impl VoiceLibrary {
    pub(super) fn new(repaint: Notify) -> Self {
        Self {
            settings: SynthesisSettings::default(),
            catalog: None,
            adapters: None,
            error: None,
            feedback: None,
            task: Some(submit(Request::Scan, repaint, |_, _| {})),
        }
    }

    pub fn catalog(&self) -> Option<&VoiceCatalog> {
        self.catalog.as_ref()
    }
    pub fn entry(&self, selection: &VoiceSelection) -> Option<&VoiceCatalogEntry> {
        self.catalog()?.entry(selection)
    }
    pub fn is_scanning(&self) -> bool {
        self.task.is_some()
    }
    pub fn catalog_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn settings_error(&self) -> Option<&str> {
        self.catalog
            .is_none()
            .then_some(self.error.as_deref())
            .flatten()
    }
    pub fn resource_busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn resource_feedback(&self) -> Option<&Result<String, String>> {
        self.feedback.as_ref()
    }
    pub fn has_databases(&self) -> bool {
        self.catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.entries.is_empty())
            || self.settings.database_roots().next().is_some()
            || self.settings.flat_databases.is_some()
    }
}

impl Runtime {
    pub fn resolve_voice(
        &self,
        track: &Track,
        reference: &GroupRef,
    ) -> Result<Option<VoiceSelection>, String> {
        if track.is_instrumental() {
            return Ok(None);
        }
        self.library.catalog().map_or(Ok(None), |catalog| {
            catalog.resolve(&track.main_ref.database, &reference.database)
        })
    }

    pub fn dictionaries(&self) -> Option<&Path> {
        self.library.settings.dictionaries.as_deref()
    }

    pub fn rescan_voices(&mut self) {
        self.library_request(Request::Scan);
    }
    pub fn import_voices(&mut self, paths: Vec<PathBuf>) {
        self.library_request(Request::ImportVoices { paths });
    }
    pub fn import_dictionaries(&mut self, path: PathBuf) {
        self.library_request(Request::ImportDictionaries { path });
    }

    pub fn remove_flat_voice(&mut self, path: PathBuf) {
        if let Some(selection) = self
            .library
            .catalog()
            .and_then(|catalog| catalog.entries.iter().find(|entry| entry.directory == path))
            .map(VoiceCatalogEntry::selection)
        {
            self.library_request(Request::RemoveVoice { selection });
        }
    }

    fn update_settings(&mut self, update: impl FnOnce(&mut SynthesisSettings)) {
        let mut settings = self.library.settings.clone();
        update(&mut settings);
        if settings != self.library.settings {
            self.library_request(Request::Settings { settings });
        }
    }

    pub fn set_databases(&mut self, path: PathBuf) {
        self.update_settings(|settings| settings.databases = Some(path));
    }
    pub fn set_sv1_databases(&mut self, path: PathBuf) {
        self.update_settings(|settings| settings.sv1_databases = Some(path));
    }
    pub fn set_flat_databases(&mut self, path: PathBuf) {
        self.update_settings(|settings| settings.flat_databases = Some(path));
    }
    pub fn set_flat_models(&mut self, path: Option<PathBuf>) {
        self.update_settings(|settings| settings.flat_models = path);
    }
    pub fn set_dictionaries(&mut self, path: PathBuf) {
        self.update_settings(|settings| settings.dictionaries = Some(path));
    }
    pub fn set_compute_settings(&mut self, backend: Backend, adapter: Option<AdapterId>) {
        self.update_settings(|settings| {
            settings.compute_backend = backend;
            settings.compute_adapter = adapter;
        });
    }

    pub fn compute_diagnostics(&self) -> Option<&super::ComputeDiagnostics> {
        self.diagnostics.as_ref()
    }
    pub fn compute_adapters(&self) -> Option<&[AdapterInfo]> {
        self.library.adapters.as_deref()
    }

    pub(super) fn library_request(&mut self, request: Request) -> bool {
        if self.library.task.is_some() {
            return false;
        }
        self.invalidate_preview();
        self.library.feedback = None;
        self.library.task = Some(submit(request, self.repaint.clone(), |_, _| {}));
        true
    }

    pub(super) fn poll_library(&mut self) {
        let Some(result) = self.library.task.as_ref().and_then(Pending::poll) else {
            return;
        };
        self.library.task = None;
        let importing = self
            .context_import
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.phase == ContextImportPhase::Importing);
        match result {
            Ok(result) => {
                self.library.settings = result.settings;
                self.library.catalog = Some(result.catalog);
                self.library.adapters = Some(result.adapters);
                self.diagnostics = Some(result.diagnostics);
                self.library.error = result.warning;
                self.library.feedback = result.message.clone().map(Ok);
                if importing {
                    self.context_import.dialog = None;
                    self.context_import.feedback = result.message.map(Ok);
                }
                self.clear_inventories();
                self.preview.resync();
            }
            Err(error) => {
                if importing && let Some(dialog) = &mut self.context_import.dialog {
                    dialog.error = Some(error.clone());
                    dialog.phase = ContextImportPhase::Ready;
                }
                self.library.feedback = Some(Err(error.clone()));
                if self.library.catalog.is_none() {
                    self.library.error = Some(error);
                }
                self.preview.resync();
            }
        }
    }

    pub(super) fn inventory_request(&mut self, selection: VoiceSelection) {
        if self.voices.contains_key(&selection) {
            return;
        }
        let task = submit(
            Request::Inventory {
                selection: selection.clone(),
            },
            self.repaint.clone(),
            |_, _| {},
        );
        self.voices.insert(selection.clone(), VoiceState::Loading);
        self.voice_jobs.insert(selection, task);
    }

    pub(super) fn poll_inventories(&mut self) {
        let mut changed = false;
        self.voice_jobs.retain(|selection, pending| {
            let Some(result) = pending.poll() else {
                return true;
            };
            self.voices.insert(
                selection.clone(),
                match result {
                    Ok(inventory) => VoiceState::Loaded(Box::new(inventory)),
                    Err(error) => VoiceState::Failed(error),
                },
            );
            changed = true;
            false
        });
        if changed {
            self.refresh_voice_failures();
        }
    }

    pub(super) fn clear_inventories(&mut self) {
        for task in std::mem::take(&mut self.voice_jobs).into_values() {
            task.cancel();
        }
        self.voices.clear();
    }

    pub fn reload_voice(&mut self, key: &VoiceSelection) {
        self.voices.remove(key);
        if let Some(pending) = self.voice_jobs.remove(key) {
            pending.cancel();
        }
        self.invalidate_preview();
        self.inventory_request(key.clone());
        self.preview.resync();
    }
}
