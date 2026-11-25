mod commands;
mod media;
mod notes;
mod project;
mod resources;
mod session;
mod target;
mod tracks;
mod tuning;

use clap::{ArgMatches, error::ErrorKind};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
};

const SCHEMA_VERSION: u32 = 1;
const MAX_INPUT: u64 = 64 * 1024 * 1024;

type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
    details: Value,
}
impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: json!({}),
        }
    }
    fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    fn exit(&self) -> u8 {
        match self.code {
            "USAGE" | "INPUT_INVALID" | "LYRIC_COUNT" => 2,
            "PROJECT_CONFLICT" | "OUTPUT_EXISTS" | "UNSAVED_CHANGES" => 4,
            "CANCELLED" => 130,
            _ => 3,
        }
    }
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        let code = match message.split_once(": ").map(|(prefix, _)| prefix) {
            Some("PROJECT_CONFLICT") => "PROJECT_CONFLICT",
            Some("OUTPUT_EXISTS") => "OUTPUT_EXISTS",
            Some("VOICE_NOT_FOUND") => "VOICE_NOT_FOUND",
            Some("VOICE_NOT_ASSIGNED") => "VOICE_NOT_ASSIGNED",
            Some("MODEL_CONTEXT_REQUIRED") => "MODEL_CONTEXT_REQUIRED",
            Some("DICTIONARY_REQUIRED") => "DICTIONARY_REQUIRED",
            Some("DICTIONARY_RESOURCE") => "DICTIONARY_RESOURCE",
            Some("VOICE_RESOURCE") => "VOICE_RESOURCE",
            Some("AUDIO_RESOURCE") => "AUDIO_RESOURCE",
            Some("COMPUTE_UNAVAILABLE") => "COMPUTE_UNAVAILABLE",
            Some("SYNTHESIS_FAILED") => "SYNTHESIS_FAILED",
            Some("CANCELLED") => "CANCELLED",
            _ => "OPERATION_FAILED",
        };
        let message = if code == "OPERATION_FAILED" {
            message
        } else {
            message
                .split_once(": ")
                .map(|(_, text)| text.to_owned())
                .unwrap_or(message)
        };
        Self::new(code, message)
    }
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("INPUT_INVALID", message)
}

#[derive(Clone, Copy)]
struct Call<'a> {
    root: &'a ArgMatches,
    args: &'a ArgMatches,
    family: &'a str,
    action: &'a str,
}
impl<'a> Call<'a> {
    fn parse(root: &'a ArgMatches) -> Option<Self> {
        let (family, args) = root.subcommand()?;
        let (action, args) = args.subcommand().unwrap_or(("", args));
        Some(Self {
            root,
            args,
            family,
            action,
        })
    }
    fn name(self) -> String {
        if self.action.is_empty() {
            self.family.to_owned()
        } else {
            format!("{}.{}", self.family, self.action)
        }
    }
    fn get(self, name: &str) -> Option<&'a str> {
        self.args
            .try_get_one::<String>(name)
            .ok()
            .flatten()
            .map(String::as_str)
    }
    fn flag(self, name: &str) -> bool {
        self.args
            .try_get_one::<bool>(name)
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false)
            || self
                .root
                .try_get_one::<bool>(name)
                .ok()
                .flatten()
                .copied()
                .unwrap_or(false)
    }
    fn required(self, name: &str) -> Result<&'a str> {
        self.get(name)
            .ok_or_else(|| invalid(format!("{} requires {name}", self.name())))
    }
    fn number<T: std::str::FromStr>(self, key: &str) -> Result<Option<T>> {
        self.get(key)
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| invalid(format!("Invalid {key}: {value}")))
            })
            .transpose()
    }
    fn path(self, name: &str) -> Result<&'a Path> {
        self.required(name).map(Path::new)
    }
    fn mutation(self) -> bool {
        match (self.family, self.action) {
            ("track", action) | ("group", action) => action != "list",
            ("note", action) => !matches!(action, "list" | "overlaps"),
            ("tempo" | "timesig" | "retake" | "lyrics", action) => action != "list",
            ("phoneme", "set" | "adjust")
            | ("pitch", "import")
            | ("voice", "assign")
            | ("parameter", "set" | "curve" | "point" | "transform")
            | ("dictionary", "set" | "delete")
            | ("midi", "import")
            | ("audio", "import" | "set") => true,
            ("parameter", "note" | "voice") => self.get("input").is_some(),
            _ => false,
        }
    }
    fn writes_output(self) -> bool {
        matches!(
            (self.family, self.action),
            ("render" | "export", "")
                | ("midi" | "phoneme" | "pitch", "export")
                | ("project", "new" | "clone")
                | ("voice", "context")
        )
    }
    fn supports_dry_run(self) -> bool {
        self.mutation()
            || self.writes_output()
            || matches!(
                (self.family, self.action),
                ("project" | "batch", "apply")
                    | ("project", "save")
                    | ("config", "set")
                    | ("voice" | "dictionary", "import")
            )
    }
}

