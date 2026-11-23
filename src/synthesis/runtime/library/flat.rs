use std::{
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    thread,
};

use super::{Runtime, VoiceLibrary, WakeOnExit};
use crate::synthesis::voice::flat;

pub(super) struct OperationResult {
    changed: bool,
    feedback: Result<String, String>,
}

impl VoiceLibrary {
    pub fn flat_busy(&self) -> bool {
        self.flat_operation.is_some()
    }

    pub fn flat_feedback(&self) -> Option<&Result<String, String>> {
        self.flat_feedback.as_ref()
    }

    pub(super) fn poll_flat_operation(&mut self) -> bool {
        let Some(operation) = self.flat_operation.as_ref() else {
            return false;
        };
        let result = match operation.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => OperationResult {
                changed: false,
                feedback: Err("Flat 声库操作线程意外终止。".into()),
            },
            Err(TryRecvError::Empty) => return false,
        };
        self.flat_operation = None;
        self.flat_feedback = Some(result.feedback);
        result.changed
    }
}

impl Runtime {
    pub fn set_flat_databases(&mut self, path: PathBuf) {
        if self.library.settings.flat_databases.as_ref() == Some(&path) {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.flat_databases = Some(path);
        if !self.library.flat_busy() && self.library.update(settings) {
            self.rescan_voices();
        }
    }

    pub fn set_flat_models(&mut self, path: Option<PathBuf>) {
        if self.library.settings.flat_models == path {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.flat_models = path;
        if !self.library.flat_busy() && self.library.update(settings) {
            self.rescan_voices();
        }
    }

    pub fn import_flat_voices(&mut self, paths: Vec<PathBuf>) {
        self.start_flat_operation(move |root| {
            let mut count = 0;
            let mut failures = Vec::new();
            for path in paths {
                match flat::import(&path, &root) {
                    Ok(voices) => count += voices.len(),
                    Err(error) => failures.push(format!("{}：{error}", path.display())),
                }
            }
            let message = format!("已导入 {count} 个 Flat 声库。");
            let feedback = if failures.is_empty() {
                Ok(message)
            } else {
                Err(format!("{message}\n{}", failures.join("\n")))
            };
            OperationResult {
                changed: count != 0,
                feedback,
            }
        });
    }

    pub fn remove_flat_voice(&mut self, path: PathBuf) {
        self.start_flat_operation(move |root| match flat::remove_import(&root, &path) {
            Ok(()) => OperationResult {
                changed: true,
                feedback: Ok("已移除导入的 Flat 声库。".into()),
            },
            Err(error) => OperationResult {
                changed: false,
                feedback: Err(error.to_string()),
            },
        });
    }

    fn start_flat_operation(
        &mut self,
        task: impl FnOnce(PathBuf) -> OperationResult + Send + 'static,
    ) {
        if self.library.flat_busy() || self.library.invalid_settings {
            return;
        }
        let root = match self.library.settings.flat_voice_directory() {
            Ok(root) => root,
            Err(error) => {
                self.library.flat_feedback = Some(Err(error));
                return;
            }
        };
        let repaint = self.repaint.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.library.flat_feedback = None;
        match thread::Builder::new()
            .name("flat-library-operation".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let _ = sender.send(task(root));
            }) {
            Ok(_) => self.library.flat_operation = Some(receiver),
            Err(error) => {
                self.library.flat_feedback = Some(Err(format!("无法启动 Flat 声库任务：{error}")))
            }
        }
    }
}
