use super::*;
use crate::synthesis::{audio::AudioLayer, plan::OwnerRequest};
use std::ops::Range;

const DEMAND_BATCH: usize = 64;
const BACKGROUND_BATCH: usize = 128;
const PARTIAL_BUDGET: usize = 24 << 20;

pub(in super::super) struct Partial {
    pub(in super::super) request: Arc<OwnerRequest>,
    pub(in super::super) prepared: progressive::PreparedRender,
}

struct Active<'network, 'tree> {
    job: RenderJob,
    prepared: progressive::PreparedRender,
    cursor: Option<progressive::Cursor<'network, 'tree>>,
}

pub(super) enum Update {
    Counts(crate::synthesis::render_cache::CacheStats),
    Stage(RenderStage),
}

pub(super) struct Scheduler<'network, 'tree, 'plan> {
    plan: &'plan RenderPlan,
    keys: BTreeMap<OwnerId, &'plan VoiceSelection>,
    aliases: BTreeMap<OwnerId, OwnerId>,
    members: BTreeMap<OwnerId, Vec<OwnerId>>,
    pending: BTreeSet<OwnerId>,
    background: Vec<(usize, PreviewSource)>,
    background_index: usize,
    recordings: BTreeSet<u64>,
    active: BTreeMap<OwnerId, Active<'network, 'tree>>,
    mixer: PreviewMixer,
}