struct Workstation {
    open: Option<project::OpenProject>,
    offline: crate::synthesis::offline::Offline,
    cancellation: Arc<AtomicBool>,
    streaming: bool,
    request_input: Option<Value>,
}
impl Workstation {
    fn new() -> Self {
        Self {
            open: None,
            offline: Default::default(),
            cancellation: Default::default(),
            streaming: false,
            request_input: None,
        }
    }

    fn execute(&mut self, call: Call<'_>) -> Result<Value> {
        if call.flag("dry-run") && !call.supports_dry_run() {
            return Err(invalid(format!(
                "--dry-run does not apply to {}",
                call.name()
            )));
        }
        if call.flag("overwrite") && !call.writes_output() {
            return Err(invalid(format!(
                "--overwrite does not apply to {}",
                call.name()
            )));
        }
        if call.get("expect-revision").is_some()
            && !call.mutation()
            && !matches!(
                (call.family, call.action),
                ("project" | "batch", "apply") | ("project", "save")
            )
        {
            return Err(invalid(format!(
                "--expect-revision does not apply to {}",
                call.name()
            )));
        }
        match (call.family, call.action) {
            ("capabilities", _) => self.capabilities(),
            ("schema", _) => self.schema(call.get("command")),
            ("doctor", _) => self.doctor(call.flag("probe-webgpu")),
            ("project", _) | ("batch", _) => self.project_command(call),
            ("config", _)
            | (
                "voice",
                "list" | "scan" | "inspect" | "validate" | "modes" | "context" | "import",
            )
            | ("dictionary", "list" | "lookup" | "import") => self.resource_command(call),
            ("midi", "inspect") | ("audio", "analyze") => self.media_query(call),
            ("render" | "export", _) => self.render_command(call),
            _ if call.mutation() => self.mutate(call),
            _ => self.project_query(call),
        }
    }

