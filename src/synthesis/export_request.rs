use super::{
    audio::{ChannelLayout, WaveFormat},
    mixer::VocalStem,
};
use crate::model::Project;
use crate::{tr, trf};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

pub struct ExportRequest {
    pub(crate) outputs: Vec<Output>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) directory: PathBuf,
    pub(crate) format: WaveFormat,
}

pub(crate) struct Output {
    pub(crate) path: PathBuf,
    pub(crate) track: Option<u64>,
    pub(crate) stem: VocalStem,
}

impl ExportRequest {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.outputs.iter().map(|output| output.path.as_path())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_project(project: &Project) -> Result<Self, String> {
        Self::with_directory(project, None)
    }

    pub(crate) fn with_directory(
        project: &Project,
        directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        let config = &project.render_config;
        crate::synthesis::mixing_options::validate_export_bypass(config)?;
        crate::synthesis::mixing_options::validate_project_mixer(
            project.extra.get("projectMixer"),
        )?;
        for track in &project.tracks {
            crate::synthesis::mixing_options::validate_mixer(&track.mixer)
                .map_err(|error| trf!("音轨 {}：{}", track.name, error))?;
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
        let directory = directory.unwrap_or_else(|| PathBuf::from(text("destination")));
        if directory.as_os_str().is_empty() {
            return Err(tr!("请选择有效的目标文件夹").into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if directory.exists() && !directory.is_dir() {
            return Err(tr!("请选择有效的目标文件夹").into());
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
            return Err(tr!("文件名称为空或含有无效字符").into());
        }
        if reserved_name(stem) {
            return Err(tr!("请使用有效的文件名称").into());
        }
        let sample_rate = number("sampleRate");
        let bit_depth = number("bitDepth");
        if ![44100, 48000, 96000].contains(&sample_rate) || ![16, 24, 32].contains(&bit_depth) {
            return Err(tr!("请选择有效的采样率和位深度").into());
        }
        let layout = match number("numChannels") {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => return Err(tr!("请选择有效的声道数").into()),
        };
        let separate_aspiration = match text("aspirationFormat") {
            "noAspiration" => false,
            "asFiles" => true,
            _ => return Err(tr!("请选择有效的气声输出格式").into()),
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
            return Err(tr!("请勾选混音导出或至少一条音轨").into());
        }
        Ok(Self {
            outputs,
            #[cfg(not(target_arch = "wasm32"))]
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
