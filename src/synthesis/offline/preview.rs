mod cpu;
mod recordings;
mod scheduler;

pub(super) use recordings::RecordingCache;
pub(super) use scheduler::Partial;

use super::{
    voices::{VoiceProgram, voice_resource},
    *,
};
use crate::synthesis::{
    execution::{self, Cancellation},
    mixer::{PreviewMixer, PreviewSource, block_at},
    plan::RenderPlan,
    render_cache::{Completion, RenderJob},
    rendered::RenderedClip,
};
use crate::tr;
use scheduler::Scheduler;
use serde::Serialize;
use std::cell::OnceCell;

#[derive(Default, Serialize, serde::Deserialize)]
pub struct PreviewMetrics {
    pub duration: Option<f64>,
    pub first_block_ms: Option<f64>,
    pub worker_ms: f64,
    pub voice_load_ms: f64,
    pub model_prepare_ms: f64,
    pub recording_ms: f64,
    pub mix_ms: f64,
    pub stages: StageTimings,
    pub error: Option<String>,
}

impl Offline {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_preview(
        &mut self,
        project: &Project,
        settings: &SynthesisSettings,
        options: &Options,
        routes: &BTreeMap<VoiceSelection, Vec<OwnerId>>,
        group_names: &BTreeMap<OwnerId, String>,
        cancelled: &Arc<AtomicBool>,
        solo: &BTreeSet<u64>,
        accompaniment: &[&Track],
        progress: &mut impl FnMut(Progress),
    ) -> Result<(BTreeMap<OwnerId, StageTimings>, PreviewMetrics), String> {
        let demand = options.preview.expect("preview demand");
        let plan = self.prepare_owners(project, options, routes, cancelled, progress)?;
        let use_cpu_workers = self.use_cpu_workers(options.backend);
        let compute = self.compute.as_ref().expect("preview compute").2.clone();
        if !routes.is_empty() {
            compute
                .check()
                .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        }
        let pending = mixer::recordings(accompaniment.iter().copied(), solo, |_| None)?.pending;
        let mixer = PreviewMixer::new(
            &plan,
            accompaniment.iter().filter_map(|track| {
                pending
                    .get(&track.id)
                    .map(|range| (track.id, range.clone()))
            }),
        )?;
        let (frames, occupied) = mixer.layout();
        progress(Progress::Layout { frames, occupied });
        let mut scheduler = Scheduler::new(&plan, routes, mixer);
        let mut times = BTreeMap::new();
        let mut metrics = PreviewMetrics::default();
        let started = Instant::now();
        let library = self.library.as_ref().expect("scanned library");
        let cancellation = Cancellation::from_flag(Arc::clone(cancelled));
        let events = std::cell::RefCell::new(progress);
        let emit = |event| events.borrow_mut()(event);
        let mut load_recording =
            |id, scheduler: &mut Scheduler<'_, '_, '_>, metrics: &mut PreviewMetrics| {
                emit(Progress::Phase(tr!("读取伴奏")));
                let track = accompaniment
                    .iter()
                    .find(|track| track.id == id)
                    .expect("indexed recording");
                let clip = track.audio_clip().expect("recording clip");
                let started = Instant::now();
                let asset = self.recordings.load(&clip.path, cancelled);
                metrics.recording_ms += ms(started);
                let layer = match asset {
                    Ok(asset) => mixer::recordings([*track], solo, |_| Some(Ok(asset.as_ref())))?
                        .layers
                        .pop(),
                    Err(error) => {
                        metrics.error.get_or_insert(error);
                        scheduler.recording_failed(id);
                        return Ok(());
                    }
                };
                scheduler.recording(id, layer, pending[&id].clone());
                Ok::<(), String>(())
            };
        scheduler.seed(&self.cache, &mut self.partial)?;
        if use_cpu_workers {
            let result = execution::scope(cancellation, || {
                self.cpu_pool.run_batch(|pool| {
                    cpu::render(
                        pool,
                        &mut scheduler,
                        &mut self.cache,
                        settings,
                        library,
                        group_names,
                        demand,
                        cancelled,
                        started,
                        &mut metrics,
                        &mut load_recording,
                        &emit,
                    )
                })
            });
            self.cache.requeue_running();
            times = result?;
            metrics.duration = scheduler.duration();
            metrics.worker_ms = ms(started);
            return Ok((times, metrics));
        }
        let programs: BTreeMap<_, OnceCell<VoiceProgram<'_>>> =
            routes.keys().map(|key| (key, OnceCell::new())).collect();
        let networks: BTreeMap<_, OnceCell<Result<progressive::Network<'_>, String>>> =
            routes.keys().map(|key| (key, OnceCell::new())).collect();
        let result: Result<(), String> = compute.scope(|| {
            execution::scope(cancellation.clone(), || {
                loop {
                    emit(Progress::Checkpoint);
                    if cancellation.is_cancelled() {
                        return Err("CANCELLED: Render cancelled".into());
                    }
                    let demand = block_at(demand());
                    let more = scheduler
                        .publish(demand, started, &mut metrics, &mut |event| emit(event))?;
                    let Some(work) = scheduler.next(demand, |_| false) else {
                        if more {
                            continue;
                        }
                        break;
                    };
                    match work {
                        (PreviewSource::Recording(id), _) => {
                            load_recording(id, &mut scheduler, &mut metrics)?;
                        }
                        (PreviewSource::Vocal(id), target) => {
                            let result = scheduler.advance(
                                id,
                                target,
                                &mut self.cache,
                                &mut self.partial,
                                |key| {
                                    let slot = &programs[key];
                                    if networks[key].get().is_none() {
                                        emit(Progress::Phase(tr!("准备声库与模型")));
                                        let started = Instant::now();
                                        let mut loaded_ms = 0.0;
                                        let program =
                                            voice_resource(library, key, &self.voices[key])
                                                .and_then(|(resource, loaded)| {
                                                    loaded_ms = loaded.unwrap_or_default();
                                                    metrics.voice_load_ms += loaded_ms;
                                                    resource.program(settings)
                                                });
                                        let network = program.and_then(|program| {
                                            let _ = slot.set(program);
                                            progressive::Network::prepare(
                                                &slot.get().expect("prepared program").voice,
                                            )
                                            .map_err(|error| format!("VOICE_RESOURCE: {error}"))
                                        });
                                        metrics.model_prepare_ms += ms(started) - loaded_ms;
                                        let _ = networks[key].set(network);
                                    }
                                    match networks[key].get().expect("prepared network") {
                                        Ok(network) => {
                                            Ok((slot.get().expect("prepared program"), network))
                                        }
                                        Err(error) => Err(error.clone()),
                                    }
                                },
                                |update| match update {
                                    scheduler::Update::Counts(stats) => {
                                        emit(Progress::Counts(stats))
                                    }
                                    scheduler::Update::Stage(stage) => emit(Progress::Stage {
                                        group: group_names[&id].clone(),
                                        stage,
                                    }),
                                },
                            );
                            if cancellation.is_cancelled() {
                                return Err("CANCELLED: Render cancelled".into());
                            }
                            if let Some(timing) = result? {
                                add_stages(&mut metrics.stages, &timing);
                                times.insert(id, timing);
                            }
                            if let Some(error) = self.cache.failure(id) {
                                metrics.error.get_or_insert_with(|| {
                                    format!("SYNTHESIS_FAILED: {}: {error}", group_names[&id])
                                });
                            }
                        }
                    }
                }
                Ok(())
            })
        });
        scheduler.save_partials(&mut self.partial);
        self.cache.requeue_running();
        result?;
        if !routes.is_empty() {
            compute
                .check()
                .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        }
        metrics.duration = scheduler.duration();
        metrics.worker_ms = ms(started);
        Ok((times, metrics))
    }
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn add_stages(total: &mut StageTimings, timing: &StageTimings) {
    total.duration_ms += timing.duration_ms;
    total.pitch_ms += timing.pitch_ms;
    total.timbre_ms += timing.timbre_ms;
    total.vocoder_ms += timing.vocoder_ms;
    total.visuals_ms += timing.visuals_ms;
}
