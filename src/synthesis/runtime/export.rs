pub use super::super::export_request::ExportRequest;
use super::{
    Runtime,
    events::WakeOnExit,
    mixer::{self, VocalStem},
};
use crate::synthesis::audio;
use std::{sync::mpsc, thread};
use crate::{tr, trf};

impl Runtime {
    pub fn export_ready(&self, revision: u64) -> bool {
        self.last_revision == Some(revision)
            && self.last_audio_generation == self.instrumentals.generation
            && self.export.is_none()
            && self.recording_error.is_none()
            && self.pending_voices.is_empty()
            && (self
                .planner
                .plan()
                .requests
                .keys()
                .all(|id| !self.routes.contains_key(id) && !self.binding_errors.contains_key(id))
                || (!self.is_busy() && self.stats.failed == 0 && self.error.is_none()))
            && (!self.export_owners.is_empty() || !self.recordings.layers.is_empty())
    }

    pub fn export_project(&mut self, request: ExportRequest, revision: u64) {
        if !self.export_ready(revision) {
            let error = self
                .recording_error
                .clone()
                .unwrap_or_else(|| tr!("请等待当前工程合成完成后再导出").into());
            self.export_result = Some(Err(error));
            return;
        }
        let includes = |track_id| {
            request.outputs.iter().any(|output| {
                output.stem != VocalStem::Aspiration
                    && output.track.is_none_or(|track| track == track_id)
            })
        };
        if let Some((_, error)) = self
            .recordings
            .failures
            .iter()
            .find(|(id, _)| includes(**id))
        {
            self.export_result = Some(Err(error.clone()));
            return;
        }
        if self.recordings.pending.keys().any(|id| includes(*id)) {
            self.export_result = Some(Err(tr!("请等待所选伴奏加载完成后再导出").into()));
            return;
        }
        let owners = self.export_owners.clone();
        let recordings = self.recordings.layers.clone();
        let solo = self.solo_tracks.clone();
        let repaint = self.repaint.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.export_result = None;
        #[cfg(target_os = "android")]
        let cancellation = crate::platform::android::lifecycle::export_cancellation();
        match thread::Builder::new()
            .name("audio-export".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let sender = tx;
                let export = || {
                    // Validate all selected sources before writing the first file.
                    let recordings_for = |stem| {
                        if stem == VocalStem::Aspiration {
                            &[][..]
                        } else {
                            recordings.as_slice()
                        }
                    };
                    let mut outputs = Vec::with_capacity(request.outputs.len());
                    for output in &request.outputs {
                        if mixer::can_mix_with_recordings(
                            &owners,
                            &solo,
                            recordings_for(output.stem),
                            output.track,
                            output.stem,
                        )? {
                            outputs.push(output);
                        } else if output.stem != VocalStem::Aspiration {
                            return Err(trf!("{} 没有可导出的音频，请检查音轨是否为空、静音或被独奏排除", output.path.display()));
                        }
                    }
                    std::fs::create_dir_all(&request.directory).map_err(|error| {
                        trf!("无法创建导出文件夹 {}：{}", request.directory.display(), error)
                    })?;
                    let mut written = Vec::with_capacity(outputs.len());
                    for output in outputs {
                        let audio = mixer::mix_with_recordings(
                            &owners,
                            &solo,
                            recordings_for(output.stem),
                            output.track,
                            output.stem,
                            None,
                        )?
                        .ok_or_else(|| {
                            trf!("{} 没有可导出的音频，请检查音轨是否为空、静音或被独奏排除", output.path.display())
                        })?;
                        audio::save_wav_as(&output.path, &audio, request.format)
                            .map_err(|e| trf!("{}（此前已完成 {} 个文件）", e, written.len()))?;
                        written.push(output.path.clone());
                    }
                    Ok(written)
                };
                #[cfg(not(target_os = "android"))]
                let result = export();
                #[cfg(target_os = "android")]
                let result = crate::synthesis::execution::scope(cancellation.clone(), export)
                    .map_err(|error| {
                        if cancellation.is_cancelled() {
                            tr!("应用已进入后台，WAV 渲染已中止。").to_owned()
                        } else {
                            error
                        }
                    });
                let _ = sender.send(result);
            }) {
            Ok(_) => self.export = Some(rx),
            Err(error) => self.export_result = Some(Err(trf!("无法启动音频导出：{}", error))),
        }
    }
}
