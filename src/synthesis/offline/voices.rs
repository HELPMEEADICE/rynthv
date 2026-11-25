use super::*;
use crate::platform::fs;
use crate::synthesis::{
    execution::{self, Cancellation},
    render_cache::{Completion, RenderJob},
    rendered::RenderedClip,
    score::{self, LyricFrontend},
    voice::{PreparedVoice, VoiceError, session_identity},
};

pub(super) struct VoiceResources {
    pub(super) inventory: VoiceInventory,
    frontend: Option<LyricFrontend>,
}

impl Offline {
    pub(super) fn load_voice(
        &mut self,
        key: &VoiceSelection,
    ) -> Result<(&mut VoiceResources, Option<f64>), String> {
        use std::collections::btree_map::Entry;
        match self.voices.entry(key.clone()) {
            Entry::Occupied(cached) => Ok((cached.into_mut(), None)),
            Entry::Vacant(slot) => {
                let started = Instant::now();
                let library = self.library.as_ref().expect("scanned library");
                let entry = library
                    .catalog
                    .entry(key)
                    .filter(|entry| entry.availability.is_loadable())
                    .ok_or_else(|| {
                        format!(
                            "VOICE_NOT_FOUND: {} revision {}",
                            key.database_id, key.revision
                        )
                    })?;
                let inventory =
                    VoiceInventory::load_with_flat(&entry.directory, &library.flat_registry)
                        .map_err(|error| {
                            format!("VOICE_RESOURCE: {}: {error}", entry.directory.display())
                        })?;
                Ok((
                    slot.insert(VoiceResources {
                        inventory,
                        frontend: None,
                    }),
                    Some(started.elapsed().as_secs_f64() * 1000.0),
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_owners(
        &mut self,
        project: &Project,
        settings: &SynthesisSettings,
        options: &Options,
        routes: &BTreeMap<VoiceSelection, Vec<OwnerId>>,
        group_names: &BTreeMap<OwnerId, String>,
        cancelled: &Arc<AtomicBool>,
        progress: &mut impl FnMut(Progress),
    ) -> Result<BTreeMap<OwnerId, StageTimings>, String> {
        let mut identities = BTreeMap::new();
        let cancellation = Cancellation::from_flag(Arc::clone(cancelled));
        #[cfg(target_arch = "wasm32")]
        self.voices.retain(|voice, _| routes.contains_key(voice));
        for (key, owners) in routes {
            if cancellation.is_cancelled() {
                return Err("CANCELLED: Render cancelled".into());
            }
            let (voice, time) = self.load_voice(key)?;
            if let Some(elapsed_ms) = time {
                progress(Progress::VoiceLoaded {
                    voice: key.clone(),
                    elapsed_ms,
                });
            }
            for owner in owners {
                identities.insert(*owner, voice.inventory.content_hash.clone());
            }
        }
        self.revision = self.revision.wrapping_add(1);
        let mut plan = self
            .planner
            .update(project, self.revision, |owner| {
                identities.get(&owner).cloned()
            })
            .clone();
        plan.requests
            .retain(|owner, _| identities.contains_key(owner));
        plan.output_controls
            .retain(|owner, _| identities.contains_key(owner));
        self.cache.reconcile(&plan);
        if routes.is_empty() {
            return Ok(BTreeMap::new());
        }
        if self.compute.as_ref().is_none_or(|(backend, adapter, _)| {
            *backend != options.backend || *adapter != options.adapter
        }) {
            self.compute = Some((
                options.backend,
                options.adapter.clone(),
                Session::new(options.backend, options.adapter.as_ref()),
            ));
            self.cache.reset();
            self.cache.reconcile(&plan);
        }
        let compute = &self.compute.as_ref().expect("render compute").2;
        compute
            .check()
            .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        compute.take_diagnostics();
        let mut timings = BTreeMap::new();
        for (key, owners) in routes {
            let resource = self.voices.get_mut(key).expect("loaded voice");
            let config = resource
                .inventory
                .configuration
                .as_ref()
                .ok_or("VOICE_RESOURCE: Voice has no configuration")?;
            let voice = resource
                .inventory
                .prepare()
                .map_err(|error| format!("VOICE_RESOURCE: {error}"))?;
            let identity = if voice.family.requires_identity() {
                Some(
                    session_identity::saved_identity(&config.name, voice.family.model_version())
                        .map_err(|error| format!("MODEL_CONTEXT_REQUIRED: {error}"))?,
                )
            } else {
                None
            };
            let frontend = match &mut resource.frontend {
                Some(frontend) => frontend,
                slot => {
                    let dictionaries = settings.dictionaries.as_deref().filter(|path| fs::is_dir(path))
                        .ok_or("DICTIONARY_REQUIRED: Set the clf-data directory with config set --dictionaries")?;
                    slot.insert(
                        LyricFrontend::new(
                            dictionaries,
                            &config.default_language,
                            &config.supported_languages,
                        )
                        .map_err(|error| format!("DICTIONARY_RESOURCE: {error}"))?,
                    )
                }
            };
            compute.scope(|| {
                execution::scope(cancellation.clone(), || {
                    let mut network = None;
                    for &owner in owners {
                        execution::checkpoint()
                            .map_err(|_| "CANCELLED: Render cancelled".to_owned())?;
                        self.cache.retry_failed(owner);
                        self.cache.reuse_queued();
                        let Some(job) = self.cache.take_owner(owner) else {
                            continue;
                        };
                        let result = (|| {
                            let network = match &mut network {
                                Some(network) => network,
                                slot => slot.insert(progressive::Network::prepare(&voice)?),
                            };
                            score::validate_voice(
                                &job.request.input,
                                &config.name,
                                &config.version,
                                &resource.inventory.database_id,
                            )?;
                            render_owner(
                                &job,
                                &voice,
                                identity,
                                frontend,
                                network,
                                #[cfg(target_arch = "wasm32")]
                                options.preview_seconds,
                                |step| {
                                    execution::checkpoint()?;
                                    match step {
                                        Step::Stage(stage) => progress(Progress::Stage {
                                            group: group_names[&owner].clone(),
                                            stage,
                                        }),
                                        #[cfg(target_arch = "wasm32")]
                                        Step::Pcm(audio) => {
                                            if let Some(audio) = mixer::preview_chunk(
                                                audio,
                                                job.request.coverage,
                                                &plan.output_controls[&owner],
                                            )
                                            .map_err(VoiceError::new)?
                                            {
                                                progress(Progress::Pcm {
                                                    group: group_names[&owner].clone(),
                                                    audio,
                                                });
                                            }
                                        }
                                    }
                                    Ok(())
                                },
                            )
                        })();
                        let result = result
                            .map(|completed| {
                                timings.insert(owner, completed.timings);
                                Arc::new(RenderedClip {
                                    audio: Arc::new(completed.audio),
                                    visuals: Arc::new(completed.visuals),
                                    state: Arc::new(completed.state),
                                })
                            })
                            .map_err(|error: VoiceError| error.to_string());
                        let completion = self.cache.finish(&job, result);
                        if cancellation.is_cancelled() {
                            return Err("CANCELLED: Render cancelled".to_owned());
                        }
                        if let Completion::Failed(error) = completion {
                            return Err(format!(
                                "SYNTHESIS_FAILED: {}: {error}",
                                group_names[&owner]
                            ));
                        }
                    }
                    Ok(())
                })
            })?;
        }
        compute
            .check()
            .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        Ok(timings)
    }
}

enum Step {
    Stage(RenderStage),
    #[cfg(target_arch = "wasm32")]
    Pcm(RenderedAudio),
}

fn render_owner(
    job: &RenderJob,
    voice: &PreparedVoice,
    identity: Option<u64>,
    frontend: &LyricFrontend,
    network: &progressive::Network<'_>,
    #[cfg(target_arch = "wasm32")] preview_seconds: Option<f64>,
    mut checkpoint: impl FnMut(Step) -> crate::synthesis::voice::Result<()>,
) -> crate::synthesis::voice::Result<super::super::engine::RenderResult> {
    let mut prepared = progressive::prepare(
        voice,
        identity,
        &job.request.input,
        frontend,
        job.base.as_deref(),
        |stage| checkpoint(Step::Stage(stage)),
    )?;
    let coverage = job.request.coverage;
    let start_seconds = coverage.sample_start.0 as f64 / f64::from(coverage.sample_rate);
    #[cfg(target_arch = "wasm32")]
    let start_seconds = preview_seconds
        .filter(|seconds| {
            *seconds >= start_seconds
                && *seconds < coverage.sample_end.0 as f64 / f64::from(coverage.sample_rate)
        })
        .unwrap_or(start_seconds);
    while !prepared.is_complete() {
        let mut cursor = prepared
            .start(network, voice, start_seconds)?
            .ok_or_else(|| VoiceError::new("No inference cursor for pending audio"))?;
        while !cursor.is_finished() {
            let chunks =
                prepared.advance(&mut cursor, 128, |stage| checkpoint(Step::Stage(stage)))?;
            #[cfg(target_arch = "wasm32")]
            if preview_seconds.is_some() {
                for audio in chunks {
                    checkpoint(Step::Pcm(audio))?;
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            drop(chunks);
        }
    }
    prepared.finish()
}
