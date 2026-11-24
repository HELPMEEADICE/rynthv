mod media;
mod notes;
mod project;
mod resources;

use clap::{Arg, ArgAction, Command, builder::PossibleValuesParser};
use serde_json::{Value, json};

fn option(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help)
        .allow_negative_numbers(true)
}

fn flag(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help)
        .action(ArgAction::SetTrue)
}

fn choice(name: &'static str, help: &'static str, values: &'static [&'static str]) -> Arg {
    option(name, help).value_parser(PossibleValuesParser::new(values))
}

fn positional(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).help(help).required(true)
}

fn family(
    name: &'static str,
    help: &'static str,
    commands: impl IntoIterator<Item = Command>,
) -> Command {
    Command::new(name)
        .about(help)
        .subcommand_required(true)
        .subcommands(commands)
}

fn project_cmd(
    name: &'static str,
    help: &'static str,
    args: impl IntoIterator<Item = Arg>,
) -> Command {
    Command::new(name)
        .about(help)
        .arg(positional("project", "SVP project path"))
        .args(args)
}

const UNITS: &[&str] = &["blick", "beat", "second", "bar"];
const LANGUAGES: &[&str] = &[
    "english",
    "mandarin",
    "japanese",
    "spanish",
    "cantonese",
    "korean",
];
const BACKENDS: &[&str] = &["auto", "cpu", "webgpu"];

fn unit() -> Arg {
    choice(
        "unit",
        "Timing unit. Beat is a quarter note. Bar positions are BAR:BEAT with one-based indices.",
        UNITS,
    )
    .default_value("beat")
}

fn language() -> Arg {
    choice("language", "Pronunciation language", LANGUAGES)
}
fn track() -> Arg {
    option(
        "track",
        "Track UUID from track list or an unambiguous track name",
    )
}
fn group() -> Arg {
    option("group", "Group reference UUID on the selected track")
}
fn note() -> Arg {
    option("note", "Persistent note UUID returned by note list")
}
fn at() -> Arg {
    option("at", "Absolute project position in --unit")
}
fn duration() -> Arg {
    option(
        "duration",
        "Duration in --unit, relative to the note or group start",
    )
}
fn from() -> Arg {
    option("from", "Inclusive start in --unit")
}
fn to() -> Arg {
    option("to", "Exclusive end in --unit")
}
fn input() -> Arg {
    option("input", "Input file, or - to read standard input")
}
fn output() -> Arg {
    option("output", "Output file path")
}
fn value() -> Arg {
    option("value", "New value")
}
fn name() -> Arg {
    option("name", "Display name or parameter key")
}
fn shared() -> Arg {
    flag(
        "shared",
        "Edit the shared definition for all linked group references",
    )
}
fn target() -> Vec<Arg> {
    vec![
        track(),
        group(),
        note(),
        from(),
        to(),
        unit(),
        option("min-pitch", "Inclusive lower MIDI pitch or note name"),
        option("max-pitch", "Inclusive upper MIDI pitch or note name"),
    ]
}
fn notes(mut args: Vec<Arg>) -> Vec<Arg> {
    args.extend(target());
    args
}
fn note_edit() -> Vec<Arg> {
    vec![
        at(),
        duration(),
        option("pitch", "MIDI number or note name (C4, D#4, Bb4)"),
        option(
            "lyric",
            "Lyrics or engine marker (- or ー continuation, br breath)",
        ),
        option("phonemes", "Whitespace-separated phoneme override"),
        language(),
        choice("mute", "Mute this note", &["true", "false"]),
    ]
}
fn curve_scope() -> Vec<Arg> {
    vec![track(), group(), shared()]
}
fn tuning(args: impl IntoIterator<Item = Arg>) -> Vec<Arg> {
    curve_scope().into_iter().chain(args).collect()
}
fn render_args() -> Vec<Arg> {
    vec![
        track(),
        group(),
        from(),
        to(),
        unit(),
        output(),
        flag("stems", "Write one WAV for each selected audible track"),
        option("output-dir", "Directory for stems"),
        flag("vocals-only", "Exclude accompaniment tracks"),
        choice("compute-backend", "Model inference backend", BACKENDS),
        option(
            "compute-adapter",
            "Adapter ID returned by doctor --probe-webgpu",
        ),
        choice(
            "sample-rate",
            "WAV sample rate in Hz",
            &["44100", "48000", "96000"],
        ),
        choice("channels", "Number of WAV channels", &["1", "2"]),
        choice(
            "bit-depth",
            "WAV encoding: 16 or 24 for integer PCM, 32 for Float32",
            &["16", "24", "32"],
        ),
        option("cache-mib", "Soft acoustic cache limit between render jobs"),
        option(
            "seed",
            "Override each note's pitch and timbre retake seeds for this render",
        ),
    ]
}

pub(super) fn command() -> Command {
    Command::new("rynthv -cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Native headless SVP editing, vocal synthesis and audio export")
        .arg(flag("json", "Emit one versioned JSON result on stdout").global(true))
        .arg(
            flag(
                "quiet",
                "Suppress progress and human-readable success output",
            )
            .global(true),
        )
        .arg(flag("no-color", "Disable colored terminal output").global(true))
        .arg(
            flag(
                "non-interactive",
                "Disable the interactive shell and prompts",
            )
            .global(true),
        )
        .arg(
            flag(
                "dry-run",
                "Validate a supported write without publishing it",
            )
            .global(true),
        )
        .arg(flag("overwrite", "Allow replacing an existing output file").global(true))
        .arg(
            option(
                "expect-revision",
                "Reject a project edit unless its saved SHA256 matches",
            )
            .global(true),
        )
        .arg(
            flag(
                "stdio",
                "Serve sequential JSON Lines requests over standard I/O",
            )
            .exclusive(true),
        )
        .subcommands(project::project_and_tracks())
        .subcommands(notes::note_and_tuning())
        .subcommands(resources::resources())
        .subcommands(media::media())
        .subcommand(
            Command::new("capabilities").about("Discover command families and installed resources"),
        )
        .subcommand(
            Command::new("schema")
                .about("Describe CLI options and machine interfaces")
                .arg(Arg::new("command").help("Dotted command name, e.g. note.add")),
        )
        .subcommand(
            Command::new("doctor")
                .about("Check resource configuration and headless environment")
                .arg(flag(
                    "probe-webgpu",
                    "Enumerate compute adapters without a display surface",
                )),
        )
}
pub(super) fn schema(entry: &Command) -> Value {
    let args: Vec<_> = entry.get_arguments().filter(|arg| arg.get_id() != "help" && arg.get_id() != "version").map(|arg| {
        let choices: Vec<_> = arg.get_possible_values().iter().map(|item| item.get_name().to_owned()).collect();
        let defaults: Vec<_> = arg.get_default_values().iter().map(|item| item.to_string_lossy()).collect();
        json!({
            "id": arg.get_id().as_str(), "long": arg.get_long(), "position": arg.get_index(),
            "description": arg.get_help().map(ToString::to_string), "required": arg.is_required_set(),
            "type": if matches!(arg.get_action(), ArgAction::SetTrue) { "boolean" } else { "string" },
            "choices": choices, "defaults": defaults
        })
    }).collect();
    let subcommands: Vec<_> = entry
        .get_subcommands()
        .filter(|command| command.get_name() != "help")
        .map(schema)
        .collect();
    json!({"name": entry.get_name(), "description": entry.get_about().map(ToString::to_string),
        "arguments": args, "subcommands": subcommands})
}
