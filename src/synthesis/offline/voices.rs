use super::*;
use crate::platform::fs;
use crate::synthesis::{
    execution::{self, Cancellation},
    plan::{OwnerInput, RenderPlan},
    render_cache::{CompletedOwner, Completion, RenderJob},
    rendered::RenderedClip,
    score::{self, LyricFrontend},
    voice::{PreparedVoice, VoiceError, flat::FlatModelRegistry, session_identity},
};
use std::{cell::OnceCell, path::Path};

pub(super) struct VoiceResources {
    pub(super) inventory: VoiceInventory,
    frontend: OnceCell<LyricFrontend>,
}

pub(super) struct VoiceProgram<'a> {
    pub(super) voice: Arc<PreparedVoice>,
    pub(super) identity: Option<u64>,
    resource: &'a VoiceResources,
    frontend: &'a LyricFrontend,
}

impl VoiceResources {
    pub(super) fn load(path: &Path, registry: &FlatModelRegistry) -> Result<Self, String> {
        let inventory = VoiceInventory::load_with_flat(path, registry)
            .map_err(|error| format!("VOICE_RESOURCE: {}: {error}", path.display()))?;
        Ok(Self {
            inventory,
            frontend: OnceCell::new(),
        })
    }

    pub(super) fn program(&self, settings: &SynthesisSettings) -> Result<VoiceProgram<'_>, String> {
        let config = self
            .inventory
            .configuration
            .as_ref()
            .ok_or("VOICE_RESOURCE: Voice has no configuration")?;
        let voice = self
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
        if self.frontend.get().is_none() {
            let dictionaries = settings.dictionaries.as_deref().filter(|path| fs::is_dir(path))
                .ok_or("DICTIONARY_REQUIRED: Set the clf-data directory with config set --dictionaries")?;
            let frontend = LyricFrontend::new(
                dictionaries,
                &config.default_language,
                &config.supported_languages,
            )
            .map_err(|error| format!("DICTIONARY_RESOURCE: {error}"))?;
            let _ = self.frontend.set(frontend);
        }
        Ok(VoiceProgram {
            voice,
            identity,
            resource: self,
            frontend: self.frontend.get().expect("prepared frontend"),
        })
    }
}

impl VoiceProgram<'_> {
    pub(super) fn prepare(
        &self,
        input: &OwnerInput,
        base: Option<(&OwnerInput, &super::super::engine::OwnerSynthesis)>,
        checkpoint: impl FnMut(RenderStage) -> crate::synthesis::voice::Result<()>,
    ) -> crate::synthesis::voice::Result<progressive::PreparedRender> {
        let inventory = &self.resource.inventory;
        let config = inventory
            .configuration
            .as_ref()
            .expect("validated voice configuration");
        score::validate_voice(input, &config.name, &config.version, &inventory.database_id)?;
        progressive::prepare(
            &self.voice,
            self.identity,
            input,
            self.frontend,
            base,
            checkpoint,
        )
    }
}

pub(super) fn voice_resource<'a>(
    library: &LibraryScan,
    key: &VoiceSelection,
    slot: &'a OnceCell<VoiceResources>,
) -> Result<(&'a VoiceResources, Option<f64>), String> {
    if let Some(resource) = slot.get() {
        return Ok((resource, None));
    }
    let started = Instant::now();
    let path = voice_path(library, key)?;
    let _ = slot.set(VoiceResources::load(path, &library.flat_registry)?);
    Ok((
        slot.get().expect("loaded voice"),
        Some(started.elapsed().as_secs_f64() * 1000.0),
    ))
}

pub(super) fn voice_path<'a>(
    library: &'a LibraryScan,
    key: &VoiceSelection,
) -> Result<&'a Path, String> {
    library
        .catalog
        .entry(key)
        .filter(|entry| entry.availability.is_loadable())
        .map(|entry| entry.directory.as_path())
        .ok_or_else(|| {
            format!(
                "VOICE_NOT_FOUND: {} revision {}",
                key.database_id, key.revision
            )
        })
}

impl Offline {
    pub(super) fn load_voice(
        &mut self,
        key: &VoiceSelection,
    ) -> Result<(&VoiceResources, Option<f64>), String> {
        let slot = self.voices.entry(key.clone()).or_default();
        voice_resource(self.library.as_ref().expect("scanned library"), key, slot)
    }

