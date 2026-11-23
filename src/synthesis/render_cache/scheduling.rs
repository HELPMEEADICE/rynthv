use super::{OwnerId, RenderCache, RenderJob, State};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

impl RenderCache {
    /// Share current ready results after reconciling the complete owner set,
    /// independently of scheduling priority. Pending owners' old baselines are
    /// reserved for their own incremental render, never for other owners.
    pub fn reuse_queued(&mut self) {
        if !std::mem::take(&mut self.reuse_pending) {
            return;
        }
        let mut ready = HashMap::<&str, Vec<_>>::new();
        for owner in self.owners.values() {
            if let Some(completed) = owner.ready() {
                ready
                    .entry(&completed.input.group_id)
                    .or_default()
                    .push(completed);
            }
        }
        if ready.is_empty() {
            return;
        }
        let reused: Vec<_> = self
            .owners
            .iter()
            .filter(|(_, owner)| matches!(owner.state, State::Queued))
            .filter_map(|(&id, owner)| {
                let input = &owner.request.input;
                let completed = ready
                    .get(input.group_id.as_str())?
                    .iter()
                    .find(|completed| completed.input.as_ref() == input.as_ref())?;
                Some((id, Arc::clone(&completed.clip)))
            })
            .collect();
        for (id, rendered) in reused {
            self.owners
                .get_mut(&id)
                .expect("queued owner")
                .complete(rendered);
            self.advance_publication();
        }
    }

    /// Only independently dispatchable owners: equivalent work already running
    /// remains queued until that owner publishes its complete cache baseline.
    pub fn queued_owners(&self) -> Vec<OwnerId> {
        let running: Vec<_> = self
            .owners
            .values()
            .filter_map(|owner| {
                matches!(owner.state, State::Running(_)).then_some(&*owner.request.input)
            })
            .collect();
        self.owners
            .iter()
            .filter(|(_, owner)| {
                matches!(owner.state, State::Queued)
                    && !running.iter().any(|input| **input == *owner.request.input)
            })
            .map(|(&id, _)| id)
            .collect()
    }

    fn is_dispatchable(&self, id: OwnerId) -> bool {
        let Some(owner) = self.owners.get(&id) else {
            return false;
        };
        matches!(owner.state, State::Queued)
            && !self.owners.values().any(|other| {
                matches!(other.state, State::Running(_))
                    && *owner.request.input == *other.request.input
            })
    }

    /// The caller chooses demand priority; the job retains the same commit and
    /// cancellation identity throughout every bounded streaming batch.
    pub fn take_owner(&mut self, id: OwnerId) -> Option<RenderJob> {
        if !self.is_dispatchable(id) {
            return None;
        }
        self.dispatch_owner(id)
    }

    pub fn take_next(&mut self) -> Option<RenderJob> {
        self.reuse_queued();
        let id = self
            .queued_owners()
            .into_iter()
            .min_by_key(|id| (self.owners[id].request.coverage.sample_start.0, *id))?;
        self.dispatch_owner(id)
    }

    fn dispatch_owner(&mut self, id: OwnerId) -> Option<RenderJob> {
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .expect("render serial exhausted");
        let owner = self.owners.get_mut(&id)?;
        let job = RenderJob {
            request: Arc::clone(&owner.request),
            base: owner.committed.clone(),
            generation: self.generation,
            serial: self.next_serial,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        owner.state = State::Running(job.clone());
        Some(job)
    }
}
