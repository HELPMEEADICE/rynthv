#[cfg(target_arch = "wasm32")]
use super::super::audio::RenderedAudio;
use super::super::rendered::VisualSnapshot;
use super::{
    CacheStats, Completion, OwnerId, PublishedOwner, RenderCache, RenderJob, RenderedClip, State,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

impl RenderCache {
    /// Stale results never enter the cache or publication, even if a worker
    /// finished without observing cancellation. Failure requires explicit retry
    /// or a changed input; normal UI updates cannot create a busy retry loop.
    pub fn finish(
        &mut self,
        job: &RenderJob,
        result: Result<Arc<RenderedClip>, String>,
    ) -> Completion {
        let current = self.owners.get_mut(&job.request.owner).filter(|owner| {
            !job.is_cancelled()
                && job.generation == self.generation
                && owner.request.epoch == job.request.epoch
                && owner.request.coverage == job.request.coverage
                && *owner.request.input == *job.request.input
                && match (&owner.committed, &job.base) {
                    (Some(current), Some(base)) => Arc::ptr_eq(current, base),
                    (None, None) => true,
                    _ => false,
                }
                && matches!(&owner.state, State::Running(active) if active.serial == job.serial)
        });
        let Some(owner) = current else {
            self.discarded = self.discarded.saturating_add(1);
            return Completion::Discarded;
        };
        match result {
            Ok(rendered) => {
                owner.complete(rendered);
                self.reuse_pending = true;
                self.advance_publication();
                Completion::Published
            }
            Err(error) => {
                owner.state = State::Failed(error.clone());
                Completion::Failed(error)
            }
        }
    }

    pub fn retry_failed(&mut self, id: OwnerId) -> bool {
        if let Some(owner) = self.owners.get_mut(&id)
            && matches!(owner.state, State::Failed(_))
        {
            owner.state = State::Queued;
            self.reuse_pending = true;
            true
        } else {
            false
        }
    }

    pub fn failure(&self, id: OwnerId) -> Option<&str> {
        match &self.owners.get(&id)?.state {
            State::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// Ready unmuted owners with current placement/control metadata. Track
    /// mute/solo/pan/gain are still handled by the playback mixer.
    pub fn snapshot(&self) -> Vec<PublishedOwner> {
        self.owners
            .iter()
            .filter_map(|(&id, owner)| {
                let completed = owner.ready()?;
                (!owner.controls.reference_muted).then(|| PublishedOwner {
                    owner: id,
                    coverage: owner.request.coverage,
                    controls: owner.controls.clone(),
                    audio: Arc::clone(&completed.clip.audio),
                    components: completed.clip.state.components.clone(),
                    visuals: Arc::clone(&completed.clip.visuals),
                })
            })
            .collect()
    }

    /// Publish only completed data together with its original input geometry,
    /// independent of instance mute and mixer controls. Runtime retains the
    /// previous completed snapshot for owners omitted while they render.
    pub fn visual_snapshot(&self) -> BTreeMap<OwnerId, VisualSnapshot> {
        self.owners
            .iter()
            .filter_map(|(&id, owner)| {
                let completed = owner.ready()?;
                Some((
                    id,
                    VisualSnapshot {
                        data: Arc::clone(&completed.clip.visuals),
                        input: Arc::clone(&completed.input),
                    },
                ))
            })
            .collect()
    }

    pub fn publication_revision(&self) -> u64 {
        self.publication_revision
    }

    pub fn stats(&self) -> CacheStats {
        let mut stats = self.counts();
        let mut seen = BTreeSet::new();
        let completed = self
            .owners
            .values()
            .filter_map(|owner| owner.committed.as_ref());
        #[cfg(target_arch = "wasm32")]
        let completed = completed.chain(self.parked.values());
        for completed in completed {
            if seen.insert(Arc::as_ptr(&completed.clip)) {
                stats.current_render_bytes += completed.clip.memory_bytes();
            }
        }
        stats
    }

    pub(crate) fn counts(&self) -> CacheStats {
        let mut stats = CacheStats {
            discarded_completions: self.discarded,
            ..CacheStats::default()
        };
        for owner in self.owners.values() {
            match &owner.state {
                State::Queued => stats.queued += 1,
                State::Running(_) => stats.running += 1,
                State::Failed(_) => stats.failed += 1,
                State::Ready => stats.ready += 1,
            }
        }
        stats
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn retain_near(&mut self, budget: usize, seconds: f64) {
        let mut order = self
            .owners
            .iter()
            .filter_map(|(&id, owner)| {
                owner.committed.as_ref()?;
                let coverage = owner.request.coverage;
                let frame = (seconds * f64::from(coverage.sample_rate)) as i64;
                let distance = if frame < coverage.sample_start.0 {
                    coverage.sample_start.0.abs_diff(frame)
                } else {
                    frame.saturating_sub(coverage.sample_end.0).max(0) as u64
                };
                Some((distance, id))
            })
            .collect::<Vec<_>>();
        order.sort_unstable();
        let mut retained = BTreeSet::new();
        let mut bytes = 0usize;
        for (_, id) in order {
            let owner = self.owners.get_mut(&id).expect("retained owner");
            let clip = &owner.committed.as_ref().expect("committed owner").clip;
            if retained.contains(&Arc::as_ptr(clip)) {
                continue;
            }
            let size = clip.memory_bytes();
            if bytes.saturating_add(size) <= budget {
                retained.insert(Arc::as_ptr(clip));
                bytes += size;
            } else {
                owner.committed = None;
                if matches!(owner.state, State::Ready) {
                    owner.state = State::Queued;
                }
            }
        }
        self.parked.retain(|_, owner| {
            let clip = &owner.clip;
            if retained.contains(&Arc::as_ptr(clip)) {
                return true;
            }
            let size = clip.memory_bytes();
            if bytes.saturating_add(size) > budget {
                return false;
            }
            retained.insert(Arc::as_ptr(clip));
            bytes += size;
            true
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn discard_removed(&mut self, present: &BTreeSet<OwnerId>) {
        self.parked.retain(|id, _| present.contains(id));
        self.owners.retain(|id, owner| {
            let retain = present.contains(id);
            if !retain {
                owner.cancel();
            }
            retain
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn requeue_running(&mut self) {
        for owner in self.owners.values_mut() {
            if matches!(owner.state, State::Running(_)) {
                owner.cancel();
                owner.state = State::Queued;
                self.reuse_pending = true;
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn published(&self, id: OwnerId) -> Option<Arc<RenderedAudio>> {
        Some(Arc::clone(&self.owners.get(&id)?.ready()?.clip.audio))
    }
}
