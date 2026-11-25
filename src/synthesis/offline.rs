//! Project rendering without a window, a display surface, or a playback device.
#[cfg(target_arch = "wasm32")]
pub(crate) mod cpu;
#[cfg(target_arch = "wasm32")]
mod preview;
mod voices;

pub use super::mixer::VocalStem;
use super::{
    audio::RenderedAudio,
    engine::{RenderStage, StageTimings, progressive},
    inference::compute::{AdapterId, Backend, Diagnostics, Session},
    instrumental::AudioAsset,
    library::LibraryScan,
    mixer,
    plan::{OwnerId, Planner},
    render_cache::{PublishedOwner, RenderCache},
    settings::{SynthesisSettings, VoiceSelection},
    voice::VoiceInventory,
};
use crate::model::{Project, Track};
#[cfg(target_arch = "wasm32")]
pub(crate) use preview::PreviewMetrics;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use voices::VoiceResources;
use web_time::Instant;

#[derive(Default)]
pub struct Offline {
    library: Option<LibraryScan>,
    settings: Option<SynthesisSettings>,
    voices: BTreeMap<VoiceSelection, std::cell::OnceCell<VoiceResources>>,
    compute: Option<(Backend, Option<AdapterId>, Session)>,
    planner: Planner,
    cache: RenderCache,
    project: String,
    owner_ids: BTreeMap<String, OwnerId>,
    note_ids: BTreeMap<String, u64>,
    revision: u64,
    #[cfg(target_arch = "wasm32")]
    recordings: preview::RecordingCache,
    #[cfg(target_arch = "wasm32")]
    partial: BTreeMap<OwnerId, preview::Partial>,
    #[cfg(target_arch = "wasm32")]
    cpu_pool: cpu::Pool,
}

pub struct Options {
    pub tracks: BTreeSet<u64>,
    pub group: Option<u64>,
    pub range_seconds: Option<Range<f64>>,
    pub vocals_only: bool,
    pub backend: Backend,
    pub adapter: Option<AdapterId>,
    /// Soft budget for completed acoustic data retained between requests.
    pub cache_bytes: usize,
    #[cfg(target_arch = "wasm32")]
    pub preview: Option<fn() -> f64>,
}

pub enum Progress {
    VoiceLoaded {
        voice: VoiceSelection,
        elapsed_ms: f64,
    },
    Stage {
        group: String,
        stage: RenderStage,
    },
    #[cfg(target_arch = "wasm32")]
    Counts(super::render_cache::CacheStats),
    #[cfg(target_arch = "wasm32")]
    Layout {
        frames: usize,
        occupied: Vec<[usize; 2]>,
    },
    #[cfg(target_arch = "wasm32")]
    Block {
        index: usize,
        samples: Option<Vec<f32>>,
        first: bool,
    },
    #[cfg(target_arch = "wasm32")]
    Phase(&'static str),
    #[cfg(target_arch = "wasm32")]
    Checkpoint,
}

pub struct Report {
    pub diagnostics: Diagnostics,
    pub owners: Vec<OwnerReport>,
    pub rendered_owners: usize,
    pub reused_owners: usize,
    pub cache_bytes: usize,
    pub elapsed_ms: f64,
    #[cfg(target_arch = "wasm32")]
    pub preview: Option<PreviewMetrics>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) cpu_activity: cpu::Activity,
}

pub struct OwnerReport {
    pub group_ref_id: String,
    pub timings: Option<StageTimings>,
    pub owner: PublishedOwner,
    pub note_ids: BTreeMap<u64, String>,
}

pub struct Output {
    pub report: Report,
    solo: BTreeSet<u64>,
    recordings: Vec<super::audio::AudioLayer>,
    owners: Vec<PublishedOwner>,
    range: Option<Range<usize>>,
}

impl Output {
    pub fn mix(&self, track: Option<u64>) -> Result<Option<Arc<RenderedAudio>>, String> {
        self.mix_stem(track, VocalStem::Combined)
    }

