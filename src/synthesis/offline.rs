//! Project rendering without a window, a display surface, or a playback device.
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
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use voices::VoiceResources;

#[derive(Default)]
pub struct Offline {
    library: Option<LibraryScan>,
    settings: Option<SynthesisSettings>,
    voices: BTreeMap<VoiceSelection, VoiceResources>,
    compute: Option<(Backend, Option<AdapterId>, Session)>,
    planner: Planner,
    cache: RenderCache,
    project: String,
    owner_ids: BTreeMap<String, OwnerId>,
    note_ids: BTreeMap<String, u64>,
    revision: u64,
}

pub struct Options {
    pub tracks: BTreeSet<u64>,
    pub group: Option<u64>,
    pub range_seconds: Option<Range<f64>>,
    pub vocals_only: bool,
    pub backend: Backend,
    pub adapter: Option<AdapterId>,
    /// Drop acoustic cache at this soft limit between requests.
    pub cache_bytes: usize,
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
}

pub struct Report {
    pub diagnostics: Diagnostics,
    pub owners: Vec<OwnerReport>,
    pub rendered_owners: usize,
    pub reused_owners: usize,
    pub cache_bytes: usize,
    pub elapsed_ms: f64,
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
    pub fn refresh(&mut self) {
        self.library = None;
        self.voices.clear();
        if let Some((_, _, session)) = &self.compute {
            session.prune_weights();
        }
        self.cache.reset();
        self.planner.reset();
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
        if self.project != project.uuid {
            self.cache.reset();
            self.planner.reset();
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
        self.owner_ids.retain(|uuid, _| {
            project
                .tracks
                .iter()
                .flat_map(Track::references)
                .any(|reference| &reference.uuid == uuid)
        });
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
                    })?
                    .ok_or_else(|| {
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
        let timings = self.render_owners(
            &vocal,
            settings,
            options,
            &routes,
            &group_names,
            &cancelled,
            &mut progress,
        )?;
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
        let mut assets = BTreeMap::new();
        for clip in accompaniment.iter().filter_map(|track| track.audio_clip()) {
            if !assets.contains_key(&clip.path) {
                let asset = AudioAsset::read_for_mix(&clip.path, &cancelled)
                    .map_err(|error| format!("AUDIO_RESOURCE: {}: {error}", clip.path.display()))?;
                assets.insert(clip.path.clone(), asset);
            }
        }
        let recordings = mixer::recordings(accompaniment, &solo, |path| assets.get(path).map(Ok))?;
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
        let diagnostics = if routes.is_empty() {
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
        if stats.current_render_bytes > options.cache_bytes {
            self.cache.reset();
        }
        Ok(Output {
            report: Report {
                diagnostics,
                owners: reports,
                rendered_owners,
                reused_owners: stats.ready.saturating_sub(rendered_owners),
                cache_bytes: self.cache.stats().current_render_bytes,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            },
            solo,
            recordings: recordings.layers,
            owners,
            range: range.map(|range| {
                (range.start * 48_000.0).ceil() as usize..(range.end * 48_000.0).ceil() as usize
            }),
        })
    }
}
