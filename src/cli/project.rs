use super::*;
use crate::{
    editor::Editor,
    model::{Project, QUARTER, Track},
    storage::{self, ProjectFile, Revision},
    synthesis::settings::SynthesisSettings,
};

pub(super) struct OpenProject {
    pub path: PathBuf,
    revision: Option<Revision>,
    pub editor: Editor,
    pub track: Option<String>,
    pub group: Option<String>,
}

impl Workstation {
    pub(super) fn project_command(&mut self, call: Call<'_>) -> Result<Value> {
        match (call.family, call.action) {
            ("batch", "run") => self.batch_run(call),
            ("batch" | "project", "apply") => self.apply(call),
            (_, "new") => {
                let path = call.path("project")?;
                let mut editor = Editor::new(Project::default());
                if let Some(bpm) = call.get("bpm") {
                    editor.set_tempo(0, target::number(bpm)?)?;
                }
                if let Some(meter) = call.get("time-signature") {
                    let (n, d) = target::signature(meter)?;
                    editor.set_meter(0, n, d)?;
                }
                let revision = write_new(path, &editor.project, call)?;
                let result = project_info(&editor.project, path, false, revision);
                if self.streaming && !call.flag("dry-run") && self.open.is_none() {
                    self.open_path(path)?;
                }
                Ok(result)
            }
            (_, "open") => {
                if !self.streaming {
                    return Err(invalid("project open requires a REPL or stdio session"));
                }
                self.ensure_clean()?;
                self.open_path(call.path("project")?)?;
                self.open_status()
            }
            (_, "status") => self.open_status(),
            (_, "close") => {
                if !call.flag("discard") {
                    self.ensure_clean()?;
                }
                self.open.take();
                Ok(json!({"open": false}))
            }
            (_, "select") => {
                let open = self.open.as_mut().ok_or_else(no_project)?;
                let track = target::track(&open.editor.project, call.get("track"))?;
                let reference = target::reference(track, call.get("group"))?;
                open.track = Some(track.main_ref.uuid.clone());
                open.group = Some(reference.uuid.clone());
                self.open_status()
            }
            (_, "undo" | "redo") => {
                let open = self.open.as_mut().ok_or_else(no_project)?;
                if call.action == "undo" {
                    open.editor.undo();
                } else {
                    open.editor.redo();
                }
                self.open_status()
            }
            (_, "save") => {
                let open = self.open.as_mut().ok_or_else(no_project)?;
                check_revision(call, open.revision)?;
                storage::validate(&open.editor.project)?;
                if !call.flag("dry-run") {
                    open.revision = Some(
                        ProjectFile::lock(&open.path)?
                            .save_if(&open.editor.project, open.revision)?,
                    );
                    open.editor.mark_saved();
                }
                self.open_status()
            }
            (_, "clone") => {
                let mut project = self.snapshot(call.path("project")?)?.0;
                project.uuid = uuid::Uuid::new_v4().to_string();
                let path = call.path("destination")?;
                separate_from_source(call.path("project")?, path)?;
                let revision = write_new(path, &project, call)?;
                Ok(project_info(&project, path, false, revision))
            }
            (_, "info" | "inspect" | "validate") => {
                let path = call.path("project")?;
                let (project, imported, revision) = self.snapshot(path)?;
                if call.action == "info" {
                    return Ok(project_info(&project, path, imported, revision));
                }
                if call.action == "inspect" {
                    return Ok(
                        json!({"info": project_info(&project, path, imported, revision), "svp": project}),
                    );
                }
                self.validate_project(&project, path)
            }
            _ => Err(invalid("Unknown project command")),
        }
    }

    pub(super) fn snapshot(&self, path: &Path) -> Result<(Project, bool, Option<Revision>)> {
        if let Some(open) = self.current(path) {
            return Ok((open.editor.project.clone(), false, open.revision));
        }
        let loaded = storage::load(path)
            .map_err(|error| Failure::new("PROJECT_LOAD", error).details(json!({"path": path})))?;
        Ok((loaded.project, loaded.imported, Some(loaded.revision)))
    }

    pub(super) fn current(&self, path: &Path) -> Option<&OpenProject> {
        let open = self.open.as_ref()?;
        (open.path == path
            || std::fs::canonicalize(path)
                .ok()
                .zip(std::fs::canonicalize(&open.path).ok())
                .is_some_and(|(a, b)| a == b))
        .then_some(open)
    }

