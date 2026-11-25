use super::{Runtime, VoiceState};
use crate::{
    model::Project,
    platform::web::{
        audio,
        client::{Pending, submit},
        protocol::{Event, Preview, Request, Visuals},
    },
    synthesis::{
        offline::PreviewMetrics,
        plan::OwnerId,
        render_cache::CacheStats,
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
    requested_at: Option<Instant>,
    task: Option<Task>,
    pub(super) end_to_first_block_ms: Option<f64>,
    pub(super) metrics: Option<PreviewMetrics>,
}

struct Task {
    pending: Pending<Preview>,
    current: Rc<Cell<bool>>,
    progress: Rc<RefCell<Option<String>>>,
    counts: Rc<Cell<Option<CacheStats>>>,
    first_block_ms: Rc<Cell<Option<f64>>>,
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
        if self.preview.revision == Some(revision) && !self.preview.dirty {
            return;
        }
        if self.preview.revision != Some(revision) {
            self.preview.project = Some(project.clone());
        }
        self.preview.revision = Some(revision);
        self.preview.dirty = false;
        let mut required = BTreeSet::new();
        let mut identities = BTreeMap::new();
        let mut failures = Vec::new();
        for track in &project.tracks {
            for reference in track.references() {
                let owner = OwnerId(reference.id);
                match self.resolve_voice(track, reference) {
                    Ok(Some(key)) => {
                        identities.insert(owner, format!("{}:{}", key.revision, key.database_id));
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
        self.planner
            .update(project, revision, |owner| identities.get(&owner).cloned());
        self.visuals
            .retain(|owner, _| self.planner.plan().requests.contains_key(owner));
        self.phonemes
            .retain(|owner, _| self.planner.plan().requests.contains_key(owner));
        self.failures = failures;
        self.refresh_recording_error();
        self.invalidate_preview();
        let now = Instant::now();
        self.preview.requested_at = Some(now);
        let delay = if self.transport.playing { 24 } else { 90 };
        self.preview.ready_at = Some(now + Duration::from_millis(delay));
        self.publish_metronome();
    }

    pub(super) fn invalidate_preview(&mut self) {
        if let Some(task) = self.preview.task.take() {
            task.current.set(false);
            task.pending.cancel();
        }
        self.preview.duration = None;
        self.preview.ready_at = None;
        self.preview.requested_at = None;
        self.preview.metrics = None;
        self.preview.end_to_first_block_ms = None;
        self.stats = Default::default();
        self.error = None;
        self.progress = None;
        audio::audio_reset();
    }

    pub(super) fn poll_preview(&mut self) {
        if self.preview.audio_generation != self.instrumentals.generation {
            self.preview.audio_generation = self.instrumentals.generation;
            self.refresh_recording_error();
        }
        if let Some(task) = self.preview.task.as_ref() {
            if let Some(counts) = task.counts.take() {
                self.stats = counts;
            }
            if let Some(first) = task.first_block_ms.take() {
                self.preview.end_to_first_block_ms = Some(first);
            }
            if let Some(progress) = task.progress.borrow_mut().take() {
                self.progress = Some(progress);
            }
            let visuals = std::mem::take(&mut *task.visuals.borrow_mut());
            let result = task.pending.poll();
            for (visual, peaks) in visuals {
                self.publish_visuals(visual, peaks);
            }
            if let Some(result) = result {
                self.preview.task = None;
                self.stats.running = 0;
                self.progress = None;
                match result {
                    Ok(result) => {
                        self.preview.duration = result.metrics.duration;
                        self.error = result.metrics.error.clone();
                        self.stats.current_render_bytes = result.cache_bytes;
                        self.diagnostics = Some(result.diagnostics);
                        audio::audio_end(result.metrics.duration.unwrap_or(0.0));
                        self.preview.metrics = Some(result.metrics);
                    }
                    Err(error) => {
                        self.stats.queued = 0;
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
        let counts = Rc::new(Cell::new(None));
        let first_block_ms = Rc::new(Cell::new(None));
        let visuals = Rc::new(RefCell::new(Vec::new()));
        let (live, messages, patches) = (current.clone(), progress.clone(), visuals.clone());
        let (groups, first_block) = (counts.clone(), first_block_ms.clone());
        let requested_at = self
            .preview
            .requested_at
            .take()
            .unwrap_or_else(Instant::now);
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
                    (Event::Counts(counts), _) => {
                        groups.set(Some(counts));
                        repaint.wake();
                    }
                    (Event::Layout { frames, occupied }, _) => {
                        audio::layout(frames, &occupied);
                    }
                    (Event::Block { index, first }, pcm) => {
                        if first {
                            first_block.set(Some(requested_at.elapsed().as_secs_f64() * 1000.0));
                            repaint.wake();
                        }
                        audio::audio_block(index, pcm);
                    }
                    (Event::Visuals(visual), Some(peaks)) => {
                        patches.borrow_mut().push((*visual, peaks));
                        repaint.wake();
                    }
                    _ => {}
                }
            },
        );
        self.preview.task = Some(Task {
            pending,
            current,
            progress,
            counts,
            first_block_ms,
            visuals,
        });
        self.instrumentals.reconcile(project, &self.repaint);
        let runtime = &*self;
        let required: BTreeSet<_> = project
            .tracks
            .iter()
            .flat_map(|track| {
                track.references().filter_map(move |reference| {
                    runtime.resolve_voice(track, reference).ok().flatten()
                })
            })
            .collect();
        for key in required {
            self.inventory_request(key);
        }
    }

    pub(super) fn prioritize_preview(&mut self, seconds: f64, immediate: bool) {
        if let Some(task) = &self.preview.task {
            task.pending.prioritize(seconds);
        } else if immediate && self.preview.ready_at.is_some() {
            self.preview.ready_at = Some(Instant::now());
            self.repaint.wake();
        }
    }

    pub(super) fn refresh_voice_failures(&mut self) {
        if let Some(project) = &self.preview.project {
            self.failures = project
                .tracks
                .iter()
                .flat_map(|track| track.references().map(move |reference| (track, reference)))
                .filter_map(|(track, reference)| {
                    let error = match self.resolve_voice(track, reference) {
                        Ok(Some(key)) => self.voice_error(&key).map(str::to_owned),
                        Ok(None) => None,
                        Err(error) => Some(error),
                    };
                    error.map(|error| (OwnerId(reference.id), error))
                })
                .collect();
        }
    }

    fn refresh_recording_error(&mut self) {
        let previous = self.recording_error.take();
        self.recording_error = self
            .preview
            .project
            .as_ref()
            .into_iter()
            .flat_map(|project| project.tracks.iter().filter_map(|track| track.audio_clip()))
            .find_map(|clip| self.instrumentals.error(&clip.path).map(str::to_owned));
        if previous.is_some()
            && self.recording_error.is_none()
            && self
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("AUDIO_RESOURCE:"))
        {
            self.preview.resync();
        }
    }

    pub fn preview_timing(&self) -> (Option<f64>, Option<&PreviewMetrics>) {
        (
            self.preview.end_to_first_block_ms,
            self.preview.metrics.as_ref(),
        )
    }

    fn publish_visuals(&mut self, mut visual: Visuals, peaks: Float32Array) {
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
        let note_ids: BTreeMap<_, _> = visual.note_ids.into_iter().collect();
        let phones = Arc::make_mut(&mut visual.phones.phones);
        for phone in phones {
            if let Some(id) = note_ids
                .get(&phone.source.note_id)
                .and_then(|uuid| ids.get(uuid))
            {
                phone.source.note_id = *id;
            }
        }
        let Some(waveform) =
            Waveform::from_peaks(visual.origin, 48_000, visual.frames, &peaks.to_vec())
        else {
            return;
        };
        let snapshot = VisualSnapshot {
            input: request.input.clone(),
            data: Arc::new(RenderVisuals {
                pitch: visual.pitch,
                phonemes: visual.phones.clone(),
                waveform,
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
        self.export_metrics = None;
        self.export = Some(submit(
            Request::Export { project },
            self.repaint.clone(),
            |_, _| {},
        ));
    }
}
