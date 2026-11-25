use super::{Runtime, VoiceState};
use crate::{
    model::Project,
    platform::web::{
        audio,
        client::{Pending, submit},
        protocol::{Event, Preview, Request, Visuals},
    },
    synthesis::{
        audio::{ChannelLayout, RenderedAudio},
        plan::OwnerId,
        rendered::{PhonemeSnapshot, RenderVisuals, VisualSnapshot, Waveform},
    },
};
use js_sys::Float32Array;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use web_time::Instant;

#[derive(Default)]
pub(super) struct State {
    pub(super) project: Option<Project>,
    pub(super) duration: Option<f64>,
    revision: Option<u64>,
    audio_generation: u64,
    dirty: bool,
    ready_at: Option<Instant>,
    task: Option<Task>,
}

struct Task {
    pending: Pending<Preview>,
    current: Rc<Cell<bool>>,
    progress: Rc<RefCell<Option<String>>>,
    visuals: Rc<RefCell<Vec<(Visuals, Float32Array)>>>,
}

impl State {
    pub(super) fn resync(&mut self) {
        self.dirty = true;
    }
    pub(super) fn is_pending(&self) -> bool {
        self.dirty || self.ready_at.is_some() || self.task.is_some()
    }
}

impl Runtime {
    pub fn sync_project(&mut self, project: &Project, revision: u64) {
        if self.preview.revision == Some(revision)
            && self.preview.audio_generation == self.instrumentals.generation
            && !self.preview.dirty
        {
            return;
        }
        if self.preview.revision != Some(revision) {
            self.preview.project = Some(project.clone());
        }
        self.preview.revision = Some(revision);
        self.preview.dirty = false;
        self.instrumentals.reconcile(project, &self.repaint);
        self.preview.audio_generation = self.instrumentals.generation;
        let mut required = BTreeSet::new();
        let mut identities = BTreeMap::new();
        let mut failures = Vec::new();
        for track in &project.tracks {
            for reference in track.references() {
                let owner = OwnerId(reference.id);
                match self.resolve_voice(track, reference) {
                    Ok(Some(key)) => {
                        if let Some(inventory) = self.inventory(&key) {
                            identities.insert(owner, inventory.content_hash.clone());
                        }
                        if let Some(error) = self.voice_error(&key) {
                            failures.push((owner, error.into()));
                        }
                        required.insert(key);
                    }
                    Err(error) => failures.push((owner, error)),
                    Ok(None) => {}
                }
            }
        }
        self.voices.retain(|key, _| required.contains(key));
        self.voice_jobs.retain(|key, task| {
            if required.contains(key) {
                true
            } else {
                task.cancel();
                false
            }
        });
        for key in required {
            self.inventory_request(key);
        }
        self.planner
            .update(project, revision, |owner| identities.get(&owner).cloned());
        self.visuals
            .retain(|owner, _| self.planner.plan().requests.contains_key(owner));
        self.phonemes
            .retain(|owner, _| self.planner.plan().requests.contains_key(owner));
        self.failures = failures;
        self.recording_error = project
            .tracks
            .iter()
            .filter_map(|track| track.audio_clip())
            .find_map(|clip| self.instrumentals.error(&clip.path).map(str::to_owned));
        self.invalidate_preview();
        self.preview.ready_at = Some(Instant::now() + Duration::from_millis(180));
        self.stats.queued = 1;
        self.publish_metronome();
    }

    pub(super) fn invalidate_preview(&mut self) {
        if let Some(task) = self.preview.task.take() {
            task.current.set(false);
            task.pending.cancel();
        }
        self.preview.duration = None;
        self.preview.ready_at = None;
        self.stats = Default::default();
        self.error = None;
        self.progress = None;
        audio::audio_reset();
    }

