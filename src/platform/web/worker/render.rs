use super::{Worker, event, fs, json, yield_events};
use crate::{
    platform::web::protocol::{Event, Preview, Visuals},
    synthesis::{
        audio,
        execution::{self, Cancellation},
        export_request::ExportRequest,
        offline::{Options, Progress},
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use web_time::Instant;

impl Worker {
    pub(super) fn render(
        &mut self,
        project: &str,
        preview: Option<f64>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let project = crate::storage::decode_project(project.as_bytes())?.project;
        let settings = self.settings().clone();
        let options = Options {
            tracks: Default::default(),
            group: None,
            range_seconds: None,
            vocals_only: false,
            backend: settings.compute_backend,
            adapter: settings.compute_adapter.clone(),
            cache_bytes: 96 << 20,
            preview_seconds: preview,
        };
        let mut last_yield = Instant::now();
        let mut last_progress = String::new();
        let output = self.offline.render(
            &project,
            &settings,
            &options,
            Arc::clone(&cancelled),
            |progress| {
                if last_yield.elapsed() >= Duration::from_millis(12) {
                    let _ = yield_events();
                    last_yield = Instant::now();
                }
                let stage = match progress {
                    Progress::VoiceLoaded { .. } => "声库已加载",
                    Progress::Stage { stage, .. } => stage.label(),
                    Progress::Pcm { group, audio } => {
                        event(
                            Event::Chunk {
                                group,
                                origin: audio.start_seconds(),
                            },
                            Some(audio.samples()),
                        );
                        return;
                    }
                };
                if last_progress != stage {
                    event(
                        Event::Progress {
                            stage: stage.into(),
                        },
                        None,
                    );
                    last_progress = stage.into();
                }
            },
        )?;
        if preview.is_none() {
            let directory = PathBuf::from("/RynthV/exports").join(uuid::Uuid::new_v4().to_string());
            let request = ExportRequest::with_directory(&project, Some(directory.clone()))?;
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let export = execution::scope(
                Cancellation::from_flag(cancelled),
                || -> Result<Vec<PathBuf>, String> {
                    let mut paths = Vec::new();
                    for target in request.outputs {
                        yield_events()?;
                        execution::checkpoint().map_err(|error| error.to_string())?;
                        if let Some(pcm) = output.mix_stem(target.track, target.stem)? {
                            audio::save_wav_policy(&target.path, &pcm, request.format, false)
                                .map_err(|error| error.to_string())?;
                            paths.push(target.path);
                        }
                    }
                    if paths.is_empty() {
                        return Err("所选音轨没有可导出的音频".into());
                    }
                    Ok(paths)
                },
            );
            if export.is_err() {
                let _ = fs::remove_dir_all(&directory);
            }
            return json(&export?);
        }

        let mut duration = None;
        for owner in &output.report.owners {
            let data = &owner.owner.visuals;
            let origin = owner.owner.audio.start_seconds();
            let mut peaks = Vec::new();
            data.waveform.append_peaks(&mut peaks);
            event(
                Event::Visuals(Box::new(Visuals {
                    group: owner.group_ref_id.clone(),
                    pitch: data.pitch.clone(),
                    phones: data.phonemes.clone(),
                    note_ids: owner.note_ids.clone(),
                    origin,
                    frames: owner.owner.audio.frames(),
                })),
                Some(&peaks),
            );
        }
        if let Some(mix) = output.mix(None)? {
            duration = Some(mix.duration_seconds());
            for (index, samples) in mix.samples().chunks(65_536 * 2).enumerate() {
                event(
                    Event::Mix {
                        origin: mix.start_seconds() + (index * 65_536) as f64 / 48_000.0,
                    },
                    Some(samples),
                );
            }
        }
        json(&Preview {
            duration,
            rendered: output.report.rendered_owners,
            reused: output.report.reused_owners,
            cache_bytes: output.report.cache_bytes,
            diagnostics: output.report.diagnostics.try_into()?,
        })
    }
}
