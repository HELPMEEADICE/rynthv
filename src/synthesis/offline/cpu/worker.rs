use super::{
    super::voices::VoiceResources,
    protocol::{Completed, Job, Update},
};
use crate::{
    platform::web::cpu::progress,
    synthesis::{
        audio::RenderedAudio,
        engine::{OwnerSynthesis, RenderStage, progressive},
        execution,
        inference::compute::{Backend, Session},
        plan::OwnerInput,
        settings::SynthesisSettings,
        voice::{VoiceError, flat::FlatModelRegistry},
    },
};
use js_sys::{Array, Float32Array};
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use web_time::Instant;

const BASELINE_BUDGET: usize = 24 << 20;
const PARTIAL_BUDGET: usize = 24 << 20;

#[derive(Default)]
struct Worker {
    settings: Option<SynthesisSettings>,
    registry: Option<FlatModelRegistry>,
    voice: Option<(PathBuf, VoiceResources)>,
    completed: VecDeque<Baseline>,
    completed_bytes: usize,
    partial: Option<Partial>,
}

struct Baseline {
    serial: u64,
    input: Arc<OwnerInput>,
    state: OwnerSynthesis,
    bytes: usize,
}

struct Partial {
    input: Arc<OwnerInput>,
    baseline: Option<u64>,
    prepared: progressive::PreparedRender,
}

thread_local! {
    static WORKER: RefCell<Worker> = RefCell::default();
    static SESSION: Session = Session::new(Backend::Cpu, None);
}

pub(crate) fn run_job(
    job: Job,
    cancelled: &AtomicBool,
    demand: impl Fn() -> Option<f64>,
) -> Result<Array, String> {
    SESSION.with(|cpu| {
        cpu.take_diagnostics();
        cpu.scope(|| WORKER.with_borrow_mut(|worker| worker.run(job, cancelled, demand, cpu)))
    })
}

