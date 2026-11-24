//! Decoded audio belongs to currently referenced files, not undo history.
mod decode;

use super::{audio::RenderedAudio, rendered::Waveform};
use crate::model::Project;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

pub struct AudioAsset {
    pub audio: Arc<RenderedAudio>,
    pub waveforms: Vec<Waveform>,
}

enum Entry {
    Loading {
        rx: mpsc::Receiver<Result<Arc<AudioAsset>, String>>,
        cancel: Arc<AtomicBool>,
    },
    Ready(Arc<AudioAsset>),
    Failed(String),
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Self::Loading { cancel, .. } = self {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
pub struct AudioLibrary {
    entries: BTreeMap<PathBuf, Entry>,
    import: Option<PathBuf>,
    pub generation: u64,
}

impl AudioLibrary {
    pub fn begin_import(&mut self, path: PathBuf, ctx: &super::notify::Notify) {
        self.import = Some(path.clone());
        self.retry(&path, ctx);
    }

    pub fn finish_import(&mut self) {
        if self.import.take().is_some() {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn reconcile(&mut self, project: &Project, ctx: &super::notify::Notify) {
        let mut paths: BTreeSet<_> = project
            .tracks
            .iter()
            .filter_map(|t| t.audio_clip().map(|c| c.path.clone()))
            .collect();
        paths.extend(self.import.iter().cloned());
        self.entries.retain(|path, _| paths.contains(path));
        for path in paths {
            self.request(path, ctx);
        }
    }

    pub fn poll(&mut self) {
        for entry in self.entries.values_mut() {
            let Entry::Loading { rx, .. } = entry else {
                continue;
            };
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => Err("音频加载线程意外停止".into()),
            };
            *entry = match result {
                Ok(asset) => Entry::Ready(asset),
                Err(error) => Entry::Failed(error),
            };
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn asset(&self, path: &Path) -> Option<&Arc<AudioAsset>> {
        match self.entries.get(path) {
            Some(Entry::Ready(asset)) => Some(asset),
            _ => None,
        }
    }

    pub fn error(&self, path: &Path) -> Option<&str> {
        match self.entries.get(path) {
            Some(Entry::Failed(error)) => Some(error),
            _ => None,
        }
    }

    pub fn retry(&mut self, path: &Path, ctx: &super::notify::Notify) {
        self.entries.remove(path);
        self.request(path.to_owned(), ctx);
        self.generation = self.generation.wrapping_add(1);
    }

    fn request(&mut self, path: PathBuf, ctx: &super::notify::Notify) {
        if self.entries.contains_key(&path) {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let file = path.clone();
        let ctx = ctx.clone();
        let result = thread::Builder::new()
            .name("audio-import".into())
            .spawn(move || {
                let result = decode::load(&file, &worker_cancel)
                    .map(Arc::new)
                    .map_err(|error| format!("{}：{error}", file.display()));
                if !worker_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(result);
                    ctx.wake();
                }
            });
        self.entries.insert(
            path,
            match result {
                Ok(_) => Entry::Loading { rx, cancel },
                Err(error) => Entry::Failed(format!("无法加载音频：{error}")),
            },
        );
    }
}
