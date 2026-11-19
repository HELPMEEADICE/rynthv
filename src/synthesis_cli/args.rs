use rynthv::synthesis::{inference::compute::Backend, settings::SynthesisSettings};
use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

use super::context::ContextImport;

pub const HELP: &str = r#"RynthV synthesis tools

  rynthv-synth render-score --input SCORE.json --wav OUTPUT.wav [OPTIONS]
  rynthv-synth list-compute-devices [--output REPORT.json]
  rynthv-synth inspect-voice [OPTIONS]
  rynthv-synth prepare-voice [OPTIONS]
  rynthv-synth import-context --context-file CONTEXT.json [OPTIONS]
  rynthv-synth import-context --identity U64 [OPTIONS]
  rynthv-synth import-context --process-id PID [OPTIONS] (Windows x64 only)
  rynthv-synth import-context --session-file PATH [OPTIONS] (Windows x64 only)
  rynthv-synth plan-project --project SVP [OPTIONS]
  rynthv-synth run-module --file INDEX --tree INDEX --node INDEX --input JSON [OPTIONS]
  rynthv-synth resolve-phone --file INDEX --tree INDEX --language NAME --phone TOKEN [OPTIONS]
  rynthv-synth inspect-flat --input PACKAGE.sfpk|BATCH.sfpks [OPTIONS]
  rynthv-synth validate-flat --input PACKAGE.sfpk [OPTIONS]
  rynthv-synth import-flat --input PACKAGE.sfpk|BATCH.sfpks [OPTIONS]
  rynthv-synth list-flat-models [OPTIONS]

OPTIONS: --voice PATH --output REPORT.json
Omitting --voice uses the voice selected in the saved RynthV synthesis settings.
--voice names an exact SV2 revision directory, an original SV1 .nofs file or a Flat .sfpk.
A native SV1 voice directory with a single .nofs file is also accepted.
Flat options: --flat-dir PATH --model-dir PATH --output REPORT.json
Flat commands use saved directories when omitted. Import defaults to the RynthV
configuration directory's flat-voices folder. Exit codes: 0 success, 1 operation
failed, 2 invalid arguments.
import-context saves a model condition for the selected SV2 or SV2-compatible
voice. Provide exactly one source: --context-file reads a RynthV context file;
--identity accepts a decimal or 0x-prefixed hexadecimal U64. Both work on all
supported platforms. --process-id reads an already-running supported SV2 online
session on Windows x64. --session-file reads an explicitly supplied SV2 session
file on Windows x64 without requiring a running editor. A file path is required;
no default directory is searched. Contexts are imported only on request; loading
a voice reads only the saved RynthV context. Native SV1 voices do not need model
contexts.
Only the matching voice and model generation are imported; raw session data and
model-condition values are not printed in the report.
render-score and run-module: --compute-backend auto|cpu|webgpu (default auto)
render-score: --repeat N (measure warm renders with the same prepared voice)
              --profile FILE (requires a build with --features synth-profile)
              --blend-voice VOICE.sfpk --blend-weight 0..1 (Flat only,
              both voices must use the same acoustic model)
              --duration-identity U64 (explicit research conditioning;
              otherwise uses the saved RynthV context;
              not used by original SV1 models)

render-score takes explicit note/phone records and writes native float32 WAV.
See examples/synthesis/mandarin-phrase.json for the score format.
run-module takes a flat frame-major f32 JSON array and returns intermediate
model values, not audio. Indices are zero-based.
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Inspect,
    Prepare,
    ImportContext,
    Plan,
    RunModule,
    ResolvePhone,
    RenderScore,
}

pub struct Args {
    pub command: Command,
    pub voice: PathBuf,
    pub output: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub module: Option<ModuleArgs>,
    pub phone: Option<PhoneArgs>,
    pub render: Option<RenderArgs>,
    pub context: Option<ContextImport>,
    pub flat_directory: Option<PathBuf>,
    pub model_directory: Option<PathBuf>,
}