    pub(super) fn poll_preview(&mut self) {
        if let Some(task) = self.preview.task.as_ref() {
            if let Some(progress) = task.progress.borrow_mut().take() {
                self.progress = Some(progress);
            }
            let visuals = std::mem::take(&mut *task.visuals.borrow_mut());
            let result = task.pending.poll();
            for (visual, pcm) in visuals {
                self.publish_visuals(visual, pcm);
            }
            if let Some(result) = result {
                self.preview.task = None;
                self.stats.running = 0;
                self.progress = None;
                match result {
                    Ok(result) => {
                        self.preview.duration = result.duration;
                        self.stats.ready = result.rendered + result.reused;
                        self.stats.current_render_bytes = result.cache_bytes;
                        self.diagnostics = result.diagnostics;
                        audio::audio_end(result.duration.unwrap_or(0.0));
                    }
                    Err(error) => {
                        self.stats.failed = 1;
                        self.error = Some(error);
                        audio::audio_end(0.0);
                    }
                }
            }
            return;
        }
        if self.preview.dirty
            || self.preview.ready_at.is_none_or(|at| at > Instant::now())
            || self.library.is_scanning()
            || !self.voice_jobs.is_empty()
            || self.is_exporting()
        {
            return;
        }
        self.preview.ready_at = None;
        self.stats.queued = 0;
        let Some(project) = self.preview.project.as_ref() else {
            return;
        };
        let serialized = match serde_json::to_string(project) {
            Ok(project) => project,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let current = Rc::new(Cell::new(true));
        let progress = Rc::new(RefCell::new(None));
        let visuals = Rc::new(RefCell::new(Vec::new()));
        let (live, messages, patches) = (current.clone(), progress.clone(), visuals.clone());
        let repaint = self.repaint.clone();
        let pending = submit(
            Request::Preview {
                project: serialized,
                position: self.transport.seconds,
            },
            self.repaint.clone(),
            move |event, pcm| {
                if !live.get() {
                    return;
                }
                match (event, pcm) {
                    (Event::Progress { stage }, _) => {
                        messages.replace(Some(stage));
                        repaint.wake();
                    }
                    (Event::Chunk { origin, .. }, Some(pcm)) => {
                        audio::audio_pcm(origin, pcm, false)
                    }
                    (Event::Mix { origin }, Some(pcm)) => audio::audio_pcm(origin, pcm, true),
                    (Event::Visuals(visual), Some(pcm)) => {
                        patches.borrow_mut().push((*visual, pcm));
                        repaint.wake();
                    }
                    _ => {}
                }
            },
        );
        self.stats.running = 1;
        self.preview.task = Some(Task {
            pending,
            current,
            progress,
            visuals,
        });
    }

    fn publish_visuals(&mut self, mut visual: Visuals, pcm: Float32Array) {
        let Some(project) = &self.preview.project else {
            return;
        };
        let Some(reference) = project
            .tracks
            .iter()
            .flat_map(|track| track.references())
            .find(|reference| reference.uuid == visual.group)
        else {
            return;
        };
        let Some(group) = project.group(&reference.group_id) else {
            return;
        };
        let owner = OwnerId(reference.id);
        let Some(request) = self.planner.plan().requests.get(&owner) else {
            return;
        };
        let ids: BTreeMap<_, _> = group
            .notes
            .iter()
            .map(|note| (&note.uuid, note.id))
            .collect();
        let phones = Arc::make_mut(&mut visual.phones.phones);
        for phone in phones {
            if let Some(id) = visual
                .note_ids
                .get(&phone.source.note_id)
                .and_then(|uuid| ids.get(uuid))
            {
                phone.source.note_id = *id;
            }
        }
        let Ok(audio) =
            RenderedAudio::new(48_000, visual.origin, ChannelLayout::Mono, pcm.to_vec())
        else {
            return;
        };
        let snapshot = VisualSnapshot {
            input: request.input.clone(),
            data: Arc::new(RenderVisuals {
                pitch: visual.pitch,
                phonemes: visual.phones.clone(),
                waveform: Waveform::new(&audio),
            }),
        };
        let timeline = self.visuals.entry(owner).or_default();
        timeline.begin(request.input.clone());
        timeline.complete(snapshot);
        self.phonemes.insert(
            owner,
            PhonemeSnapshot {
                input: request.input.clone(),
                data: visual.phones,
            },
        );
    }

    pub fn retry(&mut self) {
        self.voices
            .retain(|_, voice| !matches!(voice, VoiceState::Failed(_)));
        self.preview.resync();
    }

    pub fn export_ready(&self, revision: u64) -> bool {
        self.preview.revision == Some(revision)
            && !self.is_exporting()
            && !self.library.is_scanning()
            && self.recording_error.is_none()
            && self.preview.project.as_ref().is_some_and(|project| {
                project
                    .tracks
                    .iter()
                    .any(|track| track.audio_clip().is_some())
                    || !self.planner.plan().requests.is_empty()
            })
    }

    pub fn export_project(&mut self, revision: u64) {
        if !self.export_ready(revision) {
            return;
        }
        let Some(project) = self.preview.project.as_ref() else {
            return;
        };
        let project = match serde_json::to_string(project) {
            Ok(project) => project,
            Err(error) => {
                self.export_result = Some(Err(error.to_string()));
                return;
            }
        };
        self.export_result = None;
        self.export = Some(submit(
            Request::Export { project },
            self.repaint.clone(),
            |_, _| {},
        ));
    }
}
