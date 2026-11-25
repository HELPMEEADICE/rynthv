//! Decoded audio belongs to currently referenced files, not undo history.
mod decode;

use super::{audio::RenderedAudio, rendered::Waveform};
use crate::model::Project;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub struct AudioAsset {
    pub audio: Arc<RenderedAudio>,
    pub waveforms: Vec<Waveform>,
}

pub fn read_audio(path: &Path, cancelled: &AtomicBool) -> Result<RenderedAudio, String> {
    decode::decode(path, cancelled)
}

impl AudioAsset {
    pub(crate) fn read_for_mix(path: &Path, cancelled: &AtomicBool) -> Result<Self, String> {
        decode::load(path, cancelled, false)
    }
}

enum Entry {
    Loading {
        rx: mpsc::Receiver<Result<Arc<AudioAsset>, String>>,
        cancel: Arc<AtomicBool>,
        #[cfg(target_arch = "wasm32")]
        web: crate::platform::web::client::Pending<()>,
    },
    Ready(Arc<AudioAsset>),
    Failed(String),
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Self::Loading { cancel, .. } = self {
            cancel.store(true, Ordering::Relaxed);
        }
        #[cfg(target_arch = "wasm32")]
        if let Self::Loading { web, .. } = self {
            web.cancel();
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
                Err(mpsc::TryRecvError::Disconnected) => {
                    #[cfg(target_arch = "wasm32")]
                    if let Entry::Loading { web, .. } = entry
                        && let Some(Err(error)) = web.poll()
                    {
                        *entry = Entry::Failed(error);
                        self.generation = self.generation.wrapping_add(1);
                        continue;
                    }
                    Err("音频加载任务意外停止".into())
                }
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
        #[cfg(not(target_arch = "wasm32"))]
        let result = thread::Builder::new()
            .name("audio-import".into())
            .spawn(move || {
                let result = decode::load(&file, &worker_cancel, true)
                    .map(Arc::new)
                    .map_err(|error| format!("{}：{error}", file.display()));
                if !worker_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(result);
                    ctx.wake();
                }
            });
        #[cfg(target_arch = "wasm32")]
        let result: Result<crate::platform::web::client::Pending<()>, std::io::Error> = {
            use crate::platform::web::{
                client,
                protocol::{Event, Request},
            };
            use crate::synthesis::audio::ChannelLayout;
            let pending = client::submit::<()>(
                Request::Audio { path: file },
                ctx.clone(),
                move |event, samples| {
                    if let (Event::Audio { channels, rate }, Some(samples)) = (event, samples) {
                        let layout = if channels == 1 {
                            ChannelLayout::Mono
                        } else {
                            ChannelLayout::Stereo
                        };
                        let asset = RenderedAudio::new(rate, 0.0, layout, samples.to_vec())
                            .map(|audio| {
                                let waveforms = (0..channels)
                                    .map(|channel| Waveform::new_channel(&audio, channel))
                                    .collect();
                                Arc::new(AudioAsset {
                                    audio: Arc::new(audio),
                                    waveforms,
                                })
                            })
                            .map_err(|error| error.to_string());
                        if !worker_cancel.load(Ordering::Relaxed) {
                            let _ = tx.send(asset);
                            ctx.wake();
                        }
                    }
                },
            );
            // The data event precedes successful completion. Forward a worker
            // failure separately because it has no data packet.
            Ok(pending)
        };
        self.entries.insert(
            path,
            match result {
                Ok(_task) => Entry::Loading {
                    rx,
                    cancel,
                    #[cfg(target_arch = "wasm32")]
                    web: _task,
                },
                Err(error) => Entry::Failed(format!("无法加载音频：{error}")),
            },
        );
    }
}