impl<'network, 'tree: 'network, 'plan> Scheduler<'network, 'tree, 'plan> {
    pub(super) fn new(
        plan: &'plan RenderPlan,
        routes: &'plan BTreeMap<VoiceSelection, Vec<OwnerId>>,
        mixer: PreviewMixer,
    ) -> Self {
        let keys = routes
            .iter()
            .flat_map(|(key, owners)| owners.iter().map(move |&id| (id, key)))
            .collect();
        let mut groups = BTreeMap::<&str, Vec<OwnerId>>::new();
        let mut aliases = BTreeMap::new();
        let mut members = BTreeMap::<OwnerId, Vec<OwnerId>>::new();
        for (&id, request) in &plan.requests {
            let group = groups.entry(&request.input.group_id).or_default();
            let primary = group
                .iter()
                .copied()
                .find(|other| plan.requests[other].input == request.input)
                .unwrap_or_else(|| {
                    group.push(id);
                    id
                });
            aliases.insert(id, primary);
            members.entry(primary).or_default().push(id);
        }
        let mut background = mixer
            .sources()
            .iter()
            .copied()
            .filter(|(_, source)| match source {
                PreviewSource::Vocal(id) => aliases[id] == *id,
                PreviewSource::Recording(_) => true,
            })
            .collect::<Vec<_>>();
        background.sort_unstable();
        let recordings = background
            .iter()
            .filter_map(|(_, source)| match source {
                PreviewSource::Recording(id) => Some(*id),
                PreviewSource::Vocal(_) => None,
            })
            .collect();
        Self {
            plan,
            keys,
            aliases,
            pending: members.keys().copied().collect(),
            members,
            recordings,
            background,
            background_index: 0,
            active: BTreeMap::new(),
            mixer,
        }
    }

    pub(super) fn seed(
        &mut self,
        cache: &RenderCache,
        partial: &mut BTreeMap<OwnerId, Partial>,
    ) -> Result<(), String> {
        partial.retain(|id, work| {
            self.plan.requests.get(id).is_some_and(|current| {
                current.input == work.request.input && current.coverage == work.request.coverage
            }) && cache.published(*id).is_none()
        });
        for &id in self.plan.requests.keys() {
            if let Some(audio) = cache.published(id) {
                self.mixer.cached(id, audio)?;
                self.pending.remove(&self.aliases[&id]);
            }
        }
        Ok(())
    }

    pub(super) fn next(&mut self, demand: usize) -> Option<(PreviewSource, Option<f64>)> {
        let first = self.mixer.first_at_or_after(demand).unwrap_or(demand);
        for block in first..first.saturating_add(4) {
            for (source, target) in self.mixer.missing(block) {
                match source {
                    PreviewSource::Vocal(id) => {
                        let id = self.aliases[&id];
                        if self.pending.contains(&id) {
                            return Some((PreviewSource::Vocal(id), Some(target)));
                        }
                    }
                    PreviewSource::Recording(id) if self.recordings.contains(&id) => {
                        return Some((source, None));
                    }
                    PreviewSource::Recording(_) => {}
                }
            }
        }
        while let Some(&(_, source)) = self.background.get(self.background_index) {
            let pending = match source {
                PreviewSource::Vocal(id) => self.pending.contains(&id),
                PreviewSource::Recording(id) => self.recordings.contains(&id),
            };
            if pending {
                return Some((source, None));
            }
            self.background_index += 1;
        }
        None
    }

    pub(super) fn recording(&mut self, id: u64, layer: Option<AudioLayer>, range: Range<usize>) {
        self.recordings.remove(&id);
        self.mixer.recording(id, layer, range);
    }

    pub(super) fn recording_failed(&mut self, id: u64) {
        self.recordings.remove(&id);
    }

    pub(super) fn duration(&self) -> Option<f64> {
        self.mixer.duration()
    }

    pub(super) fn publish(
        &mut self,
        demand: usize,
        started: Instant,
        metrics: &mut PreviewMetrics,
        progress: &mut impl FnMut(Progress),
    ) -> Result<bool, String> {
        let mut started_mix = Instant::now();
        let blocks = self.mixer.take_changed(demand, 8);
        for index in blocks {
            let samples = self.mixer.mix(index);
            if samples
                .as_ref()
                .is_some_and(|samples| samples.iter().any(|sample| !sample.is_finite()))
            {
                return Err("预览混音产生了非有限的 PCM 样本".into());
            }
            let is_first = metrics.first_block_ms.is_none() && index >= demand && samples.is_some();
            if is_first {
                metrics.first_block_ms = Some(ms(started));
            }
            metrics.mix_ms += ms(started_mix);
            progress(Progress::Block {
                index,
                samples,
                first: is_first,
            });
            started_mix = Instant::now();
        }
        metrics.mix_ms += ms(started_mix);
        Ok(self.mixer.has_changed())
    }

    pub(super) fn advance(
        &mut self,
        id: OwnerId,
        target: Option<f64>,
        cache: &mut RenderCache,
        partial: &mut BTreeMap<OwnerId, Partial>,
        get_program: impl FnOnce(
            &VoiceSelection,
        ) -> Result<
            (
                &'tree VoiceProgram<'tree>,
                &'network progressive::Network<'tree>,
            ),
            String,
        >,
        mut update: impl FnMut(Update),
    ) -> Result<Option<StageTimings>, String> {
        let key = self.keys[&id];
        if !self.active.contains_key(&id) {
            let job = cache.take_owner(id).ok_or("无法调度待渲染的音符组")?;
            update(Update::Counts(cache.counts()));
            let prepared = match partial.remove(&id) {
                Some(work) => Ok(work.prepared),
                None => get_program(key).and_then(|(program, _)| {
                    program
                        .prepare(&job, |stage| {
                            update(Update::Stage(stage));
                            execution::checkpoint()
                        })
                        .map_err(|error| error.to_string())
                }),
            };
            match prepared {
                Ok(prepared) => {
                    let visible = prepared.visible_samples();
                    for &member in &self.members[&id] {
                        self.mixer.prepared(member, visible.clone());
                    }
                    for audio in prepared.ready_audio().map_err(|error| error.to_string())? {
                        self.vocal(id, audio)?;
                    }
                    self.active.insert(
                        id,
                        Active {
                            job,
                            prepared,
                            cursor: None,
                        },
                    );
                    // Publish unchanged PCM and proven silence before running a new cursor.
                    return Ok(None);
                }
                Err(error) => {
                    self.complete(cache, id, &job, Err(error))?;
                    update(Update::Counts(cache.counts()));
                    return Ok(None);
                }
            }
        }
        let active = self.active.get_mut(&id).expect("running owner");
        if !active.prepared.is_complete() {
            let mut stage = |stage| {
                update(Update::Stage(stage));
                execution::checkpoint()
            };
            let advance = || -> Result<_, String> {
                let (program, network) = get_program(key)?;
                let start = target.unwrap_or_else(|| self.mixer.next_vocal(id));
                if active.cursor.as_ref().is_none_or(|cursor| {
                    cursor.is_finished() || (target.is_some() && !cursor.covers_demand(start))
                }) {
                    stage(RenderStage::Timbre).map_err(|error| error.to_string())?;
                    active.cursor = active
                        .prepared
                        .start(network, &program.voice, start)
                        .map_err(|error| error.to_string())?;
                }
                let cursor = active.cursor.as_mut().ok_or("找不到待渲染音频的推理位置")?;
                let budget = if target.is_some() {
                    DEMAND_BATCH
                } else {
                    BACKGROUND_BATCH
                };
                active
                    .prepared
                    .advance(cursor, budget, &mut stage)
                    .map_err(|error| error.to_string())
            };
            match advance() {
                Ok(chunks) => {
                    for audio in chunks {
                        self.vocal(id, audio)?;
                    }
                }
                Err(error) => {
                    if execution::checkpoint().is_err() {
                        return Err(error);
                    }
                    let active = self.active.remove(&id).expect("running owner");
                    self.complete(cache, id, &active.job, Err(error))?;
                    update(Update::Counts(cache.counts()));
                    return Ok(None);
                }
            }
        }
        if self.active[&id].prepared.is_complete() {
            let active = self.active.remove(&id).expect("finished owner");
            let result = active.prepared.finish().map_err(|error| error.to_string());
            let timing = self.complete(cache, id, &active.job, result)?;
            update(Update::Counts(cache.counts()));
            return Ok(timing);
        }
        Ok(None)
    }

    fn vocal(&mut self, id: OwnerId, audio: RenderedAudio) -> Result<(), String> {
        let audio = Arc::new(audio);
        for &member in &self.members[&id] {
            self.mixer.vocal(member, Arc::clone(&audio))?;
        }
        Ok(())
    }

    fn complete(
        &mut self,
        cache: &mut RenderCache,
        id: OwnerId,
        job: &RenderJob,
        result: Result<RenderResult, String>,
    ) -> Result<Option<StageTimings>, String> {
        self.pending.remove(&id);
        let mut timing = None;
        let result = result.map(|result| {
            timing = Some(result.timings);
            Arc::new(RenderedClip {
                audio: Arc::new(result.audio),
                visuals: Arc::new(result.visuals),
                state: Arc::new(result.state),
            })
        });
        match cache.finish(job, result) {
            Completion::Published => {
                cache.reuse_queued();
                let audio = cache.published(id).expect("completed audio");
                for &member in &self.members[&id] {
                    self.mixer.completed(member, Arc::clone(&audio))?;
                }
            }
            Completion::Failed(error) => {
                for &member in &self.members[&id] {
                    if let Some(job) = cache.take_owner(member) {
                        cache.finish(&job, Err(error.clone()));
                    }
                }
            }
            Completion::Discarded => return Err("合成任务已失效".into()),
        }
        Ok(timing)
    }

    pub(super) fn save_partials(&mut self, partial: &mut BTreeMap<OwnerId, Partial>) {
        for (id, active) in std::mem::take(&mut self.active) {
            partial.insert(
                id,
                Partial {
                    request: active.job.request,
                    prepared: active.prepared,
                },
            );
        }
        let mut retained = 0usize;
        partial.retain(|_, work| {
            let samples = work.prepared.model_samples();
            let size =
                ((samples.end - samples.start) as usize).saturating_mul(2 * size_of::<f32>());
            if retained.saturating_add(size) <= PARTIAL_BUDGET {
                retained += size;
                true
            } else {
                false
            }
        });
    }
}