impl Worker {
    fn run(
        &mut self,
        job: Job,
        cancelled: &AtomicBool,
        demand: impl Fn() -> Option<f64>,
        cpu: &Session,
    ) -> Result<Array, String> {
        if self.settings.as_ref() != Some(&job.settings) {
            self.settings = Some(job.settings.clone());
            self.registry = None;
            self.voice = None;
        }
        let voice_load_ms = if self
            .voice
            .as_ref()
            .is_none_or(|(path, _)| *path != job.voice)
        {
            self.voice = None;
            self.completed.clear();
            self.completed_bytes = 0;
            self.partial = None;
            let started = Instant::now();
            let registry = self
                .registry
                .get_or_insert_with(|| job.settings.flat_registry());
            self.voice = Some((
                job.voice.clone(),
                VoiceResources::load(&job.voice, registry)?,
            ));
            Some(ms(started))
        } else {
            None
        };
        let resource = &self.voice.as_ref().expect("resident voice").1;
        let started = Instant::now();
        let program = resource.program(&job.settings);
        if cancelled.load(Ordering::Acquire) && resource.inventory.preparation().is_none() {
            // A cancelled model preparation must not leave its cached error resident.
            self.voice = None;
            return Err("CANCELLED: Render cancelled".into());
        }
        let program = program?;
        let network =
            progressive::Network::prepare(&program.voice).map_err(|error| error.to_string())?;
        progress(
            &Update::Model {
                voice_load_ms,
                model_prepare_ms: ms(started),
            },
            None,
        )?;
        execution::checkpoint().map_err(|error| error.to_string())?;

        let mut last_stage = None;
        let mut stage = |stage| {
            if last_stage.replace(stage) != Some(stage) {
                progress(&Update::Stage(stage), None).map_err(VoiceError::new)?;
            }
            execution::checkpoint()
        };
        let base = job.baseline.and_then(|serial| {
            let active = self.completed.iter().find(|item| item.serial == serial)?;
            Some((active.input.as_ref(), &active.state))
        });
        let partial = self
            .partial
            .take_if(|partial| partial.input == job.input && partial.baseline == job.baseline);
        let mut prepared = match partial {
            Some(partial) => partial.prepared,
            None => program
                .prepare(&job.input, base, &mut stage)
                .map_err(|error| error.to_string())?,
        };
        if job.preview {
            let visible = prepared.visible_samples();
            progress(
                &Update::Prepared {
                    first: visible.start,
                    end: visible.end,
                },
                None,
            )?;
            for audio in prepared.ready_audio().map_err(|error| error.to_string())? {
                publish(&audio)?;
            }
        } else {
            prepared.suppress_pcm_publication();
        }
        let mut render = || -> Result<(), VoiceError> {
            let mut cursor = None::<progressive::Cursor<'_, '_>>;
            while !prepared.is_complete() {
                execution::checkpoint()?;
                let target = demand().filter(|&target| prepared.needs_audio(target));
                if cursor.as_ref().is_none_or(|cursor| {
                    cursor.is_finished()
                        || target.is_some_and(|seconds| !cursor.covers_demand(seconds))
                }) {
                    stage(RenderStage::Timbre)?;
                    cursor = prepared.start(
                        &network,
                        &program.voice,
                        target.unwrap_or(job.start_seconds),
                    )?;
                }
                let cursor = cursor
                    .as_mut()
                    .ok_or_else(|| VoiceError::new("找不到待渲染音频的推理位置"))?;
                let batch = if target.is_some() { 64 } else { 128 };
                for audio in prepared.advance(cursor, batch, &mut stage)? {
                    publish(&audio).map_err(VoiceError::new)?;
                }
            }
            execution::checkpoint()
        };
        if let Err(error) = render() {
            if cancelled.load(Ordering::Acquire) {
                let samples = prepared.model_samples();
                let bytes =
                    ((samples.end - samples.start) as usize).saturating_mul(2 * size_of::<f32>());
                if bytes <= PARTIAL_BUDGET {
                    self.partial = Some(Partial {
                        input: job.input,
                        baseline: job.baseline,
                        prepared,
                    });
                }
            }
            return Err(error.to_string());
        }
        let complete = prepared
            .into_completed()
            .map_err(|error| error.to_string())?;
        let components = &complete.state.components;
        let phrase = &complete.state.prepared.phrase;
        let message = Completed {
            pcm_origin: components.start_seconds,
            visible_begin: phrase.visible_begin_seconds,
            visible_end: phrase.visible_end_seconds,
            pitch: complete.pitch,
            phonemes: complete.phonemes,
            timings: complete.timings,
            activity: cpu
                .take_diagnostics()
                .activity
                .cpu_operators
                .into_iter()
                .map(|(name, reasons)| {
                    (
                        name.to_owned(),
                        reasons
                            .into_iter()
                            .map(|(reason, count)| (reason.to_owned(), count))
                            .collect(),
                    )
                })
                .collect(),
        };
        let text = serde_json::to_string(&message).map_err(|error| error.to_string())?;
        let packet = Array::of3(
            &text.into(),
            &Float32Array::from(components.primary.as_ref()),
            &Float32Array::from(components.secondary.as_ref()),
        );
        self.remember(job.serial, job.input, complete.state);
        Ok(packet)
    }

    fn remember(&mut self, serial: u64, input: Arc<OwnerInput>, state: OwnerSynthesis) {
        let bytes = state.memory_bytes();
        if bytes > BASELINE_BUDGET {
            return;
        }
        while self.completed_bytes + bytes > BASELINE_BUDGET {
            self.completed_bytes -= self.completed.pop_front().expect("retained baseline").bytes;
        }
        self.completed_bytes += bytes;
        self.completed.push_back(Baseline {
            serial,
            input,
            state,
            bytes,
        });
    }
}

fn publish(audio: &RenderedAudio) -> Result<(), String> {
    progress(
        &Update::Pcm {
            seconds: audio.start_seconds(),
        },
        Some(audio.samples()),
    )
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