    pub fn mix_stem(
        &self,
        track: Option<u64>,
        stem: VocalStem,
    ) -> Result<Option<Arc<RenderedAudio>>, String> {
        let recordings = if stem == VocalStem::Aspiration {
            &[][..]
        } else {
            &self.recordings
        };
        mixer::mix_with_recordings(
            &self.owners,
            &self.solo,
            recordings,
            track,
            stem,
            self.range.clone(),
        )
    }
}

impl Offline {
    pub(crate) fn prepare_compute(
        &mut self,
        backend: Backend,
        adapter: Option<&AdapterId>,
    ) -> &Session {
        if self.compute.as_ref().is_none_or(|(current, selected, _)| {
            *current != backend || selected.as_ref() != adapter
        }) {
            self.compute = Some((backend, adapter.cloned(), Session::new(backend, adapter)));
            self.cache.reset();
            #[cfg(target_arch = "wasm32")]
            self.partial.clear();
        }
        &self.compute.as_ref().expect("prepared compute session").2
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn recording(
        &mut self,
        path: &std::path::Path,
        cancelled: &AtomicBool,
    ) -> Result<Arc<AudioAsset>, String> {
        self.recordings.load(path, cancelled)
    }

    pub fn refresh(&mut self) {
        self.library = None;
        self.voices.clear();
        if let Some((_, _, session)) = &self.compute {
            session.prune_weights();
        }
        self.cache.reset();
        self.planner.reset();
        #[cfg(target_arch = "wasm32")]
        {
            self.partial.clear();
            self.cpu_pool.clear_resources();
        }
    }

    pub fn library(&mut self, settings: &SynthesisSettings) -> &LibraryScan {
        if self.settings.as_ref() != Some(settings) {
            self.refresh();
            self.settings = Some(settings.clone());
        }
        self.library
            .get_or_insert_with(|| super::library::scan_roots(settings))
    }

    pub fn inventory(
        &mut self,
        settings: &SynthesisSettings,
        key: &VoiceSelection,
    ) -> Result<&VoiceInventory, String> {
        self.library(settings);
        self.load_voice(key)
            .map(|(resource, _)| &resource.inventory)
    }

    pub fn render(
        &mut self,
        project: &Project,
        settings: &SynthesisSettings,
        options: &Options,
        cancelled: Arc<AtomicBool>,
        mut progress: impl FnMut(Progress),
    ) -> Result<Output, String> {
        let started = Instant::now();
        if cancelled.load(Ordering::Acquire) {
            return Err("CANCELLED: Render cancelled".into());
        }
        super::mixing_options::validate_export_bypass(&project.render_config)?;
        let solo = mixer::solo_tracks(project)?;
        let include = |track: &Track| {
            (options.tracks.is_empty() || options.tracks.contains(&track.id))
                && (solo.is_empty() || solo.contains(&track.id))
                && track.mixer.get("mute").and_then(serde_json::Value::as_bool) == Some(false)
        };
        let project_changed = self.project != project.uuid;
        if project_changed {
            self.cache.reset();
            self.planner.reset();
            #[cfg(target_arch = "wasm32")]
            self.partial.clear();
            self.owner_ids.clear();
            self.note_ids.clear();
            self.project.clone_from(&project.uuid);
        }
        let mut vocal = project.clone();
        vocal
            .tracks
            .retain(|track| !track.is_instrumental() && include(track));
        // Keep the project library and every referenced definition available to
        // planning, including a main definition on another track.
        vocal.library.extend(
            project
                .tracks
                .iter()
                .filter(|track| {
                    !vocal
                        .tracks
                        .iter()
                        .any(|vocal_track| track.id == vocal_track.id)
                })
                .map(|track| track.main_group.clone()),
        );
        let mut retained_ids = BTreeSet::new();
        for group in vocal
            .library
            .iter_mut()
            .chain(vocal.tracks.iter_mut().map(|track| &mut track.main_group))
        {
            for note in &mut group.notes {
                note.id = *self.note_ids.entry(note.uuid.clone()).or_insert(note.id);
                retained_ids.insert(note.uuid.clone());
            }
        }
        self.note_ids.retain(|uuid, _| retained_ids.contains(uuid));
        let mut group_names = BTreeMap::new();
        let mut selected_groups = BTreeSet::new();
        for track in &mut vocal.tracks {
            for reference in std::iter::once(&mut track.main_ref).chain(&mut track.groups) {
                let selected = options.group.is_none_or(|id| id == reference.id) && !reference.mute;
                let owner = *self
                    .owner_ids
                    .entry(reference.uuid.clone())
                    .or_insert(OwnerId(reference.id));
                reference.id = owner.0;
                group_names.insert(owner, reference.uuid.clone());
                if selected {
                    selected_groups.insert(owner);
                }
            }
        }
        let current_owners: BTreeSet<_> = project
            .tracks
            .iter()
            .flat_map(Track::references)
            .map(|reference| &reference.uuid)
            .collect();
        self.owner_ids
            .retain(|uuid, _| current_owners.contains(uuid));
        #[cfg(target_arch = "wasm32")]
        self.cache
            .discard_removed(&self.owner_ids.values().copied().collect());
        self.library(settings);
        let library = self.library.as_ref().expect("scanned library");
        let mut preview = Planner::default();
        let initial = preview.update(&vocal, self.revision, |_| None);
        let mut routes = BTreeMap::<VoiceSelection, Vec<OwnerId>>::new();
        let range = options.range_seconds.clone();
        for track in &vocal.tracks {
            for reference in track.references() {
                let owner = OwnerId(reference.id);
                let Some(request) = initial.requests.get(&owner).filter(|request| {
                    selected_groups.contains(&owner)
                        && range.as_ref().is_none_or(|range| {
                            let coverage = request.coverage;
                            f64::from(coverage.sample_rate) * range.start
                                < coverage.sample_end.0 as f64
                                && f64::from(coverage.sample_rate) * range.end
                                    > coverage.sample_start.0 as f64
                        })
                }) else {
                    continue;
                };
                let key = library
                    .catalog
                    .resolve(&track.main_ref.database, &reference.database)
                    .map_err(|error| {
                        format!(
                            "VOICE_NOT_FOUND: {} / {}: {error}",
                            track.name, reference.uuid
                        )
                    })?;
                #[cfg(target_arch = "wasm32")]
                if options.preview.is_some() && key.is_none() {
                    continue;
                }
                let key = key.ok_or_else(|| {
                    format!(
                        "VOICE_NOT_ASSIGNED: {} / {} has {} notes. Assign a voice first.",
                        track.name,
                        reference.uuid,
                        request.input.notes.len()
                    )
                })?;
                routes.entry(key).or_default().push(owner);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if project_changed {
            self.voices.retain(|key, _| routes.contains_key(key));
            if let Some((_, _, compute)) = &self.compute {
                compute.prune_weights();
            }
        }
        let accompaniment: Vec<_> = project
            .tracks
            .iter()
            .filter(|track| {
                include(track)
                    && options.group.is_none()
                    && !options.vocals_only
                    && track.audio_clip().is_some_and(|clip| {
                        range.as_ref().is_none_or(|range| {
                            clip.onset_seconds < range.end && clip.end_seconds() > range.start
                        })
                    })
            })
            .collect();
        #[cfg(target_arch = "wasm32")]
        let mut metrics = None;
        #[cfg(target_arch = "wasm32")]
        let previewing = options.preview.is_some();
        #[cfg(not(target_arch = "wasm32"))]
        let previewing = false;
        let (timings, recordings) = if previewing {
            #[cfg(target_arch = "wasm32")]
            {
                let rendered = self.render_preview(
                    &vocal,
                    settings,
                    options,
                    &routes,
                    &group_names,
                    &cancelled,
                    &solo,
                    &accompaniment,
                    &mut progress,
                )?;
                metrics = Some(rendered.1);
                (rendered.0, Vec::new())
            }
            #[cfg(not(target_arch = "wasm32"))]
            unreachable!();
        } else {
            let timings = self.render_owners(
                &vocal,
                settings,
                options,
                &routes,
                &group_names,
                &cancelled,
                &mut progress,
            )?;
            let mut assets = BTreeMap::new();
            for clip in accompaniment.iter().filter_map(|track| track.audio_clip()) {
                if assets.contains_key(&clip.path) {
                    continue;
                }
                #[cfg(not(target_arch = "wasm32"))]
                let asset = AudioAsset::read_for_mix(&clip.path, &cancelled)
                    .map_err(|error| format!("AUDIO_RESOURCE: {}: {error}", clip.path.display()))?;
                #[cfg(target_arch = "wasm32")]
                let asset = self.recording(&clip.path, &cancelled)?;
                assets.insert(clip.path.clone(), asset);
            }
            let recordings = mixer::recordings(accompaniment, &solo, |path| {
                let asset = assets.get(path)?;
                #[cfg(target_arch = "wasm32")]
                let asset = asset.as_ref();
                Some(Ok(asset))
            })?
            .layers;
            (timings, recordings)
        };
        let owners = self.cache.snapshot();
        let stats = self.cache.stats();
        let rendered_owners = timings.len();
        let references: BTreeMap<_, _> = vocal
            .tracks
            .iter()
            .flat_map(Track::references)
            .map(|reference| (OwnerId(reference.id), reference))
            .collect();
        let reports = owners
            .iter()
            .map(|owner| OwnerReport {
                group_ref_id: group_names[&owner.owner].clone(),
                timings: timings.get(&owner.owner).cloned(),
                owner: owner.clone(),
                note_ids: vocal
                    .group(&references[&owner.owner].group_id)
                    .expect("validated group")
                    .notes
                    .iter()
                    .map(|note| (note.id, note.uuid.clone()))
                    .collect(),
            })
            .collect();
        let diagnostics = if cfg!(not(target_arch = "wasm32")) && routes.is_empty() {
            Session::unavailable(
                options.backend,
                "No vocal inference in selected sources".into(),
            )
            .diagnostics()
        } else {
            self.compute
                .as_ref()
                .expect("render compute")
                .2
                .take_diagnostics()
        };
        #[cfg(target_arch = "wasm32")]
        if options.backend == Backend::WebGpu
            && rendered_owners != 0
            && diagnostics.activity.completed_gpu_submissions == 0
            && !diagnostics.activity.cpu_operators.is_empty()
        {
            self.cache.reset();
            return Err("WebGPU 未完成任何模型算子。请检查设备限制，或选择 Auto／CPU。".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if stats.current_render_bytes > options.cache_bytes {
            self.cache.reset();
        }
        #[cfg(target_arch = "wasm32")]
        if stats.current_render_bytes > options.cache_bytes {
            self.cache.retain_near(
                options.cache_bytes,
                options.preview.map_or(0.0, |demand| demand()),
            );
        }
        Ok(Output {
            report: Report {
                diagnostics,
                owners: reports,
                rendered_owners,
                reused_owners: stats.ready.saturating_sub(rendered_owners),
                cache_bytes: self.cache.stats().current_render_bytes,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                #[cfg(target_arch = "wasm32")]
                preview: metrics,
                #[cfg(target_arch = "wasm32")]
                cpu_activity: self.cpu_pool.take_activity(),
            },
            solo,
            recordings,
            owners,
            range: range.map(|range| {
                (range.start * 48_000.0).ceil() as usize..(range.end * 48_000.0).ceil() as usize
            }),
        })
    }
}