pub struct RenderArgs {
    pub input: PathBuf,
    pub wav: PathBuf,
    pub duration_identity: Option<u64>,
    pub blend: Option<BlendArgs>,
    pub backend: Backend,
    pub repeat: NonZeroUsize,
    #[cfg(feature = "synth-profile")]
    pub profile: Option<PathBuf>,
}

pub struct BlendArgs {
    pub voice: PathBuf,
    pub weight: f32,
}

pub struct PhoneArgs {
    pub file: usize,
    pub tree: usize,
    pub language: String,
    pub token: String,
}

pub struct ModuleArgs {
    pub file: usize,
    pub tree: usize,
    pub node: usize,
    pub input: PathBuf,
    pub backend: Backend,
}

impl Args {
    pub fn parse(
        command: OsString,
        mut args: impl Iterator<Item = OsString>,
    ) -> Result<Option<Self>, String> {
        let command = match command.to_str() {
            Some("--help" | "-h") => return Ok(None),
            Some("inspect-voice") => Command::Inspect,
            Some("prepare-voice") => Command::Prepare,
            Some("import-context") => Command::ImportContext,
            Some("plan-project") => Command::Plan,
            Some("run-module") => Command::RunModule,
            Some("resolve-phone") => Command::ResolvePhone,
            Some("render-score") => Command::RenderScore,
            _ => {
                return Err(format!(
                    "Unknown command: {}\n{HELP}",
                    command.to_string_lossy()
                ));
            }
        };
        let mut values = std::collections::BTreeMap::new();
        while let Some(option) = args.next() {
            let key = option
                .to_str()
                .filter(|key| {
                    [
                        "--voice",
                        "--process-id",
                        "--session-file",
                        "--context-file",
                        "--identity",
                        "--output",
                        "--project",
                        "--file",
                        "--tree",
                        "--node",
                        "--input",
                        "--language",
                        "--phone",
                        "--wav",
                        "--duration-identity",
                        "--profile",
                        "--flat-dir",
                        "--model-dir",
                        "--blend-voice",
                        "--blend-weight",
                        "--compute-backend",
                        "--repeat",
                    ]
                    .contains(key)
                })
                .ok_or_else(|| format!("Unknown option: {}", option.to_string_lossy()))?;
            if values.contains_key(key) {
                return Err(format!("Repeated option: {key}"));
            }
            let value = args
                .next()
                .filter(|value| !value.to_string_lossy().starts_with("--"))
                .ok_or_else(|| format!("Missing value for {key}"))?;
            values.insert(key.to_owned(), value);
        }
        let context = if command == Command::ImportContext {
            Some(ContextImport::parse(&mut values)?)
        } else {
            None
        };
        let voice = match values.remove("--voice") {
            Some(path) => PathBuf::from(path),
            None => SynthesisSettings::load()?.selected_directory()?,
        };
        let output = values.remove("--output").map(PathBuf::from);
        let flat_directory = values.remove("--flat-dir").map(PathBuf::from);
        let model_directory = values.remove("--model-dir").map(PathBuf::from);
        if (flat_directory.is_some() || model_directory.is_some())
            && !rynthv::synthesis::voice::flat::is_voice_path(&voice)
        {
            return Err("--flat-dir and --model-dir require a Flat .sfpk voice".into());
        }
        let project = if command == Command::Plan {
            Some(PathBuf::from(
                values
                    .remove("--project")
                    .ok_or("plan-project requires --project SVP")?,
            ))
        } else {
            None
        };
        let module = if command == Command::RunModule {
            Some(ModuleArgs {
                file: index(values.remove("--file"), "--file")?,
                tree: index(values.remove("--tree"), "--tree")?,
                node: index(values.remove("--node"), "--node")?,
                input: PathBuf::from(
                    values
                        .remove("--input")
                        .ok_or("run-module requires --input JSON")?,
                ),
                backend: backend(values.remove("--compute-backend"))?,
            })
        } else {
            None
        };
        let phone = if command == Command::ResolvePhone {
            Some(PhoneArgs {
                file: index(values.remove("--file"), "--file")?,
                tree: index(values.remove("--tree"), "--tree")?,
                language: required_text(values.remove("--language"), "--language")?,
                token: required_text(values.remove("--phone"), "--phone")?,
            })
        } else {
            None
        };
        let render = if command == Command::RenderScore {
            let blend = match (
                values.remove("--blend-voice"),
                values.remove("--blend-weight"),
            ) {
                (None, None) => None,
                (Some(path), Some(weight)) => {
                    let path = PathBuf::from(path);
                    if !rynthv::synthesis::voice::flat::is_voice_path(&voice)
                        || !rynthv::synthesis::voice::flat::is_voice_path(&path)
                    {
                        return Err("Flat blending requires two .sfpk voices".into());
                    }
                    let weight = weight
                        .to_str()
                        .and_then(|value| value.parse::<f32>().ok())
                        .filter(|weight| (0.0..=1.0).contains(weight))
                        .ok_or("--blend-weight requires a numeric value between 0 and 1")?;
                    Some(BlendArgs {
                        voice: path,
                        weight,
                    })
                }
                _ => {
                    return Err("--blend-voice and --blend-weight must be provided together".into());
                }
            };
            let profile = values.remove("--profile").map(PathBuf::from);
            if profile.is_some() && !cfg!(feature = "synth-profile") {
                return Err("--profile requires cargo build --release --features synth-profile --bin rynthv-synth".into());
            }
            let duration_identity = values
                .remove("--duration-identity")
                .map(|value| {
                    let text = value.to_str().ok_or("Identity must be UTF-8")?;
                    if let Some(hex) = text.strip_prefix("0x") {
                        u64::from_str_radix(hex, 16)
                    } else {
                        text.parse()
                    }
                    .map_err(|_| "Identity must be a u64 decimal or 0x-prefixed hexadecimal number")
                })
                .transpose()?;
            Some(RenderArgs {
                input: PathBuf::from(
                    values
                        .remove("--input")
                        .ok_or("render-score requires --input JSON")?,
                ),
                wav: PathBuf::from(
                    values
                        .remove("--wav")
                        .ok_or("render-score requires --wav FILE")?,
                ),
                duration_identity,
                blend,
                backend: backend(values.remove("--compute-backend"))?,
                repeat: match values.remove("--repeat") {
                    Some(value) => value
                        .to_str()
                        .and_then(|text| text.parse().ok())
                        .ok_or("--repeat requires a positive integer")?,
                    None => NonZeroUsize::MIN,
                },
                #[cfg(feature = "synth-profile")]
                profile,
            })
        } else {
            None
        };
        if !values.is_empty() {
            return Err(format!(
                "Options are not valid for this command: {}",
                values.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(Some(Self {
            command,
            voice,
            output,
            project,
            module,
            phone,
            render,
            context,
            flat_directory,
            model_directory,
        }))
    }
}

fn backend(value: Option<OsString>) -> Result<Backend, String> {
    match value.as_ref().map(|value| value.to_str()) {
        None | Some(Some("auto")) => Ok(Backend::Auto),
        Some(Some("cpu")) => Ok(Backend::Cpu),
        Some(Some("webgpu")) => Ok(Backend::WebGpu),
        _ => Err("--compute-backend requires auto, cpu or webgpu".into()),
    }
}

fn index(value: Option<OsString>, name: &str) -> Result<usize, String> {
    value
        .and_then(|value| value.to_str().and_then(|text| text.parse().ok()))
        .ok_or_else(|| format!("This command requires {name} with a nonnegative index"))
}

fn required_text(value: Option<OsString>, name: &str) -> Result<String, String> {
    value
        .and_then(|value| value.into_string().ok())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("resolve-phone requires nonempty UTF-8 {name}"))
}
