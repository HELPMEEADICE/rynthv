mod flat;

use super::{Runtime, events::WakeOnExit};
use crate::synthesis::{
    library::{LibraryScan, scan_roots},
    settings::{SynthesisSettings, VoiceSelection},
    voice::{VoiceCatalog, VoiceCatalogEntry, flat::FlatModelRegistry},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

pub struct VoiceLibrary {
    pub settings: SynthesisSettings,
    catalog: Option<VoiceCatalog>,
    scan: Option<Receiver<LibraryScan>>,
    catalog_error: Option<String>,
    settings_error: Option<String>,
    invalid_settings: bool,
    pub(super) flat_registry: Arc<FlatModelRegistry>,
    flat_operation: Option<Receiver<flat::OperationResult>>,
    flat_feedback: Option<Result<String, String>>,
}

impl VoiceLibrary {
    pub(super) fn new(repaint: crate::synthesis::notify::Notify) -> Self {
        let (settings, settings_error) = match SynthesisSettings::load() {
            Ok(settings) => (settings, None),
            Err(error) => (SynthesisSettings::default(), Some(error)),
        };
        let invalid_settings = settings_error.is_some();
        let mut library = Self {
            settings,
            catalog: None,
            scan: None,
            catalog_error: None,
            settings_error,
            invalid_settings,
            flat_registry: Arc::default(),
            flat_operation: None,
            flat_feedback: None,
        };
        if !invalid_settings {
            library.rescan(repaint);
        }
        library
    }

    pub fn catalog(&self) -> Option<&VoiceCatalog> {
        self.catalog.as_ref()
    }

    pub fn is_scanning(&self) -> bool {
        self.scan.is_some()
    }

    pub fn catalog_error(&self) -> Option<&str> {
        self.catalog_error.as_deref()
    }

    pub fn settings_error(&self) -> Option<&str> {
        self.settings_error.as_deref()
    }

    pub(super) fn update(&mut self, settings: SynthesisSettings) -> bool {
        if self.invalid_settings {
            return false;
        }
        match settings.save() {
            Ok(()) => {
                self.settings = settings;
                self.settings_error = None;
                true
            }
            Err(error) => {
                self.settings_error = Some(error);
                false
            }
        }
    }

    pub fn entry(&self, selection: &VoiceSelection) -> Option<&VoiceCatalogEntry> {
        self.catalog.as_ref()?.entry(selection)
    }
    fn rescan(&mut self, repaint: crate::synthesis::notify::Notify) {
        // Dropping the receiver prevents an earlier directory scan from
        // replacing the catalog after the user changes paths again.
        self.scan = None;
        self.catalog = None;
        self.flat_registry = Arc::default();
        self.catalog_error = None;
        if self.invalid_settings {
            return;
        }
        if !self.settings.has_databases() {
            return;
        }
        let settings = self.settings.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("voice-library-scan".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let sender = tx;
                let result = scan_roots(&settings);
                let _ = sender.send(result);
            }) {
            Ok(_) => self.scan = Some(rx),
            Err(error) => self.catalog_error = Some(format!("无法启动声库扫描：{error}")),
        }
    }

    fn poll(&mut self) -> bool {
        let Some(scan) = &self.scan else {
            return false;
        };
        match scan.try_recv() {
            Ok(result) => {
                self.scan = None;
                self.catalog = Some(result.catalog);
                self.catalog_error = result.error;
                self.flat_registry = result.flat_registry;
                true
            }
            Err(TryRecvError::Disconnected) => {
                self.scan = None;
                self.catalog_error = Some("声库扫描线程意外终止。".into());
                true
            }
            Err(TryRecvError::Empty) => false,
        }
    }
}

impl Runtime {
    pub fn dictionaries(&self) -> Option<&Path> {
        self.library.settings.dictionaries.as_deref()
    }
    pub fn set_databases(&mut self, path: PathBuf) {
        if self.library.settings.databases.as_ref() == Some(&path) {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.databases = Some(path);
        if self.library.update(settings) {
            self.rescan_voices();
        }
    }
    pub fn set_sv1_databases(&mut self, path: PathBuf) {
        if self.library.settings.sv1_databases.as_ref() == Some(&path) {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.sv1_databases = Some(path);
        if self.library.update(settings) {
            self.rescan_voices();
        }
    }
    pub fn set_dictionaries(&mut self, path: PathBuf) {
        if self.library.settings.dictionaries.as_ref() == Some(&path) {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.dictionaries = Some(path);
        if self.library.update(settings) {
            self.clear_voices();
        }
    }
    pub fn rescan_voices(&mut self) {
        self.clear_voices();
        self.library.rescan(self.repaint.clone());
    }
    pub(super) fn poll_library(&mut self) {
        if self.library.poll_flat_operation() {
            self.rescan_voices();
        }
        if self.library.poll() {
            self.last_revision = None;
        }
    }
}