    fn open_path(&mut self, path: &Path) -> Result<()> {
        let path = std::path::absolute(path).map_err(|error| {
            Failure::new("PROJECT_LOAD", format!("{}: {error}", path.display()))
        })?;
        let loaded = storage::load(&path).map_err(|error| Failure::new("PROJECT_LOAD", error))?;
        if loaded.imported {
            return Err(Failure::new(
                "PROJECT_IMPORT_REQUIRED",
                "Use project clone to save the imported SVP 153 document as SVP 196, then open the copy.",
            ));
        }
        self.open = Some(OpenProject {
            path,
            revision: Some(loaded.revision),
            editor: Editor::new(loaded.project),
            track: None,
            group: None,
        });
        Ok(())
    }

    pub(super) fn ensure_clean(&self) -> Result<()> {
        if self.open.as_ref().is_some_and(|open| open.editor.dirty()) {
            return Err(Failure::new(
                "UNSAVED_CHANGES",
                "Save the open project or close it with --discard.",
            ));
        }
        Ok(())
    }

    fn open_status(&self) -> Result<Value> {
        Ok(self.open.as_ref().map_or_else(|| json!({"open": false}), |open| json!({
            "open": true, "path": open.path, "dirty": open.editor.dirty(),
            "revision": open.revision.map(Revision::hex), "track_id": open.track, "group_ref_id": open.group,
            "can_undo": open.editor.can_undo(), "can_redo": open.editor.can_redo(),
        })))
    }

    pub(super) fn mutate(&mut self, call: Call<'_>) -> Result<Value> {
        let path = call.path("project")?;
        let result = self.edit_project(path, call, |workstation, editor, selection| {
            workstation.edit(call, editor, &selection)
        })?;
        if !call.flag("dry-run") && self.current(path).is_some() {
            let session = self.open.as_mut().expect("current project");
            let edit = &result["edit"];
            match (call.family, call.action) {
                ("track", "add" | "copy") => {
                    session.track = edit["id"].as_str().map(str::to_owned);
                    session.group.clone_from(&session.track);
                }
                ("group", "add" | "copy" | "link" | "split" | "move") => {
                    session.track = edit["track_id"].as_str().map(str::to_owned);
                    session.group = edit["id"].as_str().map(str::to_owned);
                }
                _ => {}
            }
        }
        Ok(result)
    }

    fn edit_project(
        &mut self,
        path: &Path,
        call: Call<'_>,
        run: impl FnOnce(&mut Self, &mut Editor, Selection) -> Result<Value>,
    ) -> Result<Value> {
        let (project, imported, revision) = self.snapshot(path)?;
        if imported {
            return Err(Failure::new(
                "PROJECT_IMPORT_REQUIRED",
                "Clone the imported SVP 153 project before editing it.",
            ));
        }
        check_revision(call, revision)?;
        let selection = Selection::from_open(self.current(path));
        let is_open = self.current(path).is_some();
        let mut editor = Editor::new(project);
        let result = run(self, &mut editor, selection)?;
        storage::validate(&editor.project)
            .map_err(|error| Failure::new("PROJECT_INVALID", error))?;
        let revision = if call.flag("dry-run") {
            revision
        } else if is_open {
            let open = self.open.as_mut().expect("open edit");
            open.editor.begin(&call.name());
            open.editor.project = editor.project;
            open.editor.finish();
            open.revision
        } else {
            Some(ProjectFile::lock(path)?.save_if(&editor.project, revision)?)
        };
        Ok(
            json!({"dry_run": call.flag("dry-run"), "saved": !is_open && !call.flag("dry-run"),
            "revision": revision.map(Revision::hex), "edit": result}),
        )
    }

