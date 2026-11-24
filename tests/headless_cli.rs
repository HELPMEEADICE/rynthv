#![cfg(feature = "cli")]

use rynthv::{
    model::Project,
    storage::{self, ProjectFile},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Workspace(PathBuf);

fn check_render_cancellation(work: &Workspace, bank_root: &str, dictionaries: &str) {
    use rynthv::synthesis::{
        inference::compute::Backend,
        offline::{Offline, Options, Progress},
        settings::SynthesisSettings,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let project = storage::load(&work.path("song.svp")).unwrap().project;
    let settings = SynthesisSettings {
        sv1_databases: Some(bank_root.into()),
        dictionaries: Some(dictionaries.into()),
        compute_backend: Backend::Cpu,
        ..SynthesisSettings::default()
    };
    let options = Options {
        tracks: Default::default(),
        group: None,
        range_seconds: None,
        vocals_only: true,
        backend: Backend::Cpu,
        adapter: None,
        cache_bytes: 1024 * 1024 * 64,
    };
    let mut offline = Offline::default();
    let flag = Arc::new(AtomicBool::new(false));
    let error = offline
        .render(&project, &settings, &options, Arc::clone(&flag), |event| {
            if matches!(event, Progress::Stage { .. }) {
                flag.store(true, Ordering::Release);
            }
        })
        .err()
        .expect("current synthesis should have been cancelled");
    assert!(error.starts_with("CANCELLED:"), "{error}");

    flag.store(false, Ordering::Release);
    let completed = offline
        .render(&project, &settings, &options, flag, |_| {})
        .unwrap();
    assert_eq!(completed.report.rendered_owners, 1);
    let audio = completed.mix(None).unwrap().unwrap();
    assert!(audio.samples().iter().any(|sample| sample.abs() > 0.0001));
}

#[cfg(feature = "gui")]
#[test]
#[ignore = "subprocess used by native_project_renders_with_an_installed_voice"]
fn export_through_editor_runtime() {
    use rynthv::synthesis::{
        notify::Notify,
        runtime::{ExportRequest, Runtime},
    };
    use std::time::{Duration, Instant};

    let directory =
        PathBuf::from(std::env::var_os("RYNTHV_PARITY_DIR").expect("parent fixture path"));
    let mut project = storage::load(&directory.join("song.svp")).unwrap().project;
    for (key, value) in [
        ("destination", json!(directory)),
        ("filename", json!("editor-export")),
        ("sampleRate", json!(48_000)),
        ("bitDepth", json!(32)),
        ("numChannels", json!(1)),
    ] {
        project.render_config.insert(key.into(), value);
    }
    let request = ExportRequest::from_project(&project).unwrap();
    let mut runtime = Runtime::new(Notify::new(|| {}));
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        runtime.poll();
        runtime.sync_project(&project, 1);
        if runtime.export_ready(1) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Editor export did not become ready: {:?}",
            runtime.error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    runtime.export_project(request, 1);
    loop {
        runtime.poll();
        if let Some(result) = runtime.export_result.take() {
            assert_eq!(result.unwrap(), vec![directory.join("editor-export.wav")]);
            return;
        }
        assert!(Instant::now() < deadline, "Editor export did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn shared_groups_and_tempo_aware_tuning_are_safe_to_edit() {
    let work = Workspace::new();
    work.args(&["project", "new", "song.svp", "--bpm", "120"]);
    work.args(&["tempo", "set", "song.svp", "--beat", "1", "--bpm", "60"]);
    let group = work.args(&[
        "group",
        "add",
        "song.svp",
        "--name",
        "Verse",
        "--duration",
        "8",
    ]);
    let group = group["edit"]["id"].as_str().unwrap();
    let note = work.args(&[
        "note",
        "add",
        "song.svp",
        "--group",
        group,
        "--at",
        "0.25",
        "--duration",
        "0.5",
        "--unit",
        "second",
        "--pitch",
        "C4",
    ]);
    let note = &note["edit"]["notes"][0];
    assert_eq!(note["duration_blick"], 529_200_000);
    let id = note["id"].as_str().unwrap();
    let linked = work.args(&["group", "link", "song.svp", "--group", group, "--at", "8"]);
    let linked = linked["edit"]["id"].as_str().unwrap();
    let (status, error) = reply(
        work.cli()
            .args([
                "note", "set", "song.svp", "--group", group, "--note", id, "--pitch", "E4",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    assert!(!status.status.success());
    assert_eq!(error["error"]["code"], "SHARED_GROUP");
    work.args(&["group", "independent", "song.svp", "--group", linked]);
    let independent = work.args(&["note", "list", "song.svp", "--group", linked]);
    assert_ne!(independent["notes"][0]["id"], id);
    work.args(&[
        "note",
        "set",
        "song.svp",
        "--note",
        id,
        "--lyric",
        "あ",
        "--phonemes",
        "a",
    ]);
    fs::write(
        work.path("phoneme.json"),
        br#"{"leftOffset":-0.03,"durationScale":1.25}"#,
    )
    .unwrap();
    work.args(&[
        "phoneme",
        "adjust",
        "song.svp",
        "--note",
        id,
        "--index",
        "0",
        "--input",
        "phoneme.json",
    ]);
    let edited = work.args(&["phoneme", "inspect", "song.svp", "--note", id]);
    assert_eq!(edited["mapping"][0]["override"], "a");
    assert_eq!(
        edited["mapping"][0]["stored_attributes"][0]["durationScale"],
        1.25
    );
    work.args(&[
        "parameter",
        "point",
        "song.svp",
        "--group",
        group,
        "--name",
        "pitchDelta",
        "--at",
        "0",
        "--value",
        "20",
    ]);
    let pitch = work.args(&["pitch", "inspect", "song.svp", "--group", group]);
    assert_eq!(pitch["pitch_delta"]["points"], json!([0, 20.0]));
}
impl Workspace {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("rynthv-cli-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn cli(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rynthv"));
        command
            .current_dir(&self.0)
            .env("LOCALAPPDATA", &self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .arg("-cli");
        command
    }
    fn args(&self, args: &[&str]) -> Value {
        let (output, document) = reply(self.cli().args(args).arg("--json").output().unwrap());
        assert!(output.status.success(), "{document}");
        document["result"].clone()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn reply(output: Output) -> (Output, Value) {
    let document: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {:?}\n{}",
            output,
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(document["schema_version"], 1);
    (output, document)
}

#[test]
fn cli_help_and_doctor_need_no_desktop_resources() {
    let work = Workspace::new();
    let help = work.cli().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("headless"));
    let doctor = work.args(&["doctor"]);
    assert_eq!(doctor["display_environment"]["display"], false);
    assert!(doctor["webgpu_adapters"].is_null());
    let output = work.cli().stdin(Stdio::null()).output().unwrap();
    assert!(output.status.success());
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("rynthv>")
    );
    let export_help = Command::new(env!("CARGO_BIN_EXE_rynthv-synth"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(export_help.status.success());
    assert!(String::from_utf8_lossy(&export_help.stdout).contains("render-score"));
}

#[test]
fn stable_ids_unicode_midi_and_legacy_metadata_roundtrip() {
    let work = Workspace::new();
    let name = "夜空.svp";
    let created = work.args(&[
        "project",
        "new",
        name,
        "--bpm",
        "128",
        "--time-signature",
        "7/8",
    ]);
    let track = created["tracks"][0]["id"].as_str().unwrap();
    let note = work.args(&[
        "note",
        "add",
        name,
        "--track",
        track,
        "--at",
        "0",
        "--duration",
        "1",
        "--pitch",
        "Bb4",
        "--lyric",
        "そ",
    ]);
    let id = note["edit"]["notes"][0]["id"].as_str().unwrap();
    work.args(&["note", "set", name, "--note", id, "--pitch", "C5"]);
    let note = work.args(&["note", "list", name, "--note", id]);
    assert_eq!(note["notes"][0]["midi_pitch"], 72);
    work.args(&["midi", "export", name, "--output", "song.mid"]);
    let midi = work.args(&["midi", "inspect", "song.mid"]);
    let notes = midi["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|track| track["notes"].as_array()?.first())
        .expect("vocal MIDI track");
    assert_eq!(notes["lyrics"], "そ");
    work.args(&["project", "new", "import.svp"]);
    work.args(&[
        "midi",
        "import",
        "import.svp",
        "--input",
        "song.mid",
        "--time-map",
    ]);
    assert_eq!(
        work.args(&["timesig", "list", "import.svp"])["meters"][0]["numerator"],
        7
    );
    let mut raw: Value = serde_json::from_slice(&fs::read(work.path(name)).unwrap()).unwrap();
    raw["tracks"][0]["mainGroup"]["notes"][0]
        .as_object_mut()
        .unwrap()
        .remove("uuid");
    raw["futureExtension"] = json!({"preserve":true});
    fs::write(work.path(name), serde_json::to_vec(&raw).unwrap()).unwrap();
    let note = work.args(&["note", "list", name]);
    let stable = note["notes"][0]["id"].as_str().unwrap();
    assert_eq!(work.args(&["note", "list", name])["notes"][0]["id"], stable);
    work.args(&["phoneme", "set", name, "--note", stable, "--value", "s o"]);
    assert_eq!(
        work.args(&["project", "inspect", name])["svp"]["futureExtension"]["preserve"],
        true
    );
}
#[test]
fn json_transactions_and_concurrency_do_not_overwrite_changes() {
    let work = Workspace::new();
    work.args(&["project", "new", "song.svp"]);
    let before = fs::read(work.path("song.svp")).unwrap();
    fs::write(work.path("batch.json"),br#"{"operations":[["note","add","--at","0","--duration","1","--pitch","C4"],["note","set","--note","missing","--pitch","D4"]]}"#).unwrap();
    let (output, error) = reply(
        work.cli()
            .args([
                "project",
                "apply",
                "song.svp",
                "--input",
                "batch.json",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    assert!(!output.status.success());
    assert_eq!(error["error"]["code"], "NOTE_NOT_FOUND");
    assert_eq!(error["error"]["details"]["operation_index"], 1);
    assert_eq!(fs::read(work.path("song.svp")).unwrap(), before);
    let _lock = ProjectFile::lock(&work.path("song.svp")).unwrap();
    let (output, error) = reply(
        work.cli()
            .args(["track", "add", "song.svp", "--name", "Conflict", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(error["error"]["code"], "PROJECT_CONFLICT");
    assert_eq!(fs::read(work.path("song.svp")).unwrap(), before);
    work.args(&["track", "add", "song.svp", "--name", "Preview", "--dry-run"]);
    assert_eq!(fs::read(work.path("song.svp")).unwrap(), before);
}

#[test]
fn stdio_session_can_undo_redo_and_recover_after_error() {
    let work = Workspace::new();
    work.args(&["project", "new", "song.svp"]);
    let mut child = work
        .cli()
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        let requests = [
            json!({"id":1,"args":["project","open","song.svp"]}),
            json!({"id":2,"args":["note","import","--input","-"],"input":[{"at":0,"duration":1,"pitch":"D4"}]}),
            json!({"id":3,"args":["note","set","--note","missing","--pitch","E4"]}),
            json!({"id":4,"args":["project","undo"]}),
            json!({"id":5,"args":["project","redo"]}),
            json!({"id":6,"args":["exit"]}),
            json!({"id":7,"args":["project","save"]}),
            json!({"id":8,"args":["exit"]}),
        ];
        for request in requests {
            writeln!(input, "{request}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 8);
    assert_eq!(replies[1]["ok"], true, "{:?}", replies[1]);
    assert_eq!(replies[2]["error"]["code"], "NOTE_NOT_FOUND");
    assert_eq!(replies[3]["result"]["can_redo"], true);
    assert_eq!(replies[4]["result"]["dirty"], true);
    assert_eq!(replies[5]["error"]["code"], "UNSAVED_CHANGES");
    assert_eq!(replies[7]["id"], 8);
    assert_eq!(
        work.args(&["note", "list", "song.svp"])["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn optimistic_save_rejects_a_stale_revision() {
    let work = Workspace::new();
    let path = work.path("song.svp");
    storage::save(&path, &Project::default()).unwrap();
    let loaded = storage::load(&path).unwrap();
    let mut other = loaded.project.clone();
    other.tracks[0].name = "Updated".into();
    storage::save(&path, &other).unwrap();
    let file = ProjectFile::lock(&path).unwrap();
    assert!(
        file.save_if(&loaded.project, Some(loaded.revision))
            .unwrap_err()
            .contains("PROJECT_CONFLICT")
    );
    assert_eq!(
        storage::load(&path).unwrap().project.tracks[0].name,
        "Updated"
    );
}

#[test]
fn mixed_audio_and_stems_preserve_native_samples() {
    use rynthv::synthesis::audio::{ChannelLayout, RenderedAudio, save_wav};
    let work = Workspace::new();
    let wave = RenderedAudio::new(
        48_000,
        0.0,
        ChannelLayout::Mono,
        (0..4800)
            .map(|i| (i as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 0.2)
            .collect(),
    )
    .unwrap();
    save_wav(&work.path("tone.wav"), &wave).unwrap();
    work.args(&["project", "new", "song.svp"]);
    work.args(&[
        "audio", "import", "song.svp", "--input", "tone.wav", "--name", "A",
    ]);
    work.args(&[
        "audio", "import", "song.svp", "--input", "tone.wav", "--name", "B", "--offset", "0.05",
    ]);
    let rendered = work.args(&[
        "render",
        "song.svp",
        "--output",
        "mix.wav",
        "--sample-rate",
        "48000",
        "--channels",
        "2",
        "--bit-depth",
        "32",
        "--compute-backend",
        "cpu",
    ]);
    assert_eq!(rendered["compute_backend_actual"], "idle");
    let analyzed = work.args(&["audio", "analyze", "mix.wav"]);
    let peak = analyzed["measurements"]["peak"][0].as_f64().unwrap();
    assert!((0.28..0.29).contains(&peak));
    let f0 = work.args(&["audio", "analyze", "tone.wav", "--f0"]);
    let hz = f0["estimated_f0"]["frames"][0]["frequency_hz"]
        .as_f64()
        .unwrap();
    assert!((437.0..443.0).contains(&hz), "{f0}");
    let result = work.args(&[
        "render",
        "song.svp",
        "--stems",
        "--output-dir",
        "stems",
        "--compute-backend",
        "cpu",
    ]);
    assert_eq!(result["outputs"].as_array().unwrap().len(), 2);
}

#[test]
#[ignore = "requires RYNTHV_TEST_SV1_ROOT and RYNTHV_TEST_DICTIONARIES with an installed Japanese SV1 model"]
fn native_project_renders_with_an_installed_voice() {
    let work = Workspace::new();
    let bank_root = std::env::var("RYNTHV_TEST_SV1_ROOT").unwrap();
    let dictionaries = std::env::var("RYNTHV_TEST_DICTIONARIES").unwrap();
    work.args(&[
        "config",
        "set",
        "--sv1-databases",
        &bank_root,
        "--dictionaries",
        &dictionaries,
        "--compute-backend",
        "cpu",
    ]);
    let catalog = work.args(&["voice", "list"]);
    let id = catalog["voices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["family"] == "Sv1"
                && entry["languages"]
                    .as_array()
                    .is_some_and(|items| items.contains(&json!("japanese")))
        })
        .or_else(|| catalog["voices"].as_array().unwrap().first())
        .expect("catalog has a voice")["database_id"]
        .as_str()
        .unwrap();
    work.args(&["project", "new", "song.svp"]);
    work.args(&[
        "voice",
        "assign",
        "song.svp",
        "--voice",
        id,
        "--language",
        "japanese",
    ]);
    work.args(&[
        "note",
        "add",
        "song.svp",
        "--at",
        "0",
        "--duration",
        "1",
        "--pitch",
        "C4",
        "--lyric",
        "あ",
    ]);
    check_render_cancellation(&work, &bank_root, &dictionaries);
    for backend in std::env::var("RYNTHV_TEST_BACKENDS")
        .unwrap_or_else(|_| "cpu".into())
        .split(',')
    {
        let filename = format!("{backend}.wav");
        let rendered = work.args(&[
            "render",
            "song.svp",
            "--compute-backend",
            backend,
            "--output",
            &filename,
            "--sample-rate",
            "48000",
            "--bit-depth",
            "32",
            "--quiet",
        ]);
        let actual = rendered["compute_backend_actual"].as_str().unwrap();
        if backend == "cpu" {
            assert_eq!(actual, "cpu");
        } else {
            assert!(matches!(actual, "mixed" | "webgpu"));
        }
        let analyzed = work.args(&["audio", "analyze", &filename]);
        assert!(analyzed["measurements"]["peak"][0].as_f64().unwrap() > 0.0001);
        let pitch = work.args(&[
            "render",
            "analyze",
            "song.svp",
            "--compute-backend",
            backend,
            "--quiet",
        ]);
        assert!(
            !pitch["owners"][0]["predicted_pitch"]["values"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(pitch["sample_rate"], 48000);
        assert!(Path::new(&work.path(&filename)).exists());
    }
    #[cfg(feature = "gui")]
    if work.path("cpu.wav").is_file() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "export_through_editor_runtime"])
            .env("RYNTHV_PARITY_DIR", &work.0)
            .env("LOCALAPPDATA", &work.0)
            .env("XDG_CONFIG_HOME", &work.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            fs::read(work.path("cpu.wav")).unwrap(),
            fs::read(work.path("editor-export.wav")).unwrap(),
            "GUI export Runtime and offline CLI should produce the same float WAV"
        );
    }
}
