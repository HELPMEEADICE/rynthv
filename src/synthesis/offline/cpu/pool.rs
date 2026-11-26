use super::{
    super::{OwnerId, StageTimings, SynthesisSettings},
    protocol::{Activity, Completed, Job, Update},
};
use crate::tr;
use crate::{
    platform::web::cpu::{
        cancel_cpu_job, cpu_slots, poll_cpu_packet, prioritize_cpu_job, start_cpu_job, stop_jobs,
    },
    synthesis::{
        audio::{ChannelLayout, RenderedAudio},
        engine::PcmComponents,
        render_cache::RenderJob,
        rendered::{RenderState, RenderVisuals, RenderedClip, Waveform},
        score::SAMPLE_RATE,
    },
};
use js_sys::{Array, Float32Array};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use wasm_bindgen::{JsCast, JsValue};
use web_time::Instant;

#[derive(Default)]
pub(in crate::synthesis::offline) struct Pool {
    slots: Vec<Slot>,
    activity: Activity,
}

#[derive(Default)]
struct Slot {
    voice: Option<PathBuf>,
    recent: Option<OwnerId>,
    active: Option<Active>,
}

pub(in crate::synthesis::offline) struct Active {
    pub job: RenderJob,
    pub preempted: bool,
    demand: Option<f64>,
}

pub(in crate::synthesis::offline) enum JobEvent {
    Progress(Update, Option<RenderedAudio>),
    Done(Box<Result<(Arc<RenderedClip>, StageTimings), String>>),
}

impl Pool {
    pub fn run_batch<T>(
        &mut self,
        run: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.slots.is_empty() {
            self.slots.resize_with(cpu_slots(), Slot::default);
        }
        let result = run(self);
        // Release the parent resource lock only after all child jobs have
        // unwound, including the jobs whose PCM will never be published.
        let stopped = stop_jobs();
        for slot in &mut self.slots {
            slot.active = None;
        }
        result.and_then(|value| stopped.map(|()| value))
    }

    pub fn clear_resources(&mut self) {
        self.slots.clear();
    }

    pub fn take_activity(&mut self) -> Activity {
        std::mem::take(&mut self.activity)
    }

    pub fn available(&self) -> bool {
        self.slots.iter().any(|slot| slot.active.is_none())
    }

