use super::{documents, js_error, poll, task};
use crate::{app::App, storage, tr, ui};
use eframe::egui::{Context, Id, Modal, ScrollArea};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[derive(Serialize, Deserialize)]
pub(in crate::app) struct Snapshot {
    project: String,
    name: Option<PathBuf>,
    dirty: bool,
}

#[derive(Deserialize)]
pub(super) struct ProjectEntry {
    id: String,
    name: String,
}

pub(super) struct Save {
    project: String,
    revision: u64,
    name: PathBuf,
}

impl App {
    pub fn open_path(&mut self, path: PathBuf) {
        let ctx = self.browser_context();
        self.browser.loading = Some(task(&ctx, async move {
            let bytes = documents::read(&path).await?;
            let mut loaded = storage::decode_project(&bytes)?;
            if let Some(parent) = path.parent() {
                storage::resolve_project_paths(&mut loaded.project, parent);
            }
            let mut project = Vec::new();
            storage::write_project(&mut project, &loaded.project)?;
            let _ = JsFuture::from(documents::delete_document(&path.to_string_lossy())).await;
            Ok(Snapshot {
                project: String::from_utf8(project).map_err(|error| error.to_string())?,
                name: path.file_name().map(PathBuf::from),
                dirty: loaded.imported,
            })
        }));
    }

    pub(in crate::app) fn restore_web_project(&mut self, snapshot: Snapshot) {
        match storage::decode_project(snapshot.project.as_bytes()) {
            Ok(loaded) => {
                self.install_project(loaded.project);
                self.path = snapshot.name;
                self.browser.recovered_dirty = snapshot.dirty;
                self.status = tr!("已打开浏览器工程").into();
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn project_snapshot(&self, dirty: bool) -> Result<Snapshot, String> {
        let mut bytes = Vec::new();
        storage::write_project(&mut bytes, &self.editor.project)?;
        Ok(Snapshot {
            project: String::from_utf8(bytes).map_err(|e| e.to_string())?,
            name: self.path.clone(),
            dirty,
        })
    }

    pub fn save_project(&mut self, save_as: bool, ctx: &Context) -> bool {
        if self.browser.saving.is_some() {
            return false;
        }
        ui::phonemes::phoneme_deactivate(self);
        ui::piano::cancel_audition(self);
        ui::tempo_input::finish(self);
        self.commit_lyric();
        self.editor.finish();
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or(tr!("未命名.svp"))
            .to_owned();
        if save_as {
            let mut project = self.editor.project.clone();
            for clip in project
                .tracks
                .iter_mut()
                .filter_map(|track| track.instrumental.as_mut()?.clip.as_mut())
            {
                if clip.path.starts_with("/RynthV/imports")
                    && let Some(name) = clip.path.file_name()
                {
                    clip.path = PathBuf::from(name);
                }
            }
            project.render_config.remove("destination");
            let mut bytes = Vec::new();
            match storage::write_project(&mut bytes, &project) {
                Ok(()) => documents::download_bytes(&bytes, &name, "application/json"),
                Err(error) => self.error = Some(error),
            }
            return self.error.is_none();
        }
        let result = self.project_snapshot(false).and_then(|mut snapshot| {
            snapshot.name = Some(PathBuf::from(&name));
            serde_json::to_string(&snapshot).map_err(|error| error.to_string())
        });
        match result {
            Ok(snapshot) => {
                let project = self.editor.project.uuid.clone();
                let revision = self.editor.revision;
                let operation = documents::save_project(&project, &name, &snapshot);
                self.browser.saving = Some(task(ctx, async move {
                    JsFuture::from(operation).await.map_err(js_error)?;
                    Ok(Save {
                        project,
                        revision,
                        name: PathBuf::from(name),
                    })
                }));
            }
            Err(error) => self.error = Some(error),
        }
        false
    }

    pub(in crate::app) fn list_local_projects(&mut self) {
        let ctx = self.browser_context();
        self.browser.listing = Some(task(&ctx, documents::json(documents::project_index())));
    }

    pub(in crate::app) fn local_project_dialog(&mut self, ctx: &Context) {
        let Some(projects) = self.browser.local.as_ref() else {
            return;
        };
        let mut selected = None;
        let dialog = Modal::new(Id::new("web-projects")).show(ctx, |ui| {
            ui.heading(tr!("浏览器工程"));
            if projects.is_empty() {
                ui.label(tr!("已保存的工程会显示在这里。"));
            }
            ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.55)
                .show(ui, |ui| {
                    for project in projects {
                        if ui.button(&project.name).clicked() {
                            selected = Some(project.id.clone());
                        }
                    }
                });
            if ui.button(tr!("关闭")).clicked() {
                ui.close();
            }
        });
        if let Some(id) = selected {
            self.browser.local = None;
            self.browser.loading = Some(task(ctx, async move {
                let value = documents::record(&format!("project:{id}"))
                    .await?
                    .ok_or(tr!("找不到已保存的工程"))?;
                serde_json::from_str(&value).map_err(|error| error.to_string())
            }));
        } else if dialog.should_close() {
            self.browser.local = None;
        }
    }

    pub(super) fn poll_web_projects(&mut self, ctx: &Context) {
        self.browser.ctx = Some(ctx.clone());
        if let Some(result) = self.browser.loading.as_ref().and_then(poll) {
            self.browser.loading = None;
            match result {
                Ok(snapshot) => self.restore_web_project(snapshot),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.browser.listing.as_ref().and_then(poll) {
            self.browser.listing = None;
            match result {
                Ok(projects) => self.browser.local = Some(projects),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.browser.saving.as_ref().and_then(poll) {
            self.browser.saving = None;
            match result {
                Ok(save) => {
                    if self.editor.project.uuid == save.project
                        && self.editor.revision == save.revision
                    {
                        self.path = Some(save.name);
                        self.editor.mark_saved();
                        self.browser.recovered_dirty = false;
                        self.browser.last_recovery = Some((save.project, save.revision, false));
                        self.status = tr!("工程已保存到浏览器").into();
                        if let Some(action) = self.pending.take() {
                            self.perform_file(action, ctx);
                        }
                    }
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn browser_context(&self) -> Context {
        self.browser.ctx.clone().unwrap_or_default()
    }
}

pub(crate) fn download(path: &Path, ctx: &Context) {
    let promise = documents::download_file(&path.to_string_lossy());
    let context = ctx.clone();
    spawn_local(async move {
        if let Err(error) = JsFuture::from(promise).await {
            context
                .data_mut(|data| data.insert_temp(Id::new("web-download-error"), js_error(error)));
            context.request_repaint();
        }
    });
}
