use super::{Command, visuals::SessionVisuals};
use crate::synthesis::{
    engine::progressive::{Cursor, PreparedRender},
    plan::{OwnerId, OwnerInput, RenderPlan},
    render_cache::{CacheStats, RenderCache, RenderJob},
    runtime::mixer::LiveMixer,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, mpsc::Receiver},
};

pub(super) struct Session<'network, 'tree> {
    pub job: RenderJob,
    pub prepared: PreparedRender,
    pub cursor: Option<Cursor<'network, 'tree>>,
    pub seek_generation: u64,
    pub visuals: SessionVisuals,
}

impl<'network, 'tree> Session<'network, 'tree> {
    pub fn new(
        job: RenderJob,
        prepared: PreparedRender,
        seek_generation: u64,
    ) -> crate::synthesis::voice::Result<Self> {
        let visuals = SessionVisuals::new(&prepared, Arc::clone(&job.request.input))?;
        Ok(Self {
            job,
            prepared,
            cursor: None,
            seek_generation,
            visuals,
        })
    }
}

pub(super) struct WorkState<'network, 'tree> {
    pub cache: RenderCache,
    pub plan: RenderPlan,
    pub mirrors: BTreeMap<OwnerId, Vec<OwnerId>>,
    pub revision: u64,
    pub sessions: BTreeMap<OwnerId, Session<'network, 'tree>>,
    pub live: Option<LiveMixer>,
    pub demand_seconds: f64,
    pub playing: bool,
    pub seek_generation: u64,
    pub reset_generation: u64,
    pub(super) solo_tracks: BTreeSet<u64>,
    pub(super) published_revision: u64,
    pub(super) rebuild_source: bool,
    pub(super) force_publication: bool,
    pub(super) last_stats: Option<CacheStats>,
    pub(super) last_status_revision: Option<u64>,
}

impl<'network, 'tree> WorkState<'network, 'tree> {
    pub fn new() -> Self {
        Self {
            cache: RenderCache::default(),
            plan: RenderPlan::default(),
            mirrors: BTreeMap::new(),
            revision: 0,
            sessions: BTreeMap::new(),
            live: None,
            demand_seconds: 0.0,
            playing: false,
            seek_generation: 0,
            reset_generation: 0,
            solo_tracks: BTreeSet::new(),
            published_revision: 0,
            rebuild_source: false,
            force_publication: false,
            last_stats: None,
            last_status_revision: None,
        }
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Plan {
                plan,
                revision,
                solo_tracks,
            } => {
                self.revision = revision;
                self.solo_tracks = solo_tracks;
                self.cache.reconcile(&plan);
                self.sessions
                    .retain(|_, session| !session.job.is_cancelled());
                self.plan = plan;
                self.rebuild_mirrors();
                self.rebuild_source = true;
                self.force_publication = true;
            }
            Command::Demand {
                seconds,
                playing,
                seek,
            } => {
                if seconds.is_finite() {
                    self.demand_seconds = seconds.max(0.0);
                }
                self.playing = playing;
                if seek {
                    self.seek_generation = self.seek_generation.wrapping_add(1);
                }
            }
            Command::Reset => {
                self.reset_generation = self.reset_generation.wrapping_add(1);
                self.cache.reset();
                self.sessions.clear();
                self.plan = RenderPlan::default();
                self.mirrors.clear();
                self.solo_tracks.clear();
                self.rebuild_source = true;
                self.force_publication = true;
            }
            Command::Retry => {
                self.force_publication = true;
                self.last_status_revision = None;
                for owner in self.plan.requests.keys() {
                    self.cache.retry_failed(*owner);
                }
            }
        }
    }

    pub fn drain(&mut self, commands: &Receiver<Command>) {
        while let Ok(command) = commands.try_recv() {
            self.apply(command);
        }
    }

    /// Identical queued owners share one render job; they must also share its
    /// ready intervals, otherwise overlapping linked instances would hold
    /// playback until the whole shared owner completes.
    pub(super) fn mirrors(&self, owner: OwnerId) -> &[OwnerId] {
        self.mirrors.get(&owner).map_or(&[][..], Vec::as_slice)
    }

    fn rebuild_mirrors(&mut self) {
        self.mirrors.clear();
        let mut groups: Vec<(&OwnerInput, Vec<OwnerId>)> = Vec::new();
        for (&id, request) in &self.plan.requests {
            if let Some((_, group)) = groups
                .iter_mut()
                .find(|(input, _)| **input == *request.input)
            {
                group.push(id);
            } else {
                groups.push((&request.input, vec![id]));
            }
        }
        for (_, group) in groups {
            for &id in &group {
                self.mirrors.insert(id, group.clone());
            }
        }
    }
}
