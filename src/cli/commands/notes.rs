use super::*;

pub(super) fn note_and_tuning() -> Vec<Command> {
    let mut change = note_edit();
    change.push(shared());
    let mut split = vec![at().required(true), shared()];
    split.extend(target());
    vec![
        family(
            "note",
            "Create and edit timed vocal notes",
            [
                project_cmd(
                    "list",
                    "List visible notes with project and local timing",
                    target(),
                ),
                project_cmd(
                    "add",
                    "Insert a note at the requested position",
                    notes(
                        note_edit()
                            .into_iter()
                            .map(|arg| {
                                if ["at", "duration", "pitch"].contains(&arg.get_id().as_str()) {
                                    arg.required(true)
                                } else {
                                    arg
                                }
                            })
                            .chain([shared()])
                            .collect(),
                    ),
                ),
                project_cmd(
                    "import",
                    "Insert a JSON array of notes with explicit times and pitches",
                    [track(), group(), unit(), shared(), input().required(true)],
                ),
                project_cmd(
                    "set",
                    "Set selected note properties; --at also moves a note",
                    notes(change.clone()),
                ),
                project_cmd(
                    "move",
                    "Move one note to a new absolute position",
                    notes(vec![at().required(true), shared()]),
                ),
                project_cmd(
                    "copy",
                    "Copy selected notes in-place or to a new starting position",
                    notes(vec![at(), shared()]),
                ),
                project_cmd("delete", "Delete selected notes", notes(vec![shared()])),
                project_cmd(
                    "transpose",
                    "Transpose filtered notes by a signed semitone count",
                    notes(vec![
                        option("semitones", "Signed semitone change").required(true),
                        shared(),
                    ]),
                ),
                project_cmd(
                    "quantize",
                    "Snap both boundaries to a whole-note fraction grid",
                    notes(vec![
                        option("grid", "Grid as a fraction, e.g. 1/16").required(true),
                        shared(),
                    ]),
                ),
                project_cmd("split", "Split one note and continue its syllable", split),
                project_cmd(
                    "merge",
                    "Extend the first of the selected same-pitch overlapping or adjacent notes",
                    notes(vec![shared()]),
                ),
                project_cmd(
                    "overlaps",
                    "Report overlapping selected notes in each instance",
                    target(),
                ),
            ],
        ),
        family(
            "lyrics",
            "Inspect, assign and resolve vocal lyrics",
            [
                project_cmd("list", "Show lyrics mapped to note UUIDs", target()),
                project_cmd(
                    "fill",
                    "Assign one lyric token per selected note",
                    notes(vec![
                        language(),
                        shared(),
                        option(
                            "text",
                            "Whitespace-separated syllables, or text segmented by --language",
                        )
                        .conflicts_with("input"),
                        input(),
                        flag(
                            "syllabify",
                            "Use native language dictionaries and store resolved phoneme overrides",
                        ),
                        flag(
                            "literal",
                            "Assign explicit whitespace-separated lyric tokens",
                        )
                        .conflicts_with("syllabify"),
                    ]),
                ),
            ],
        ),
        family(
            "phoneme",
            "Read and edit pronunciation and per-phone attributes",
            [
                project_cmd(
                    "inspect",
                    "Inspect automatic and overridden pronunciation for a note",
                    target(),
                ),
                project_cmd(
                    "export",
                    "Export automatic and overridden pronunciation to JSON",
                    notes(vec![output().required(true)]),
                ),
                project_cmd(
                    "set",
                    "Set or clear the phoneme override for one note",
                    notes(vec![value().required(true), shared()]),
                ),
                project_cmd(
                    "adjust",
                    "Set per-phone timing and articulation attributes from a JSON object",
                    notes(vec![
                        option("index", "Zero-based index in the source phoneme sequence")
                            .required(true),
                        input().required(true),
                        shared(),
                    ]),
                ),
            ],
        ),
        family(
            "pitch",
            "Read and edit score pitch data in cents and local Blick",
            [
                project_cmd(
                    "inspect",
                    "Show pitch deviation, system pitch and pitch controls",
                    curve_scope(),
                ),
                project_cmd(
                    "export",
                    "Export pitch controls and pitch deviation as JSON",
                    tuning([output().required(true)]),
                ),
                project_cmd(
                    "import",
                    "Apply pitch controls and pitch deviation from JSON",
                    tuning([input().required(true)]),
                ),
            ],
        ),
        family(
            "parameter",
            "Inspect or edit automation, note attributes and vocal modes",
            [
                project_cmd(
                    "list",
                    "List curves, units and SVP editor limits",
                    curve_scope(),
                ),
                project_cmd(
                    "set",
                    "Replace a curve with a constant",
                    tuning([name().required(true), value().required(true)]),
                ),
                project_cmd(
                    "curve",
                    "Replace a curve with a JSON array of [local Blick, value] points",
                    tuning([name().required(true), input().required(true)]),
                ),
                project_cmd(
                    "point",
                    "Set or remove one control point in local Blick",
                    tuning([
                        name().required(true),
                        at().required(true),
                        value().conflicts_with("delete"),
                        flag("delete", "Remove the point"),
                    ]),
                ),
                project_cmd(
                    "transform",
                    "Scale, offset, smooth or reset the selected part of a curve",
                    tuning([
                        name().required(true),
                        from(),
                        to(),
                        option("scale", "Numeric multiplier"),
                        option("offset", "Additive offset in the parameter unit"),
                        option("smooth", "Smoothing passes"),
                        flag("reset", "Remove points in the requested range"),
                    ]),
                ),
                project_cmd(
                    "note",
                    "Inspect or merge note performance attributes from a JSON object",
                    notes(vec![input(), shared()]),
                ),
                project_cmd(
                    "voice",
                    "Inspect or set an instance's voice settings or vocal modes from JSON",
                    tuning([input()]),
                ),
            ],
        ),
        family(
            "retake",
            "Inspect and manage note retake seeds",
            [
                project_cmd(
                    "list",
                    "List retakes and current selection for the selected note",
                    target(),
                ),
                project_cmd(
                    "create",
                    "Create and select a retake for one note",
                    notes(vec![
                        choice(
                            "component",
                            "Which retake seeds to regenerate",
                            &["pitch", "timbre", "pitch-and-timbre", "duration", "all"],
                        )
                        .default_value("pitch-and-timbre"),
                        option(
                            "seed",
                            "Set the chosen components to this unsigned 32-bit seed",
                        ),
                        shared(),
                    ]),
                ),
                project_cmd(
                    "select",
                    "Set the active retake ID for a note",
                    notes(vec![
                        option("id", "Retake numeric ID").required(true),
                        shared(),
                    ]),
                ),
                project_cmd(
                    "prune",
                    "Keep the default and active retakes",
                    notes(vec![shared()]),
                ),
            ],
        ),
    ]
}
