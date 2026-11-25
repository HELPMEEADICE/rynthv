use super::{Worker, event, fs, json, pump_events, yield_events};
use crate::{
    platform::web::protocol::{ComputeDiagnostics, Event, Export, ExportMetrics, Preview, Visuals},
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
};
use web_time::Instant;

impl Worker {
    pub(super) fn render(
        &mut self,
        project: &str,
        preview: Option<fn() -> f64>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let started = Instant::now();
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
            preview,
        };
        let mut last_progress = String::new();
        let mut first_worker_block_ms = None;
        let output = self.offline.render(
            &project,
            &settings,
            &options,
            Arc::clone(&cancelled),
            |progress| {
                let stage = match progress {
                    Progress::Checkpoint => {
                        pump_events();
                        return;
                    }
                    Progress::VoiceLoaded { .. } => "声库已加载",
                    Progress::Stage { stage, .. } => stage.label(),
                    Progress::Phase(stage) => stage,
                    Progress::Counts(stats) => {
                        event(Event::Counts(stats), None);
                        return;
                    }
                    Progress::Layout { frames, occupied } => {
                        event(Event::Layout { frames, occupied }, None);
                        return;
                    }
                    Progress::Block {
                        index,
                        samples,
                        first,
                    } => {
                        if first {
                            first_worker_block_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                        }
                        event(Event::Block { index, first }, samples.as_deref());
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
                pump_events();
            },
        )?;
        if preview.is_none() {
            let directory = PathBuf::from("/RynthV/exports").join(uuid::Uuid::new_v4().to_string());
            let request = ExportRequest::with_directory(&project, Some(directory.clone()))?;
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let mut mix_ms = 0.0;
            let mut wav_ms = 0.0;
            let export = execution::scope(
                Cancellation::from_flag(cancelled),
                || -> Result<Vec<PathBuf>, String> {
                    let mut paths = Vec::new();
                    for target in request.outputs {
                        yield_events()?;
                        execution::checkpoint().map_err(|error| error.to_string())?;
                        let mixing = Instant::now();
                        let pcm = output.mix_stem(target.track, target.stem)?;
                        mix_ms += mixing.elapsed().as_secs_f64() * 1000.0;
                        if let Some(pcm) = pcm {
                            let writing = Instant::now();
                            audio::save_wav_policy(&target.path, &pcm, request.format, false)
                                .map_err(|error| error.to_string())?;
                            wav_ms += writing.elapsed().as_secs_f64() * 1000.0;
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
            return json(&Export {
                paths: export?,
                metrics: ExportMetrics {
                    render_ms: output.report.elapsed_ms,
                    mix_ms,
                    wav_ms,
                    worker_ms: started.elapsed().as_secs_f64() * 1000.0,
                },
                diagnostics: ComputeDiagnostics::for_render(
                    output.report.diagnostics,
                    output.report.cpu_activity,
                )?,
            });
        }

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
        let mut metrics = output.report.preview.expect("preview metrics");
        metrics.first_block_ms = first_worker_block_ms;
        metrics.worker_ms = started.elapsed().as_secs_f64() * 1000.0;
        json(&Preview {
            metrics,
            cache_bytes: output.report.cache_bytes,
            diagnostics: ComputeDiagnostics::for_render(
                output.report.diagnostics,
                output.report.cpu_activity,
            )?,
        })
    }
}
