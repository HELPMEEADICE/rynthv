use super::App;
use crate::{model::Project, storage, synthesis::notify::Notify};
use eframe::egui::Context;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    time::Duration,
};

const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct Snapshot {
    project: Project,
    path: Option<PathBuf>,
    revision: Option<storage::Revision>,
    dirty: bool,
    position: i64,
}

#[derive(PartialEq, Eq)]
struct Stamp {
    edit: u64,
    path: Option<PathBuf>,
    dirty: bool,
}

#[derive(Default)]
pub(super) struct Recovery {
    worker: Option<(SyncSender<Option<Snapshot>>, Receiver<String>)>,
    written: Option<Stamp>,
}

impl Recovery {
    pub(super) fn init(ctx: &Context) -> Result<(Self, Option<Snapshot>), String> {
        let directory = crate::platform::config_directory()?;
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let path = directory.join("recovery.json");
        let snapshot = match File::open(&path) {
            Ok(file) => {
                let size = file.metadata().map_err(|error| error.to_string())?.len();
                if size > MAX_SNAPSHOT_BYTES {
                    return Err("工程恢复文件超过大小上限".into());
                }
                Some(
                    serde_json::from_reader(file.take(MAX_SNAPSHOT_BYTES))
                        .map_err(|error| format!("无法读取工程恢复文件：{error}"))?,
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        let (sender, queue) = mpsc::sync_channel::<Option<Snapshot>>(1);
        let (errors, receiver) = mpsc::channel();
        let notify = Notify::new({
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        });
        std::thread::Builder::new()
            .name("project-recovery".into())
            .spawn(move || {
                for snapshot in queue {
                    let result = if let Some(snapshot) = snapshot {
                        storage::atomic_write_with(&path, |writer| {
                            serde_json::to_writer(writer, &snapshot)
                                .map_err(|error| error.to_string())
                        })
                    } else {
                        std::fs::remove_file(&path)
                            .or_else(|error| {
                                if error.kind() == std::io::ErrorKind::NotFound {
                                    Ok(())
                                } else {
                                    Err(error)
                                }
                            })
                            .map_err(|error| error.to_string())
                    };
                    if let Err(error) = result {
                        let _ = errors.send(error);
                    }
                    notify.wake();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok((
            Self {
                worker: Some((sender, receiver)),
                written: None,
            },
            snapshot,
        ))
    }

    pub(super) fn invalidate(&mut self) {
        self.written = None;
    }
}

impl App {
    pub(super) fn restore_snapshot(&mut self, snapshot: Snapshot) -> Result<(), String> {
        storage::validate(&snapshot.project)?;
        self.install_project(snapshot.project);
        self.path = snapshot.path;
        self.project_revision = snapshot.revision;
        self.recovered_dirty = snapshot.dirty;
        self.editor.transport.position = snapshot.position.clamp(0, self.editor.project_end());
        self.status = "已恢复上次工作".into();
        Ok(())
    }

    pub(super) fn update_recovery(&mut self, ctx: &Context) {
        let dirty = self.has_unsaved_changes();
        let Some((sender, errors)) = &self.recovery.worker else {
            return;
        };
        if let Ok(error) = errors.try_recv() {
            self.status = format!("工程自动保存失败：{error}");
        }
        if self.editor.is_editing() {
            return;
        }
        let stamp = Stamp {
            edit: self.editor.revision,
            path: self.path.clone(),
            dirty,
        };
        if self.recovery.written.as_ref() == Some(&stamp) {
            return;
        }
        let snapshot = (dirty || self.path.is_some()).then(|| Snapshot {
            project: self.editor.project.clone(),
            path: self.path.clone(),
            revision: self.project_revision,
            dirty,
            position: self.editor.transport.position,
        });
        match sender.try_send(snapshot) {
            Ok(()) => self.recovery.written = Some(stamp),
            Err(TrySendError::Full(_)) => ctx.request_repaint_after(Duration::from_millis(250)),
            Err(TrySendError::Disconnected(_)) => self.recovery.worker = None,
        }
    }
}
