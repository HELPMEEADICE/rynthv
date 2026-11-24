use super::{
    Runtime,
    events::WakeOnExit,
    mixer::{self, VocalStem},
};
use crate::{
    model::Project,
    synthesis::audio::{self, ChannelLayout, WaveFormat},
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

pub struct ExportRequest {
    outputs: Vec<Output>,
    directory: PathBuf,
    format: WaveFormat,
}

struct Output {
    path: PathBuf,
    track: Option<u64>,
    stem: VocalStem,
}

impl ExportRequest {
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.outputs.iter().map(|output| output.path.as_path())
    }

    pub fn from_project(project: &Project) -> Result<Self, String> {
        let config = &project.render_config;
        crate::synthesis::mixing_options::validate_export_bypass(config)?;
        crate::synthesis::mixing_options::validate_project_mixer(
            project.extra.get("projectMixer"),
        )?;
        for track in &project.tracks {
            crate::synthesis::mixing_options::validate_mixer(&track.mixer)
                .map_err(|error| format!("音轨 {}：{error}", track.name))?;
        }
        let text = |key: &str| {
            config
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        let number = |key: &str| {
            config
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        let directory = PathBuf::from(text("destination"));
        if directory.as_os_str().is_empty() || (directory.exists() && !directory.is_dir()) {
            return Err("请选择有效的目标文件夹".into());
        }
        let name = text("filename").trim();
        let stem = name
            .rsplit_once('.')
            .filter(|(_, extension)| extension.eq_ignore_ascii_case("wav"))
            .map_or(name, |(stem, _)| stem);
        if name.is_empty()
            || name.ends_with(['.', ' '])
            || stem.is_empty()
            || stem.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        {
            return Err("文件名称为空或含有无效字符".into());
        }
        if reserved_name(stem) {
            return Err("请使用有效的文件名称".into());
        }
        let sample_rate = number("sampleRate");
        let bit_depth = number("bitDepth");
        if ![44100, 48000, 96000].contains(&sample_rate) || ![16, 24, 32].contains(&bit_depth) {
            return Err("请选择有效的采样率和位深度".into());
        }
        let layout = match number("numChannels") {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => return Err("请选择有效的声道数".into()),
        };
        let separate_aspiration = match text("aspirationFormat") {
            "noAspiration" => false,
            "asFiles" => true,
            _ => return Err("请选择有效的气声输出格式".into()),
        };
        let mut outputs = Vec::new();
        let mut add_output = |name: String, track: Option<u64>, vocal: bool| {
            let split = separate_aspiration && vocal;
            outputs.push(Output {
                path: directory.join(format!("{name}.wav")),
                track,
                stem: if split {
                    VocalStem::Primary
                } else {
                    VocalStem::Combined
                },
            });
            if split {
                outputs.push(Output {
                    path: directory.join(format!("{name}-aspiration.wav")),
                    track,
                    stem: VocalStem::Aspiration,
                });
            }
        };
        if config
            .get("exportMixDown")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            add_output(
                stem.to_owned(),
                None,
                project.tracks.iter().any(|track| !track.is_instrumental()),
            );
        }
        for (index, track) in project.tracks.iter().enumerate() {
            if track
                .extra
                .get("renderEnabled")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
            {
                continue;
            }
            let safe: String = track
                .name
                .chars()
                .map(|c| {
                    if c.is_control() || "<>:\"/\\|?*".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            add_output(
                format!(
                    "{stem}-{:02}-{}",
                    index + 1,
                    safe.trim_end_matches(['.', ' '])
                ),
                Some(track.id),
                !track.is_instrumental(),
            );
        }
        if outputs.is_empty() {
            return Err("请勾选混音导出或至少一条音轨".into());
        }
        Ok(Self {
            outputs,
            directory,
            format: WaveFormat {
                sample_rate: sample_rate as u32,
                layout,
                bit_depth: bit_depth as u16,
            },
        })
    }
}

fn reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (1..=9).any(|n| base == format!("COM{n}") || base == format!("LPT{n}"))
}

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
                .unwrap_or_else(|| "请等待当前工程合成完成后再导出".into());
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
            self.export_result = Some(Err("请等待所选伴奏加载完成后再导出".into()));
            return;
        }
        let owners = self.export_owners.clone();
        let recordings = self.recordings.layers.clone();
        let solo = self.solo_tracks.clone();
        let repaint = self.repaint.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.export_result = None;
        match thread::Builder::new()
            .name("audio-export".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let sender = tx;
                let result = (|| {
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
                            return Err(format!(
                                "{} 没有可导出的音频，请检查音轨是否为空、静音或被独奏排除",
                                output.path.display()
                            ));
                        }
                    }
                    std::fs::create_dir_all(&request.directory).map_err(|error| {
                        format!(
                            "无法创建导出文件夹 {}：{error}",
                            request.directory.display()
                        )
                    })?;
                    let mut written = Vec::with_capacity(outputs.len());
                    for output in outputs {
                        let audio = mixer::mix_with_recordings(
                            &owners,
                            &solo,
                            recordings_for(output.stem),
                            output.track,
                            output.stem,
                        )?
                        .ok_or_else(|| {
                            format!(
                                "{} 没有可导出的音频，请检查音轨是否为空、静音或被独奏排除",
                                output.path.display()
                            )
                        })?;
                        audio::save_wav_as(&output.path, &audio, request.format)
                            .map_err(|e| format!("{e}（此前已完成 {} 个文件）", written.len()))?;
                        written.push(output.path.clone());
                    }
                    Ok(written)
                })();
                let _ = sender.send(result);
            }) {
            Ok(_) => self.export = Some(rx),
            Err(error) => self.export_result = Some(Err(format!("无法启动音频导出：{error}"))),
        }
    }
}
