//! Browser file dialogs and IndexedDB workspace state for the shared editor.

mod projects;

use super::{App, FileAction, picker::Completion};
use crate::{
    platform::web::{documents, js_error},
    ui::{settings::Preferences, state::ViewState},
};
use eframe::egui::Context;
use serde::Deserialize;
use std::{collections::VecDeque, future::Future, path::PathBuf, sync::mpsc, time::Duration};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_time::Instant;

pub(crate) use projects::download;
use projects::{ProjectEntry, Save, Snapshot};

#[wasm_bindgen(raw_module = "./browser.js")]
extern "C" {
    pub fn set_title(title: &str);
    fn storage_usage() -> js_sys::Promise;
}

pub(crate) struct Startup {
    pub(super) view: Option<ViewState>,
    pub(super) preferences: Option<Preferences>,
    pub(super) recovery: Option<Snapshot>,
    pub(super) font: Result<Vec<u8>, String>,
}

impl Startup {
    pub(crate) async fn load() -> Self {
        Self {
            view: read("workspace").await,
            preferences: read("preferences").await,
            recovery: read("recovery").await,
            font: documents::font().await,
        }
    }
}

async fn read<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
    let value = documents::record(key).await.ok().flatten()?;
    serde_json::from_str(&value).ok()
}

type Task<T> = mpsc::Receiver<Result<T, String>>;

#[derive(Default)]
pub(super) struct State {
    pub(super) picker: Option<(Completion, Task<Vec<PathBuf>>)>,
    pub(super) dropped_project: Option<PathBuf>,
    dropped: VecDeque<PathBuf>,
    pub(super) recovered_dirty: bool,
    loading: Option<Task<Snapshot>>,
    saving: Option<Task<Save>>,
    listing: Option<Task<Vec<ProjectEntry>>>,
    local: Option<Vec<ProjectEntry>>,
    persistence: Option<Task<()>>,
    last_recovery: Option<(String, u64, bool)>,
    ctx: Option<Context>,
    storage: Option<Task<Usage>>,
    storage_report: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    usage: f64,
    quota: f64,
}

pub(super) fn task<T: 'static>(
    ctx: &Context,
    work: impl Future<Output = Result<T, String>> + 'static,
) -> Task<T> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    spawn_local(async move {
        let _ = tx.send(work.await);
        ctx.request_repaint();
    });
    rx
}

fn poll<T>(task: &Task<T>) -> Option<Result<T, String>> {
    match task.try_recv() {
        Ok(value) => Some(value),
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(Err("浏览器文件任务已中断".into())),
    }
}

