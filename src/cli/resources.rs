use super::{project::Selection, target, *};
use crate::{
    editor::Editor,
    model::{Language, PronunciationDictionary},
    synthesis::{
        engine::progressive,
        inference::compute::{self, AdapterId, Backend},
        score::LyricFrontend,
        settings::{SynthesisSettings, VoiceSelection},
        voice::{VoiceCatalog, VoiceCatalogEntry, VoiceFamily, flat, session_identity},
    },
};
use std::collections::BTreeMap;

pub(super) fn settings() -> Result<SynthesisSettings> {
    SynthesisSettings::load().map_err(|error| Failure::new("CONFIG_INVALID", error))
}
pub(super) fn backend(value: &str) -> Result<Backend> {
    match value {
        "auto" => Ok(Backend::Auto),
        "cpu" => Ok(Backend::Cpu),
        "webgpu" => Ok(Backend::WebGpu),
        _ => Err(invalid("Expected auto, cpu or webgpu")),
    }
}
pub(super) fn adapter(value: &str) -> Result<Option<AdapterId>> {
    if value == "auto" {
        return Ok(None);
    }
    serde_json::from_str(value).map(Some).map_err(|error| {
        invalid(format!(
            "Pass the adapter id JSON object from doctor --probe-webgpu: {error}"
        ))
    })
}
pub(super) fn dictionary_frontend(language: Language) -> Result<LyricFrontend> {
    let path = dictionary_root(&settings()?)?;
    LyricFrontend::new(path, language.id(), &[language.id().to_owned()])
        .map_err(|error| Failure::new("DICTIONARY_RESOURCE", error.to_string()))
}
pub(super) fn dictionary_root(settings: &SynthesisSettings) -> Result<PathBuf> {
    settings
        .dictionaries
        .as_ref()
        .filter(|path| path.is_dir())
        .cloned()
        .ok_or_else(|| {
            Failure::new(
                "DICTIONARY_REQUIRED",
                "Set the clf-data directory with config set --dictionaries",
            )
        })
}
fn resolve_entry<'a>(
    catalog: &'a VoiceCatalog,
    id: &str,
    revision: Option<u32>,
) -> Result<&'a VoiceCatalogEntry> {
    catalog
        .entries
        .iter()
        .filter(|entry| {
            entry.database_id == id && revision.is_none_or(|revision| revision == entry.revision)
        })
        .max_by_key(|entry| (entry.availability.is_loadable(), entry.revision))
        .ok_or_else(|| {
            Failure::new(
                "VOICE_NOT_FOUND",
                "Voice was not found. Check voice list and your configured roots.",
            )
            .details(json!({"voice_id":id,"revision":revision}))
        })
}
fn selectable(entry: &VoiceCatalogEntry) -> Result<VoiceSelection> {
    if let crate::synthesis::voice::VoiceAvailability::Unsupported(error) = &entry.availability {
        return Err(Failure::new("VOICE_RESOURCE", error)
            .details(json!({"voice_id":entry.database_id,"revision":entry.revision})));
    }
    Ok(entry.selection())
}
fn dictionary_files(settings: &SynthesisSettings) -> Result<Vec<PathBuf>> {
    let Some(path) = settings.dictionaries.as_ref() else {
        return Ok(Vec::new());
    };
    let mut files: Vec<_> = std::fs::read_dir(path)
        .map_err(|error| Failure::new("DICTIONARY_RESOURCE", error.to_string()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()
        .map_err(|error| Failure::new("DICTIONARY_RESOURCE", error.to_string()))?;
    files.sort();
    Ok(files)
}
impl Workstation {
    pub(super) fn schema(&self, name: Option<&str>) -> Result<Value> {
        let mut command = commands::command();
        command.build();
        let mut entry = &command;
        if let Some(name) = name {
            for segment in name.split('.') {
                entry = entry
                    .find_subcommand(segment)
                    .filter(|command| command.get_name() != "help")
                    .ok_or_else(|| {
                        Failure::new("UNKNOWN_COMMAND", format!("Unknown command {name}"))
                    })?;
            }
        }
        Ok(json!({
            "protocol_version": SCHEMA_VERSION,
            "command": commands::schema(entry),
            "interfaces": {
                "result": {"required":["schema_version","ok","command","warnings"],"success":["result"],"failure":["error"]},
                "stdio_request": {"type":"object","required":["id","args"],"properties":{
                    "id":{"type":["string","integer"]},"args":{"type":"array","items":{"type":"string"}},
                    "input":{}}, "shutdown":{"args":["exit"]}},
                "batch_apply_input":{"type":"object","required":["operations"],"properties":{
                    "operations":{"type":"array","items":{"type":"array","items":{"type":"string"}}}}},
                "note_import_input":{"type":"array","items":{"type":"object","required":["at","duration","pitch"],
                    "properties":{"at":{"type":["number","string"]},"duration":{"type":["number","string"]},
                        "pitch":{"type":["integer","string"]},"lyric":{"type":"string"},"phonemes":{"type":"string"},
                        "language":{"enum":Language::ALL.map(Language::id)},"mute":{"type":"boolean"}}}},
                "curve_input":{"type":"array","items":{"type":"array","prefixItems":[{"type":"integer","unit":"local_blick"},{"type":"number"}],"minItems":2,"maxItems":2}}
            },
            "limits":{"midi_note":[0,127],"pan":[-1,1],"blick_per_quarter":crate::model::QUARTER,
                "time_signature_denominators":[1,2,4,8,16,32,64],"time_signature_numerator":[1,64],
                "parameters":super::tuning::parameter_specs()},
            "exit_codes":{"success":0,"io":1,"usage":2,"operation_failed":3,"conflict":4,"cancelled":130}
        }))
    }

    pub(super) fn capabilities(&mut self) -> Result<Value> {
        let settings = settings();
        let mut command = commands::command();
        command.build();
        let (voices, error) = match settings {
            Ok(settings) => {
                let library = self.offline.library(&settings);
                (json!(library.catalog.entries), library.error.clone())
            }
            Err(error) => (json!([]), Some(error.message)),
        };
        Ok(json!({
            "version":env!("CARGO_PKG_VERSION"), "protocol_version":SCHEMA_VERSION,
            "gui_built":cfg!(feature="gui"),"command":commands::schema(&command),
            "voice_families":["SV1","SV2","SV2-compatible","Flat"],"voices":voices,"catalog_error":error,
            "render_backends":["cpu","webgpu","auto"],"wav_bits":[16,24,32],
            "features":{"atomic_project_edits":true,"stdio":true,"repl":true,"predicted_pitch":true,
                "audio_f0_estimation":true,"lossless_midi_tuning":false,"group_instance_retakes":false,
                "merge_unrelated_group_definitions":false,"independent_background_jobs":false},
            "keyboard_history":"current REPL session"
        }))
    }

    pub(super) fn doctor(&mut self, probe: bool) -> Result<Value> {
        let loaded = settings();
        let result = match loaded {
            Ok(settings) => {
                let roots = [
                    ("databases", settings.databases.as_ref()), ("sv1_databases", settings.sv1_databases.as_ref()),
                    ("flat_databases", settings.flat_databases.as_ref()), ("flat_models", settings.flat_models.as_ref()),
                    ("dictionaries", settings.dictionaries.as_ref())
                ].map(|(kind, path)| json!({"kind":kind,"path":path,"exists":path.is_some_and(|path| path.exists())}));
                let library = self.offline.library(&settings);
                json!({"ok":library.error.is_none(),"roots":roots,"catalog_error":library.error,"catalog_warnings":library.catalog.warnings,
                    "voice_count":library.catalog.entries.len(),"configured_backend":settings.compute_backend})
            }
            Err(error) => json!({"ok":false,"error":error.message}),
        };
        let devices = probe.then(|| {
            compute::list_devices()
                .into_iter()
                .map(|info| json!({"id":info.id,"selectable":info.is_selectable(),"device":info}))
                .collect::<Vec<_>>()
        });
        Ok(json!({
            "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"gui_built":cfg!(feature="gui"),
            "display_environment":{"display":std::env::var_os("DISPLAY").is_some(),"wayland_display":std::env::var_os("WAYLAND_DISPLAY").is_some()},
            "config_path":SynthesisSettings::path().ok(),"resources":result,"webgpu_adapters":devices
        }))
    }

    pub(super) fn resource_command(&mut self, call: Call<'_>) -> Result<Value> {
        let mut settings = settings()?;
        match (call.family, call.action) {
            ("config", "show") => {
                Ok(json!({"path": SynthesisSettings::path()?, "settings": settings}))
            }
            ("config", "set") => {
                let paths = [
                    ("databases", &mut settings.databases),
                    ("sv1-databases", &mut settings.sv1_databases),
                    ("flat-databases", &mut settings.flat_databases),
                    ("flat-models", &mut settings.flat_models),
                    ("dictionaries", &mut settings.dictionaries),
                ];
                for (key, slot) in paths {
                    if let Some(value) = call.get(key) {
                        if value == "none" {
                            *slot = None;
                            continue;
                        }
                        let path = std::fs::canonicalize(value)
                            .map_err(|error| invalid(format!("{key}: {value}: {error}")))?;
                        if !path.is_dir() && !(key == "sv1-databases" && path.is_file()) {
                            return Err(invalid(format!("{key} requires a directory")));
                        }
                        *slot = Some(path);
                    }
                }
                if let Some(value) = call.get("compute-backend") {
                    settings.compute_backend = backend(value)?;
                }
                if let Some(value) = call.get("compute-adapter") {
                    settings.compute_adapter = adapter(value)?;
                }
                if !call.flag("dry-run") {
                    settings
                        .save()
                        .map_err(|error| Failure::new("CONFIG_WRITE", error))?;
                    self.offline.refresh();
                }
                Ok(
                    json!({"path":SynthesisSettings::path()?,"settings":settings,"dry_run":call.flag("dry-run")}),
                )
            }
            ("dictionary", "list") => Ok(
                json!({"directory":settings.dictionaries,"files":dictionary_files(&settings)?,"languages":Language::ALL}),
            ),
            ("dictionary", "lookup") => {
                let language = Language::parse(call.required("language")?)
                    .ok_or_else(|| invalid("Unsupported language"))?;
                let word = call.required("word")?;
                let syllables = dictionary_frontend(language)?
                    .resolve(language, word)
                    .map_err(|error| {
                        Failure::new("LYRIC_UNRESOLVED", error.to_string())
                            .details(json!({"word":word,"language":language}))
                    })?;
                Ok(json!({"word":word,"language":language,"syllables":syllables}))
            }
            ("voice", "scan" | "list") => {
                if call.action == "scan" {
                    self.offline.refresh();
                }
                let library = self.offline.library(&settings);
                Ok(
                    json!({"voices":library.catalog.entries,"warnings":library.catalog.warnings,
                    "scan_error":library.error,"flat_models":library.flat_registry.models().collect::<Vec<_>>()}),
                )
            }
            ("voice", "import") => {
                let path = call.path("input")?;
                let root = settings.flat_voice_directory()?;
                if call.flag("dry-run") {
                    let manifests = flat::inspect_import(path)
                        .map_err(|error| Failure::new("VOICE_RESOURCE", error.to_string()))?;
                    return Ok(json!({"dry_run":true,"destination":root,"voices":manifests}));
                }
                let voices = flat::import(path, &root)
                    .map_err(|error| Failure::new("VOICE_RESOURCE", error.to_string()))?;
                self.offline.refresh();
                Ok(json!({"voices":voices}))
            }
            ("voice", "inspect" | "validate" | "modes" | "context") => {
                let (key, entry) = {
                    let entry = resolve_entry(
                        &self.offline.library(&settings).catalog,
                        call.required("voice")?,
                        call.number("revision")?,
                    )?;
                    (selectable(entry)?, entry.clone())
                };
                let inventory = self.offline.inventory(&settings, &key)?;
                let config = inventory.configuration.as_ref().ok_or_else(|| {
                    Failure::new(
                        "VOICE_RESOURCE",
                        "Voice metadata has no synthesis configuration",
                    )
                })?;
                match call.action {
                    "inspect" => Ok(json!({"entry":entry,"inventory":inventory})),
                    "modes" => {
                        let prepared = inventory.prepare().map_err(voice_failure)?;
                        Ok(
                            json!({"voice_id":key.database_id,"revision":key.revision,"family":prepared.family,
                            "languages":config.supported_languages,"default_language":config.default_language,
                            "modes":prepared.style_names,"phones":prepared.phone_inventory().map_err(voice_failure)?,
                            "editor_parameter_limits":super::tuning::parameter_specs(),
                            "model_features":{"pitch_anchors":prepared.family == VoiceFamily::Sv2,
                                "duration_retake":prepared.family == VoiceFamily::Sv2,
                                "pitch_retake":true,"timbre_retake":true,"pitch_curve":true}}),
                        )
                    }
                    "validate" => {
                        let prepared = inventory.prepare().map_err(voice_failure)?;
                        progressive::Network::prepare(&prepared).map_err(voice_failure)?;
                        if prepared.family.requires_identity() {
                            session_identity::saved_identity(
                                &config.name,
                                prepared.family.model_version(),
                            )
                            .map_err(|error| {
                                Failure::new("MODEL_CONTEXT_REQUIRED", error.to_string())
                            })?;
                        }
                        Ok(
                            json!({"valid":true,"voice_id":key.database_id,"revision":key.revision,"preparation":prepared.summary()}),
                        )
                    }
                    "context" => {
                        let source = call.path("input")?;
                        let version = config.family.model_version();
                        if !config.family.requires_identity() {
                            return Err(invalid("This voice does not require a model context"));
                        }
                        session_identity::validate_context_file(&config.name, version, source)
                            .map_err(voice_failure)?;
                        if session_identity::has_context_record(&config.name, version)
                            .map_err(voice_failure)?
                            && !call.flag("overwrite")
                        {
                            return Err(Failure::new(
                                "OUTPUT_EXISTS",
                                "A context is already stored for this voice. Use --overwrite to replace it.",
                            ));
                        }
                        if !call.flag("dry-run") {
                            session_identity::import_context_file_with_policy(
                                &config.name,
                                version,
                                source,
                                call.flag("overwrite"),
                            )
                            .map_err(|error| Failure::from(error.to_string()))?;
                            self.offline.refresh();
                        }
                        Ok(
                            json!({"voice_id":key.database_id,"model_version":version,"dry_run":call.flag("dry-run")}),
                        )
                    }
                    _ => unreachable!(),
                }
            }
            _ => Err(invalid("Unsupported resource command")),
        }
    }

    pub(super) fn edit_resource(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        if call.family == "dictionary" && call.action == "delete" {
            let name = call.required("name")?;
            if !editor.project.dictionaries.contains_key(name) {
                return Err(invalid("Project dictionary was not found"));
            }
            editor.remove_dictionary(name);
            return Ok(json!({"removed":name}));
        }
        let track = target::scope_track(
            &editor.project,
            selection.track(call),
            selection.group(call),
        )?;
        if track.is_instrumental() {
            return Err(invalid("Choose a vocal track"));
        }
        let track_id = track.id;
        let reference = target::reference(track, selection.group(call))?;
        let reference_id = reference.id;
        if call.family == "voice" {
            let settings = settings()?;
            let id = call.required("voice")?;
            let entry = resolve_entry(
                &self.offline.library(&settings).catalog,
                id,
                call.number("revision")?,
            )?;
            let key = selectable(entry)?;
            let name = entry.display_name.clone();
            let language = call.get("language").and_then(Language::parse);
            if let Some(language) = language
                && !entry.languages.is_empty()
                && !entry
                    .languages
                    .iter()
                    .filter_map(|value| Language::parse(value))
                    .any(|item| item == language)
            {
                return Err(Failure::new(
                    "LANGUAGE_UNSUPPORTED",
                    "The selected voice does not list the requested language",
                ));
            }
            editor.select_track(track_id);
            if selection.group(call).is_some() {
                editor.set_instance_voice(
                    track_id,
                    reference_id,
                    (&key.database_id, &name, key.revision),
                )?;
                editor.select_reference(track_id, reference_id);
            } else {
                editor.set_track_voice(Some((&key.database_id, &name, key.revision)))?;
            }
            if let Some(language) = language {
                editor.set_voice_language(Some(language))?;
            }
            return Ok(json!({"voice":key,"language":language}));
        }
        let name = call.required("name")?.trim();
        if name.is_empty() {
            return Err(invalid("Dictionary name must be nonempty"));
        }
        let language = Language::parse(call.required("language")?)
            .ok_or_else(|| invalid("Unsupported language"))?;
        let dictionary = editor
            .project
            .dictionaries
            .entry(name.to_owned())
            .or_insert_with(|| PronunciationDictionary {
                language,
                entries: BTreeMap::new(),
            });
        if dictionary.language != language {
            return Err(invalid("Existing dictionary uses a different language"));
        }
        if let Some(word) = call.get("word") {
            editor.set_dictionary_entry(name, word, call.get("phones"))?;
        } else if call.get("phones").is_some() {
            return Err(invalid("--phones requires --word"));
        }
        if call.flag("assign") {
            editor
                .project
                .tracks
                .iter_mut()
                .flat_map(|track| std::iter::once(&mut track.main_ref).chain(&mut track.groups))
                .find(|reference| reference.id == reference_id)
                .expect("selected instance")
                .extra
                .insert("dictionary".into(), json!(name));
        }
        Ok(json!({"name":name,"language":language,"assigned":call.flag("assign")}))
    }
}
fn voice_failure(error: crate::synthesis::voice::VoiceError) -> Failure {
    Failure::new("VOICE_RESOURCE", error.to_string())
}
