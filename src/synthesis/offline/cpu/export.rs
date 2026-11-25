use super::{
    super::{voices::voice_path, *},
    JobEvent, Update,
};
use crate::{
    platform::web::cpu::wait_packets,
    synthesis::{plan::RenderPlan, render_cache::Completion},
};
use std::collections::VecDeque;

impl Offline {
    pub(in crate::synthesis::offline) fn render_cpu_owners(
        &mut self,
        plan: &RenderPlan,
        settings: &SynthesisSettings,
        routes: &BTreeMap<VoiceSelection, Vec<OwnerId>>,
        group_names: &BTreeMap<OwnerId, String>,
        cancelled: &AtomicBool,
        progress: &mut impl FnMut(Progress),
    ) -> Result<BTreeMap<OwnerId, StageTimings>, String> {
        let mut pending: VecDeque<_> = routes
            .iter()
            .flat_map(|(key, owners)| owners.iter().map(move |&owner| (key, owner)))
            .filter(|(_, owner)| plan.requests.contains_key(owner))
            .collect();
        let aliases = plan.owner_aliases(pending.iter().map(|&(_, owner)| owner));
        pending.retain(|&(_, owner)| aliases[&owner] == owner);
        let library = self.library.as_ref().expect("scanned library");
        let cache = &mut self.cache;
        let result = self.cpu_pool.run_batch(|pool| {
            let mut order = VecDeque::new();
            let mut completed = BTreeMap::new();
            let mut timings = BTreeMap::new();
            loop {
                progress(Progress::Checkpoint);
                if cancelled.load(Ordering::Acquire) {
                    return Err("CANCELLED: Render cancelled".into());
                }
                while pool.available() {
                    let Some(&(key, owner)) = pending.front() else {
                        break;
                    };
                    cache.reuse_queued();
                    if cache.published(owner).is_some() {
                        pending.pop_front();
                        continue;
                    }
                    let job = cache.take_owner(owner).ok_or("无法调度待导出的音符组")?;
                    let path = voice_path(library, key)?;
                    let slot = pool.start(job, path, settings, false, None)?;
                    order.push_back((slot, key, owner));
                    pending.pop_front();
                }
                if order.is_empty() {
                    return Ok(timings);
                }
                let Some((slot, event)) = pool.poll()? else {
                    wait_packets()?;
                    continue;
                };
                let &(_, key, owner) = order
                    .iter()
                    .find(|&&(index, _, _)| index == slot)
                    .expect("scheduled export owner");
                match event {
                    JobEvent::Progress(
                        Update::Model {
                            voice_load_ms: Some(elapsed_ms),
                            ..
                        },
                        _,
                    ) => {
                        progress(Progress::VoiceLoaded {
                            voice: key.clone(),
                            elapsed_ms,
                        });
                    }
                    JobEvent::Progress(Update::Stage(stage), _) => progress(Progress::Stage {
                        group: group_names[&owner].clone(),
                        stage,
                    }),
                    JobEvent::Progress(_, _) => {}
                    JobEvent::Done(result) => {
                        completed.insert(slot, result);
                        // Hold slots until earlier results commit so both the
                        // error order and buffered PCM are bounded.
                        while let Some(&(index, _, owner)) = order.front() {
                            let Some(result) = completed.remove(&index) else {
                                break;
                            };
                            order.pop_front();
                            let job = pool.release(index).job;
                            let result = result.map(|(clip, timing)| {
                                timings.insert(owner, timing);
                                clip
                            });
                            match cache.finish(&job, result) {
                                Completion::Published => cache.reuse_queued(),
                                Completion::Failed(error) => {
                                    return Err(format!(
                                        "SYNTHESIS_FAILED: {}: {error}",
                                        group_names[&owner]
                                    ));
                                }
                                Completion::Discarded => return Err("合成任务已失效".into()),
                            }
                        }
                    }
                }
            }
        });
        cache.requeue_running();
        result
    }
}
