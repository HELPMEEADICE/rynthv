use super::*;

pub(super) fn media() -> Vec<Command> {
    vec![
        family(
            "tempo",
            "Read or edit the BPM map",
            [
                project_cmd("list", "List tempo markers in Blick", []),
                project_cmd(
                    "set",
                    "Insert or replace a BPM marker",
                    [
                        option("beat", "Quarter-note position").required(true),
                        option("bpm", "Beats per minute").required(true),
                    ],
                ),
                project_cmd(
                    "delete",
                    "Remove a non-initial BPM marker",
                    [option("beat", "Quarter-note position").required(true)],
                ),
            ],
        ),
        family(
            "timesig",
            "Read or edit meter changes",
            [
                project_cmd("list", "List meter markers using one-based bar numbers", []),
                project_cmd(
                    "set",
                    "Insert or replace a meter marker",
                    [
                        option("bar", "One-based bar number").required(true),
                        value().required(true),
                    ],
                ),
                project_cmd(
                    "delete",
                    "Remove a non-initial meter marker",
                    [option("bar", "One-based bar number").required(true)],
                ),
            ],
        ),
        family(
            "midi",
            "Inspect Standard MIDI files and import or export note tracks",
            [
                Command::new("inspect")
                    .about("Read notes, lyrics, tracks and time map from a MIDI file")
                    .arg(positional("source", "MIDI file")),
                project_cmd(
                    "import",
                    "Import MIDI lanes into an SVP; optionally use the MIDI time map",
                    [
                        input().required(true),
                        option("tracks", "Comma-separated zero-based MIDI track indices"),
                        flag(
                            "time-map",
                            "Replace project BPM and meter with MIDI markers",
                        ),
                    ],
                ),
                project_cmd(
                    "export",
                    "Write project notes, lyrics and time markers as MIDI",
                    [output().required(true), track()],
                ),
            ],
        ),
        family(
            "audio",
            "Import and inspect decoded audio",
            [
                project_cmd(
                    "import",
                    "Add an accompaniment clip without copying the audio file",
                    [
                        input().required(true),
                        name(),
                        track(),
                        option("offset", "Project start in seconds"),
                    ],
                ),
                project_cmd(
                    "set",
                    "Change a clip's project offset, source offset or duration",
                    [
                        track().required(true),
                        option("offset", "Project start in seconds"),
                        option("source-offset", "Source start in seconds"),
                        option("duration", "Visible duration in seconds"),
                    ],
                ),
                Command::new("analyze")
                    .about("Measure native audio sample rate, duration, RMS, peak and silence")
                    .arg(positional("source", "Audio file"))
                    .arg(flag(
                        "f0",
                        "Estimate the dominant monophonic F0 from decoded audio",
                    )),
            ],
        ),
        Command::new("render")
            .about("Synthesize selected SVP content and export WAV")
            .arg(positional("project", "SVP project path").required(false))
            .args(render_args())
            .args_conflicts_with_subcommands(true)
            .subcommand(project_cmd(
                "analyze",
                "Render in memory and report engine pitch, phone timing and audio levels",
                render_args(),
            )),
        project_cmd(
            "export",
            "Synthesize SVP content and export WAV",
            render_args(),
        ),
    ]
}