    fn read(&self, name: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let read = if name == "-" {
            if self.streaming {
                return match &self.request_input {
                    Some(Value::String(text)) => Ok(text.as_bytes().to_vec()),
                    Some(value) => {
                        serde_json::to_vec(value).map_err(|error| invalid(error.to_string()))
                    }
                    None => Err(invalid(
                        "Supply the JSON Lines request input field with --input -",
                    )),
                };
            }
            if io::stdin().is_terminal() {
                return Err(invalid("Pipe input to stdin or provide a file"));
            }
            io::stdin().take(MAX_INPUT + 1).read_to_end(&mut bytes)
        } else {
            std::fs::File::open(name)
                .and_then(|file| file.take(MAX_INPUT + 1).read_to_end(&mut bytes))
        };
        read.map_err(|error| Failure::new("INPUT_READ", format!("{name}: {error}")))?;
        if bytes.len() as u64 > MAX_INPUT {
            return Err(invalid("Input exceeds 64 MiB"));
        }
        Ok(bytes)
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T> {
        serde_json::from_slice(&self.read(name)?)
            .map_err(|error| invalid(format!("{name}: {error}")))
    }
}

fn json_file(path: &Path, value: &Value, call: Call<'_>) -> Result<()> {
    separate_from_source(call.path("project")?, path)?;
    check_output(path, call.flag("overwrite"))?;
    if !call.flag("dry-run") {
        crate::storage::atomic_write_policy(path, call.flag("overwrite"), |writer| {
            serde_json::to_writer_pretty(writer, value).map_err(|error| error.to_string())
        })?;
    }
    Ok(())
}

fn separate_from_source(source: &Path, output: &Path) -> Result<()> {
    if source == output
        || std::fs::canonicalize(source)
            .ok()
            .zip(std::fs::canonicalize(output).ok())
            .is_some_and(|(source, output)| source == output)
    {
        return Err(invalid(
            "Output must be a separate file from the source project",
        ));
    }
    Ok(())
}

fn check_output(path: &Path, overwrite: bool) -> Result<()> {
    if !overwrite && path.symlink_metadata().is_ok() {
        return Err(Failure::new(
            "OUTPUT_EXISTS",
            "Output already exists. Specify --overwrite to replace it.",
        )
        .details(json!({"path": path})));
    }
    Ok(())
}

fn envelope(command: &str, result: &Result<Value>) -> Value {
    match result {
        Ok(result) => {
            json!({"schema_version": SCHEMA_VERSION, "ok": true, "command": command, "result": result,
                "warnings":result.get("warnings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()})
        }
        Err(error) => json!({"schema_version": SCHEMA_VERSION, "ok": false, "command": command,
            "error": {"code": error.code, "message": error.message, "details": error.details}, "warnings": []}),
    }
}

fn emit(command: &str, result: Result<Value>, json: bool, quiet: bool) -> ExitCode {
    if json {
        let out = io::stdout();
        let mut output = out.lock();
        if serde_json::to_writer(&mut output, &envelope(command, &result)).is_err()
            || writeln!(output).is_err()
        {
            return ExitCode::FAILURE;
        }
    } else {
        match &result {
            Ok(result) if !quiet => println!(
                "{}",
                serde_json::to_string_pretty(result).expect("JSON value")
            ),
            Err(error) => eprintln!("{}: {}", error.code, error.message),
            _ => {}
        }
    }
    ExitCode::from(result.as_ref().err().map_or(0, Failure::exit))
}

pub fn run(args: impl Iterator<Item = OsString>) -> ExitCode {
    let args: Vec<_> = std::iter::once("rynthv -cli".into()).chain(args).collect();
    let wants_json = args.iter().any(|arg| arg == "--json");
    let parsed = match commands::command().try_get_matches_from(args) {
        Ok(parsed) => parsed,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            return emit(
                "",
                Err(Failure::new("USAGE", error.to_string())),
                wants_json,
                false,
            );
        }
    };
    let mut workstation = Workstation::new();
    if parsed.get_flag("stdio") {
        return workstation.stdio();
    }
    let Some(call) = Call::parse(&parsed) else {
        if !parsed.get_flag("non-interactive")
            && !parsed.get_flag("json")
            && !parsed.get_flag("quiet")
            && io::stdin().is_terminal()
            && io::stdout().is_terminal()
        {
            return workstation.repl();
        }
        if parsed.get_flag("json") {
            return emit(
                "",
                Err(Failure::new("USAGE", "Specify a command or --stdio")),
                true,
                false,
            );
        }
        let _ = commands::command().print_help();
        return ExitCode::SUCCESS;
    };
    if matches!(call.family, "render" | "export" | "batch" | "audio") {
        workstation.install_interrupt();
    }
    let name = call.name();
    let result = workstation.execute(call);
    emit(&name, result, call.flag("json"), call.flag("quiet"))
}