    fn apply(&mut self, call: Call<'_>) -> Result<Value> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Batch {
            operations: Vec<Vec<String>>,
        }
        let batch: Batch = self.read_json(call.required("input")?)?;
        let path = call.path("project")?;
        self.edit_project(path, call, |workstation, editor, selection| {
            let mut edits = Vec::new();
            for (index, mut args) in batch.operations.into_iter().enumerate() {
                let result = (|| {
                    if args.len() < 2 { return Err(invalid("Each operation starts with a command family and subcommand")); }
                    args.insert(2, path.to_string_lossy().into_owned());
                    let root = commands::command().try_get_matches_from(std::iter::once("rynthv -cli".to_owned()).chain(args))
                        .map_err(|error| invalid(error.to_string()))?;
                    let op = Call::parse(&root).filter(|op| op.mutation())
                        .ok_or_else(|| invalid("Only project edit commands can participate in an atomic transaction"))?;
                    if op.flag("dry-run") || op.flag("overwrite") || op.get("input") == Some("-") || op.get("expect-revision").is_some() {
                        return Err(invalid("Use transaction-level options and provide operation input files"));
                    }
                    workstation.edit(op, editor, &selection)
                })().map_err(|error| {
                    let details = json!({"operation_index": index, "cause": error.details});
                    error.details(details)
                })?;
                edits.push(result);
            }
            Ok(json!({"operations": edits}))
        })
    }

    fn batch_run(&mut self, call: Call<'_>) -> Result<Value> {
        let batch: Vec<Vec<String>> = self.read_json(call.required("input")?)?;
        let mut results = Vec::new();
        let mut failed = 0;
        for args in batch {
            let parsed = commands::command()
                .try_get_matches_from(std::iter::once("rynthv -cli".to_owned()).chain(args));
            let mut name = String::new();
            let result = parsed
                .map_err(|error| invalid(error.to_string()))
                .and_then(|args| {
                    let item = Call::parse(&args)
                        .ok_or_else(|| invalid("Batch item requires a command"))?;
                    name = item.name();
                    if item.family == "batch" || item.get("input") == Some("-") {
                        return Err(invalid(
                            "Nested batches and stdin input are unavailable inside batch run",
                        ));
                    }
                    self.execute(item)
                });
            failed += usize::from(result.is_err());
            results.push(envelope(&name, &result));
        }
        if failed > 0 {
            return Err(
                Failure::new("BATCH_FAILED", format!("{failed} batch jobs failed"))
                    .details(json!({"jobs": results})),
            );
        }
        Ok(json!({"jobs": results}))
    }

    pub(super) fn edit(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        match call.family {
            "track" | "group" => tracks::edit(call, editor, selection),
            "note" | "lyrics" => self.edit_notes(call, editor, selection),
            "phoneme" | "pitch" | "parameter" | "retake" => {
                self.edit_tuning(call, editor, selection)
            }
            "voice" | "dictionary" => self.edit_resource(call, editor, selection),
            "tempo" | "timesig" | "midi" | "audio" => self.edit_media(call, editor, selection),
            _ => Err(invalid("Unsupported project edit")),
        }
    }

    pub(super) fn project_query(&mut self, call: Call<'_>) -> Result<Value> {
        let path = call.path("project")?;
        let (project, _, _) = self.snapshot(path)?;
        let selection = Selection::from_open(self.current(path));
        match call.family {
            "track" | "group" => tracks::query(call, &project, &selection),
            "note" | "lyrics" => notes::query(call, &project, &selection),
            "pitch" | "parameter" | "phoneme" | "retake" => {
                self.query_tuning(call, &project, &selection)
            }
            "dictionary" => Ok(json!({"dictionaries": project.dictionaries})),
            "tempo" | "timesig" | "midi" => self.query_media(call, &project, &selection),
            _ => Err(invalid("Unsupported project query")),
        }
    }

    fn validate_project(&mut self, project: &Project, path: &Path) -> Result<Value> {
        storage::validate(project).map_err(|error| Failure::new("PROJECT_INVALID", error))?;
        let mut issues = Vec::new();
        for clip in project.tracks.iter().filter_map(Track::audio_clip) {
            if !clip.path.is_file() {
                issues.push(json!({"code":"AUDIO_RESOURCE","path":clip.path,"message":"Accompaniment file missing"}));
            }
        }
        let settings = match SynthesisSettings::load() {
            Ok(settings) => settings,
            Err(error) => {
                issues.push(json!({"code":"CONFIG_INVALID","message":error}));
                SynthesisSettings::default()
            }
        };
        let library = self.offline.library(&settings);
        for track in project
            .tracks
            .iter()
            .filter(|track| !track.is_instrumental())
        {
            for reference in track.references() {
                let group = project.group(&reference.group_id).expect("validated group");
                let mut ranges: Vec<_> = group
                    .notes
                    .iter()
                    .filter(|note| !note.muted())
                    .filter_map(|note| {
                        reference
                            .note_range(note)
                            .map(|range| (range, note.uuid.as_str()))
                    })
                    .collect();
                if reference.mute || ranges.is_empty() {
                    continue;
                }
                ranges.sort_unstable();
                if let Some(pair) = ranges.windows(2).find(|pair| pair[0].0.1 > pair[1].0.0) {
                    issues.push(json!({"code":"NOTE_OVERLAP","group_ref_id":reference.uuid,
                        "note_ids":[pair[0].1,pair[1].1],"message":"Visible notes overlap within a single vocal group"}));
                }
                match library.catalog.resolve(&track.main_ref.database, &reference.database) {
                    Err(message) => issues.push(json!({"code":"VOICE_NOT_FOUND","group_ref_id":reference.uuid,"message":message})),
                    Ok(None) => issues.push(json!({"code":"VOICE_NOT_ASSIGNED","group_ref_id":reference.uuid,"message":"Assign an installed voice before rendering"})),
                    Ok(Some(key)) => {
                        if !settings.dictionaries.as_ref().is_some_and(|path| path.is_dir()) {
                            issues.push(json!({"code":"DICTIONARY_REQUIRED","group_ref_id":reference.uuid,
                                "message":"Configure the clf-data directory"}));
                        }
                        if let Some(entry) = library.catalog.entry(&key)
                            && let Some(family) = entry.family.filter(|family| family.requires_identity())
                            && let Err(error) = crate::synthesis::voice::session_identity::saved_identity(&entry.display_name, family.model_version())
                        {
                            issues.push(json!({"code":"MODEL_CONTEXT_REQUIRED","group_ref_id":reference.uuid,
                                "message":error.to_string(),"voice":key}));
                        }
                    }
                }
                let name = crate::model::dictionary_name(track, reference);
                if !name.is_empty() && !project.dictionaries.contains_key(name) {
                    issues.push(json!({"code":"DICTIONARY_RESOURCE","group_ref_id":reference.uuid,"name":name,"message":"Assigned project dictionary missing"}));
                }
            }
        }
        Ok(
            json!({"path":path,"valid":true,"resources_ready":issues.is_empty(),"issues":issues,"catalog_error":library.error}),
        )
    }
}