impl App {
    pub(super) fn poll_browser(&mut self, ctx: &Context) {
        ctx.request_repaint_after(Duration::from_millis(120));
        if let Some(error) = ctx.data_mut(|data| {
            data.remove_temp::<String>(eframe::egui::Id::new("web-download-error"))
        }) {
            self.error = Some(error);
        }
        if let Some(result) = self
            .browser
            .picker
            .as_ref()
            .and_then(|(_, task)| poll(task))
        {
            let (complete, _) = self.browser.picker.take().expect("completed picker");
            match result {
                Ok(paths) if !paths.is_empty() => complete(self, ctx, paths),
                Ok(_) => {}
                Err(error) => self.error = Some(error),
            }
        }
        self.poll_web_projects(ctx);
        if let Some(result) = self.browser.storage.as_ref().and_then(poll) {
            self.browser.storage = None;
            match result {
                Ok(usage) => self.browser.storage_report = Some(usage),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.browser.persistence.as_ref().and_then(poll) {
            self.browser.persistence = None;
            if let Err(error) = result {
                self.browser.last_recovery = None;
                self.last_layout.clear();
                self.last_preferences.clear();
                self.status = format!("无法自动保存：{error}");
            }
        }
        self.poll_drop(ctx);
    }

    pub(crate) fn browser_storage_panel(&mut self, ui: &mut eframe::egui::Ui) {
        if let Some(usage) = &self.browser.storage_report {
            ui.weak(format!(
                "当前源约占用 {:.1} MiB／配额 {:.1} MiB",
                usage.usage / 1048576.0,
                usage.quota / 1048576.0
            ));
        }
        ui.horizontal_wrapped(|ui| {
            let idle = self.browser.storage.is_none();
            if ui
                .add_enabled(idle, eframe::egui::Button::new("估算存储用量"))
                .clicked()
            {
                self.browser.storage = Some(task(ui.ctx(), documents::json(storage_usage())));
            }
            if ui
                .add_enabled(
                    idle && !self.synthesis.is_exporting(),
                    eframe::egui::Button::new("清理 WAV 导出缓存"),
                )
                .clicked()
            {
                self.synthesis.export_result = None;
                let clear = documents::clear_exports();
                self.browser.storage = Some(task(ui.ctx(), async move {
                    JsFuture::from(clear).await.map_err(js_error)?;
                    documents::json(storage_usage()).await
                }));
            }
        });
    }

    fn poll_drop(&mut self, ctx: &Context) {
        #[derive(Deserialize)]
        struct Drop {
            paths: Option<Vec<PathBuf>>,
            error: Option<String>,
        }
        if let Some(result) =
            documents::take_drop().and_then(|value| serde_json::from_str::<Drop>(&value).ok())
        {
            if let Some(error) = result.error {
                self.error = Some(error);
            }
            if let Some(paths) = result.paths {
                let includes_project = paths.iter().any(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("svp"))
                });
                self.browser
                    .dropped
                    .extend(paths.into_iter().filter(|path| {
                        !includes_project
                            || path
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("svp"))
                    }));
            }
        }
        if self.pending.is_some()
            || self.error.is_some()
            || self.synthesis.library.resource_busy()
            || self.browser.loading.is_some()
            || self.audio_import.is_some()
            || self.midi_import.is_some()
        {
            return;
        }
        let Some(path) = self.browser.dropped.pop_front() else {
            return;
        };
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "svp" => {
                self.browser.dropped_project = Some(path);
                self.request_file(FileAction::OpenDropped, ctx);
            }
            "mid" | "midi" => self.load_midi_import(path, super::MidiImportMode::Tracks, ctx),
            "svpk" | "sfpk" | "sfpks" => self.synthesis.import_voices(vec![path]),
            "zip" | "7z" => self.synthesis.import_dictionaries(path),
            "wav" | "mp3" | "flac" | "ogg" | "aif" | "aiff" | "aac" | "m4a" | "mp4" => {
                self.editor.finish();
                let track = self.editor.add_instrumental_track();
                self.begin_audio_import(track, path, false, ctx);
            }
            _ => self.error = Some("不支持拖入此文件类型".into()),
        }
    }

    pub(super) fn persist_layout(&mut self) {
        self.last_persist = Instant::now();
        if self.browser.persistence.is_some() || self.browser.saving.is_some() {
            return;
        }
        let mut records = Vec::new();
        if let Ok(view) = serde_json::to_string_pretty(&self.view)
            && view != self.last_layout
        {
            self.last_layout = view.clone();
            records.push(("workspace", view));
        }
        if let Ok(preferences) = serde_json::to_string_pretty(&self.preferences)
            && preferences != self.last_preferences
        {
            self.last_preferences = preferences.clone();
            records.push(("preferences", preferences));
        }
        let tag = (
            self.editor.project.uuid.clone(),
            self.editor.revision,
            self.has_unsaved_changes(),
        );
        if !self.editor.is_editing() && self.browser.last_recovery.as_ref() != Some(&tag) {
            match self
                .project_snapshot(tag.2)
                .and_then(|snapshot| serde_json::to_string(&snapshot).map_err(|e| e.to_string()))
            {
                Ok(snapshot) => {
                    self.browser.last_recovery = Some(tag);
                    records.push(("recovery", snapshot));
                }
                Err(error) => self.status = format!("无法自动保存：{error}"),
            }
        }
        if records.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.browser.persistence = Some(rx);
        // Start these transactions immediately so a later explicit save is ordered after them.
        let writes: Vec<_> = records
            .into_iter()
            .map(|(key, value)| documents::write_record(key, &value))
            .collect();
        spawn_local(async move {
            let mut error = None;
            for promise in writes {
                if let Err(cause) = JsFuture::from(promise).await {
                    error = Some(js_error(cause));
                }
            }
            let _ = tx.send(error.map_or(Ok(()), Err));
        });
    }
}