    pub fn active(&self) -> impl Iterator<Item = (usize, &Active)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| Some((index, slot.active.as_ref()?)))
    }

    pub fn release(&mut self, index: usize) -> Active {
        self.slots[index].active.take().expect("scheduled CPU job")
    }

    pub fn start(
        &mut self,
        job: RenderJob,
        voice: &Path,
        settings: &SynthesisSettings,
        preview: bool,
        demand: Option<f64>,
    ) -> Result<usize, String> {
        let owner = job.request.owner;
        let base = job.base.as_ref().and_then(|base| match base.clip.state {
            RenderState::CpuWorker { slot, serial, .. } => Some((slot, serial)),
            RenderState::Local(_) => None,
        });
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .filter(|(_, slot)| slot.active.is_none())
            .max_by_key(|(index, slot)| {
                (
                    base.is_some_and(|(source, _)| source == *index),
                    slot.voice.as_deref() == Some(voice),
                    slot.recent == Some(owner),
                    slot.voice.is_none(),
                    std::cmp::Reverse(*index),
                )
            })
            .ok_or(tr!("CPU Worker 池已满"))?;
        let coverage = job.request.coverage;
        let payload = Job {
            serial: job.serial(),
            baseline: base
                .filter(|(source, _)| *source == index)
                .map(|(_, serial)| serial),
            voice: voice.to_owned(),
            settings: settings.clone(),
            input: Arc::clone(&job.request.input),
            start_seconds: coverage.sample_start.0 as f64 / f64::from(coverage.sample_rate),
            preview,
            demand,
        };
        let json = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        slot.voice = Some(payload.voice);
        slot.recent = Some(owner);
        slot.active = Some(Active {
            job,
            preempted: false,
            demand,
        });
        start_cpu_job(index, &json);
        Ok(index)
    }

    pub fn prioritize(&mut self, targets: &BTreeMap<OwnerId, f64>) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(active) = slot.active.as_mut().filter(|active| !active.preempted) else {
                continue;
            };
            let target = targets.get(&active.job.request.owner).copied();
            if target != active.demand {
                active.demand = target;
                prioritize_cpu_job(index, target);
            }
        }
    }

    pub fn preempt_background(&mut self, targets: &BTreeMap<OwnerId, f64>) {
        if self.active().any(|(_, active)| active.preempted) {
            return;
        }
        if let Some((index, active)) =
            self.slots
                .iter_mut()
                .enumerate()
                .rev()
                .find_map(|(index, slot)| {
                    let active = slot.active.as_mut()?;
                    (!active.preempted && !targets.contains_key(&active.job.request.owner))
                        .then_some((index, active))
                })
        {
            active.preempted = true;
            cancel_cpu_job(index);
        }
    }

    pub fn poll(&mut self) -> Result<Option<(usize, JobEvent)>, String> {
        let Some(packet) = poll_cpu_packet() else {
            return Ok(None);
        };
        let index = packet
            .get(0)
            .as_f64()
            .ok_or(tr!("CPU Worker 消息缺少槽位"))? as usize;
        let active = self.slots[index]
            .active
            .as_ref()
            .expect("scheduled CPU job");
        let done = packet
            .get(1)
            .as_bool()
            .ok_or(tr!("CPU Worker 消息缺少完成标记"))?;
        let error = packet.get(2).as_string();
        let payload = packet.get(3).dyn_into::<Array>();
        let event = if done {
            let result = match error {
                Some(error) => Err(error),
                None => {
                    let payload = payload.map_err(|_| tr!("CPU Worker 消息缺少完成数据"))?;
                    let serial = active.job.serial();
                    self.decode_completed(index, serial, &payload)
                }
            };
            JobEvent::Done(Box::new(result))
        } else {
            let payload = payload.map_err(|_| tr!("CPU Worker 消息缺少进度数据"))?;
            let update = decode(&payload)?;
            let audio = match update {
                Update::Pcm { seconds } => Some(
                    RenderedAudio::new(
                        SAMPLE_RATE,
                        seconds,
                        ChannelLayout::Mono,
                        pcm(&payload, 1)?,
                    )
                    .map_err(|error| error.to_string())?,
                ),
                _ => None,
            };
            JobEvent::Progress(update, audio)
        };
        Ok(Some((index, event)))
    }

    fn decode_completed(
        &mut self,
        slot: usize,
        serial: u64,
        packet: &Array,
    ) -> Result<(Arc<RenderedClip>, StageTimings), String> {
        let result: Completed = decode(packet)?;
        for (name, reasons) in result.activity {
            let counts = self.activity.entry(name).or_default();
            for (reason, count) in reasons {
                *counts.entry(reason).or_default() += count;
            }
        }
        let components = PcmComponents {
            start_seconds: result.pcm_origin,
            primary: pcm(packet, 1)?.into(),
            secondary: pcm(packet, 2)?.into(),
        };
        let audio = Arc::new(
            components
                .publish(result.visible_begin, result.visible_end)
                .map_err(|error| error.to_string())?,
        );
        let started = Instant::now();
        let visuals = Arc::new(RenderVisuals {
            waveform: Waveform::new(&audio),
            pitch: result.pitch,
            phonemes: result.phonemes,
        });
        let mut timings = result.timings;
        timings.visuals_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok((
            Arc::new(RenderedClip {
                audio,
                visuals,
                state: RenderState::CpuWorker {
                    components,
                    slot,
                    serial,
                },
            }),
            timings,
        ))
    }
}

fn decode<T: serde::de::DeserializeOwned>(packet: &Array) -> Result<T, String> {
    let text = packet
        .get(0)
        .as_string()
        .ok_or(tr!("CPU Worker 消息缺少元数据"))?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn pcm(packet: &Array, index: u32) -> Result<Vec<f32>, String> {
    packet
        .get(index)
        .dyn_into::<Float32Array>()
        .map_err(|_: JsValue| tr!("CPU Worker 消息缺少 PCM 样本").into())
        .map(|samples| samples.to_vec())
}