#[derive(Default)]
pub(super) struct Selection {
    pub track: Option<String>,
    pub group: Option<String>,
}
impl Selection {
    pub fn from_open(open: Option<&OpenProject>) -> Self {
        let Some(open) = open else {
            return Self::default();
        };
        let track = open.track.as_ref().and_then(|id| {
            open.editor
                .project
                .tracks
                .iter()
                .find(|track| &track.main_ref.uuid == id)
        });
        Self {
            track: track.map(|track| track.main_ref.uuid.clone()),
            group: track
                .and_then(|track| {
                    open.group
                        .as_ref()
                        .filter(|id| track.references().any(|reference| &reference.uuid == *id))
                })
                .cloned(),
        }
    }
    pub fn track<'a>(&'a self, call: Call<'a>) -> Option<&'a str> {
        call.get("track").or_else(|| {
            self.track
                .as_deref()
                .filter(|_| call.get("group").is_none())
        })
    }
    pub fn group<'a>(&'a self, call: Call<'a>) -> Option<&'a str> {
        call.get("group").or_else(|| {
            call.get("track")
                .is_none()
                .then_some(self.group.as_deref())
                .flatten()
        })
    }
}

fn no_project() -> Failure {
    Failure::new("NO_PROJECT", "Open a project first with project open FILE")
}

fn write_new(path: &Path, project: &Project, call: Call<'_>) -> Result<Option<Revision>> {
    storage::validate(project).map_err(|error| Failure::new("PROJECT_INVALID", error))?;
    let overwrite = call.flag("overwrite");
    check_output(path, overwrite)?;
    if !call.flag("dry-run") {
        let file = ProjectFile::lock(path)?;
        check_output(path, overwrite)?;
        let expected = if overwrite {
            Revision::read(path)?
        } else {
            None
        };
        return Ok(Some(file.save_if(project, expected)?));
    }
    Ok(None)
}

fn check_revision(call: Call<'_>, revision: Option<Revision>) -> Result<()> {
    if let Some(expected) = call.get("expect-revision")
        && revision.map(Revision::hex).as_deref() != Some(expected)
    {
        return Err(Failure::new(
            "PROJECT_CONFLICT",
            "Project revision differs from the supplied SHA256. Reload it before retrying.",
        )
        .details(json!({"expected":expected,"actual":revision.map(Revision::hex)})));
    }
    Ok(())
}

fn project_info(
    project: &Project,
    path: &Path,
    imported: bool,
    revision: Option<Revision>,
) -> Value {
    let end = project.end();
    json!({
        "path": path, "project_id": project.uuid, "svp_version": project.version,
        "imported": imported, "revision": revision.map(Revision::hex), "blick_per_beat": QUARTER,
        "end_blick": end, "duration_seconds": project.time.seconds_at(end).max(0.0),
        "render_config": project.render_config, "tracks": project.tracks.iter().map(tracks::track_json).collect::<Vec<_>>()
    })
}
