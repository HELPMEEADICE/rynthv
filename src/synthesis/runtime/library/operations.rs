use std::{
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    thread,
};

use super::{Runtime, VoiceLibrary, WakeOnExit};
use crate::synthesis::{
    dictionary_import,
    voice::{flat, svpk},
};

pub(super) enum OperationResult {
    Voices {
        changed: bool,
        feedback: Result<String, String>,
    },
    Dictionaries(Result<tempfile::TempDir, String>),
}

impl VoiceLibrary {
    pub fn resource_busy(&self) -> bool {
        self.resource_operation.is_some()
    }

    pub fn resource_feedback(&self) -> Option<&Result<String, String>> {
        self.resource_feedback.as_ref()
    }
}

impl Runtime {
    pub fn set_flat_databases(&mut self, path: PathBuf) {
        if self.library.settings.flat_databases.as_ref() == Some(&path) {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.flat_databases = Some(path);
        if !self.library.resource_busy() && self.library.update(settings) {
            self.rescan_voices();
        }
    }

    pub fn set_flat_models(&mut self, path: Option<PathBuf>) {
        if self.library.settings.flat_models == path {
            return;
        }
        let mut settings = self.library.settings.clone();
        settings.flat_models = path;
        if !self.library.resource_busy() && self.library.update(settings) {
            self.rescan_voices();
        }
    }

    pub fn import_voices(&mut self, paths: Vec<PathBuf>) {
        let flat_root = self.library.settings.flat_voice_directory();
        self.start_resource_operation(move || {
            let mut count = 0;
            let mut failures = Vec::new();
            for path in paths {
                let result = if svpk::is_package(&path) {
                    svpk::import(&path, false)
                        .map(|_| 1)
                        .map_err(|error| error.to_string())
                } else {
                    flat_root.as_ref().map_err(Clone::clone).and_then(|root| {
                        flat::import(&path, root)
                            .map(|voices| voices.len())
                            .map_err(|error| error.to_string())
                    })
                };
                match result {
                    Ok(imported) => count += imported,
                    Err(error) => failures.push(format!("{}：{error}", path.display())),
                }
            }
            let message = format!("已导入 {count} 个声库。");
            let feedback = if failures.is_empty() {
                Ok(message)
            } else {
                Err(format!("{message}\n{}", failures.join("\n")))
            };
            OperationResult::Voices {
                changed: count != 0,
                feedback,
            }
        });
    }

    pub fn import_dictionaries(&mut self, path: PathBuf) {
        self.start_resource_operation(move || {
            OperationResult::Dictionaries(
                dictionary_import::prepare(&path, false)
                    .map_err(|error| format!("{}：{error}", path.display())),
            )
        });
    }

    pub fn remove_flat_voice(&mut self, path: PathBuf) {
        let root = self.library.settings.flat_voice_directory();
        self.start_resource_operation(move || {
            let feedback = root.and_then(|root| {
                flat::remove_import(&root, &path)
                    .map(|()| "已移除导入的 Flat 声库。".into())
                    .map_err(|error| error.to_string())
            });
            OperationResult::Voices {
                changed: feedback.is_ok(),
                feedback,
            }
        });
    }

    pub(super) fn poll_resource_operation(&mut self) {
        let Some(receiver) = self.library.resource_operation.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => OperationResult::Voices {
                changed: false,
                feedback: Err("资源处理线程意外终止。".into()),
            },
            Err(TryRecvError::Empty) => return,
        };
        self.library.resource_operation = None;
        let feedback = match result {
            OperationResult::Voices { changed, feedback } => {
                if changed {
                    self.rescan_voices();
                }
                feedback
            }
            OperationResult::Dictionaries(result) => result.and_then(|directory| {
                let mut settings = self.library.settings.clone();
                settings.dictionaries = Some(directory.path().to_owned());
                if !self.library.update(settings) {
                    return Err(self
                        .library
                        .settings_error()
                        .unwrap_or("无法保存词典设置")
                        .to_owned());
                }
                let _ = directory.keep();
                self.clear_voices();
                Ok("已导入并启用发音词典。".into())
            }),
        };
        self.library.resource_feedback = Some(feedback);
    }

    fn start_resource_operation(
        &mut self,
        task: impl FnOnce() -> OperationResult + Send + 'static,
    ) {
        if self.library.resource_busy() || self.library.invalid_settings {
            return;
        }
        let repaint = self.repaint.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.library.resource_feedback = None;
        match thread::Builder::new()
            .name("voice-resource-operation".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let _ = sender.send(task());
            }) {
            Ok(_) => self.library.resource_operation = Some(receiver),
            Err(error) => {
                self.library.resource_feedback = Some(Err(format!("无法启动资源处理任务：{error}")))
            }
        }
    }
}
