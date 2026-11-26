use super::{
    super::{
        cpu::{JobEvent, Pool, Update},
        voices::voice_path,
    },
    *,
};
use crate::platform::web::cpu::wait_packets;
use crate::tr;

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    pool: &mut Pool,
    scheduler: &mut Scheduler<'_, '_, '_>,
    cache: &mut RenderCache,
    settings: &SynthesisSettings,
    library: &LibraryScan,
    group_names: &BTreeMap<OwnerId, String>,
    demand: fn() -> f64,
    cancelled: &AtomicBool,
    started: Instant,
    metrics: &mut PreviewMetrics,
    load_recording: &mut impl FnMut(
        u64,
        &mut Scheduler<'_, '_, '_>,
        &mut PreviewMetrics,
    ) -> Result<(), String>,
    emit: &impl Fn(Progress),
) -> Result<BTreeMap<OwnerId, StageTimings>, String> {
    let mut times = BTreeMap::new();
    loop {
        emit(Progress::Checkpoint);
        if cancelled.load(Ordering::Acquire) {
            return Err("CANCELLED: Render cancelled".into());
        }
        let demand = block_at(demand());
        let more = scheduler.publish(demand, started, metrics, &mut |event| emit(event))?;
        let mut targets = BTreeMap::new();
        for (source, target) in scheduler.demand_sources(demand) {
            if let (PreviewSource::Vocal(id), Some(target)) = (source, target) {
                targets.entry(id).or_insert(target);
            }
        }
        pool.prioritize(&targets);
        let running: BTreeMap<_, _> = pool
            .active()
            .map(|(_, active)| (active.job.request.owner, active.preempted))
            .collect();
        if !pool.available() && targets.keys().any(|id| running.get(id) != Some(&false)) {
            pool.preempt_background(&targets);
        }

        let next = if pool.available() {
            scheduler.next(demand, |owner| running.contains_key(&owner))
        } else {
            scheduler
                .demand_sources(demand)
                .find(|(source, _)| matches!(source, PreviewSource::Recording(_)))
        };
        match next {
            Some((PreviewSource::Vocal(id), target)) => {
                let job = cache.take_owner(id).ok_or(tr!("无法调度待渲染的音符组"))?;
                let path = voice_path(library, scheduler.key(id));
                match path {
                    Ok(path) => {
                        emit(Progress::Phase(tr!("准备声库与模型")));
                        pool.start(job, path, settings, true, target)?;
                    }
                    Err(error) => {
                        metrics.error.get_or_insert_with(|| {
                            format!("SYNTHESIS_FAILED: {}: {error}", group_names[&id])
                        });
                        scheduler.complete(cache, id, &job, Err(error))?;
                    }
                }
                emit(Progress::Counts(cache.counts()));
                continue;
            }
            Some((PreviewSource::Recording(id), _)) => {
                load_recording(id, scheduler, metrics)?;
                continue;
            }
            None => {}
        }
        let Some((slot, event)) = pool.poll()? else {
            if more {
                continue;
            }
            if running.is_empty() {
                break;
            }
            wait_packets()?;
            continue;
        };
        let active = pool
            .active()
            .find_map(|(index, active)| (index == slot).then_some(active))
            .expect("scheduled preview owner");
        let id = active.job.request.owner;
        match event {
            JobEvent::Progress(update, audio) if !active.preempted => match update {
                Update::Model {
                    voice_load_ms,
                    model_prepare_ms,
                } => {
                    metrics.voice_load_ms += voice_load_ms.unwrap_or_default();
                    metrics.model_prepare_ms += model_prepare_ms;
                }
                Update::Stage(stage) => emit(Progress::Stage {
                    group: group_names[&id].clone(),
                    stage,
                }),
                Update::Prepared { first, end } => scheduler.remote_prepared(id, first..end),
                Update::Pcm { .. } => scheduler.vocal(id, audio.expect("progress PCM"))?,
            },
            JobEvent::Progress(..) => {}
            JobEvent::Done(result) => {
                let active = pool.release(slot);
                if cancelled.load(Ordering::Acquire) {
                    return Err("CANCELLED: Render cancelled".into());
                }
                if active.preempted && result.is_err() {
                    cache.requeue_owner(&active.job);
                } else {
                    if active.preempted
                        && let Ok((clip, _)) = result.as_ref()
                    {
                        scheduler.remote_completed_pcm(id, &clip.audio)?;
                    }
                    if let Some(timing) = scheduler.complete(cache, id, &active.job, *result)? {
                        add_stages(&mut metrics.stages, &timing);
                        times.insert(id, timing);
                    }
                    if let Some(error) = cache.failure(id) {
                        metrics.error.get_or_insert_with(|| {
                            format!("SYNTHESIS_FAILED: {}: {error}", group_names[&id])
                        });
                    }
                }
                emit(Progress::Counts(cache.counts()));
            }
        }
    }
    Ok(times)
}