    pub(super) fn prepare_owners(
        &mut self,
        project: &Project,
        options: &Options,
        routes: &BTreeMap<VoiceSelection, Vec<OwnerId>>,
        cancelled: &AtomicBool,
        progress: &mut impl FnMut(Progress),
    ) -> Result<RenderPlan, String> {
        let mut identities = BTreeMap::new();
        for (key, owners) in routes {
            if cancelled.load(Ordering::Acquire) {
                return Err("CANCELLED: Render cancelled".into());
            }
            #[cfg(target_arch = "wasm32")]
            let identity = {
                // Browser resources are managed and their replacement invalidates this session.
                // Bind the plan before opening any voice so only demanded voices load first.
                self.voices.entry(key.clone()).or_default();
                format!("{}:{}", key.revision, key.database_id)
            };
            #[cfg(not(target_arch = "wasm32"))]
            let identity = {
                let (voice, time) = self.load_voice(key)?;
                if let Some(elapsed_ms) = time {
                    progress(Progress::VoiceLoaded {
                        voice: key.clone(),
                        elapsed_ms,
                    });
                }
                voice.inventory.content_hash.clone()
            };
            for owner in owners {
                identities.insert(*owner, identity.clone());
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
        if cfg!(target_arch = "wasm32") || !routes.is_empty() {
            self.prepare_compute(options.backend, options.adapter.as_ref())
                .take_diagnostics();
        }
        #[cfg(target_arch = "wasm32")]
        self.cpu_pool.take_activity();
        self.cache.reconcile(&plan);
        for &owner in identities.keys() {
            self.cache.retry_failed(owner);
        }
        self.cache.reuse_queued();
        #[cfg(target_arch = "wasm32")]
        progress(Progress::Counts(self.cache.counts()));
        Ok(plan)
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
        let plan = self.prepare_owners(project, options, routes, cancelled, progress)?;
        if plan.requests.is_empty() {
            return Ok(BTreeMap::new());
        }
        let compute = self.compute.as_ref().expect("render compute").2.clone();
        compute
            .check()
            .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        #[cfg(target_arch = "wasm32")]
        if self.use_cpu_workers(options.backend) {
            return self.render_cpu_owners(
                &plan,
                settings,
                routes,
                group_names,
                cancelled,
                progress,
            );
        }
        let cancellation = Cancellation::from_flag(Arc::clone(cancelled));
        let mut timings = BTreeMap::new();
        let library = self.library.as_ref().expect("scanned library");
        let cache = &mut self.cache;
        compute.scope(|| {
            execution::scope(cancellation.clone(), || {
                for (key, owners) in routes {
                    let program = OnceCell::new();
                    let network = OnceCell::new();
                    for &owner in owners {
                        execution::checkpoint()
                            .map_err(|_| "CANCELLED: Render cancelled".to_owned())?;
                        cache.reuse_queued();
                        let Some(job) = cache.take_owner(owner) else {
                            continue;
                        };
                        #[cfg(target_arch = "wasm32")]
                        let partial = self
                            .partial
                            .remove(&owner)
                            .filter(|work| {
                                work.request.input == job.request.input
                                    && work.request.coverage == job.request.coverage
                            })
                            .map(|work| work.prepared);
                        let result = (|| -> Result<_, String> {
                            if program.get().is_none() {
                                let (resource, elapsed) =
                                    voice_resource(library, key, &self.voices[key])?;
                                if let Some(elapsed_ms) = elapsed {
                                    progress(Progress::VoiceLoaded {
                                        voice: key.clone(),
                                        elapsed_ms,
                                    });
                                }
                                let _ = program.set(resource.program(settings)?);
                            }
                            let program = program.get().expect("prepared voice program");
                            if network.get().is_none() {
                                let _ = network.set(
                                    progressive::Network::prepare(&program.voice)
                                        .map_err(|error| error.to_string())?,
                                );
                            }
                            let network = network.get().expect("prepared network");
                            render_owner(
                                &job,
                                program,
                                network,
                                #[cfg(target_arch = "wasm32")]
                                partial,
                                |stage| {
                                    progress(Progress::Stage {
                                        group: group_names[&owner].clone(),
                                        stage,
                                    });
                                    execution::checkpoint()
                                },
                            )
                            .map_err(|error| error.to_string())
                        })();
                        let result = result.map(|completed| {
                            timings.insert(owner, completed.timings.clone());
                            Arc::new(RenderedClip::from(completed))
                        });
                        let completion = cache.finish(&job, result);
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
                }
                Ok(())
            })
        })?;
        compute
            .check()
            .map_err(|error| format!("COMPUTE_UNAVAILABLE: {error}"))?;
        Ok(timings)
    }
}

fn render_owner<'tree>(
    job: &RenderJob,
    program: &'tree VoiceProgram<'tree>,
    network: &progressive::Network<'tree>,
    #[cfg(target_arch = "wasm32")] partial: Option<progressive::PreparedRender>,
    mut checkpoint: impl FnMut(RenderStage) -> crate::synthesis::voice::Result<()>,
) -> crate::synthesis::voice::Result<super::super::engine::RenderResult> {
    let prepare = |checkpoint| {
        program.prepare(
            &job.request.input,
            job.base.as_deref().and_then(CompletedOwner::local_baseline),
            checkpoint,
        )
    };
    #[cfg(target_arch = "wasm32")]
    let mut prepared = match partial {
        Some(prepared) => prepared,
        None => prepare(&mut checkpoint)?,
    };
    #[cfg(not(target_arch = "wasm32"))]
    let mut prepared = prepare(&mut checkpoint)?;
    prepared.suppress_pcm_publication();
    let coverage = job.request.coverage;
    let start_seconds = coverage.sample_start.0 as f64 / f64::from(coverage.sample_rate);
    while !prepared.is_complete() {
        let mut cursor = prepared
            .start(network, &program.voice, start_seconds)?
            .ok_or_else(|| VoiceError::new("No inference cursor for pending audio"))?;
        while !cursor.is_finished() {
            prepared.advance(&mut cursor, 128, &mut checkpoint)?;
        }
    }
    prepared.finish()
}
