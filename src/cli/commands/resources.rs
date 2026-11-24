use super::*;

pub(super) fn resources() -> Vec<Command> {
    vec![
        family(
            "voice",
            "Discover, inspect and assign installed voicebanks",
            [
                Command::new("list").about("List catalog entries without loading weights"),
                Command::new("scan").about("Refresh the voice and Flat model registries"),
                Command::new("inspect")
                    .about("Read actual voice metadata and model resources")
                    .args(voice_key()),
                Command::new("validate")
                    .about("Prepare voice models and validate saved context")
                    .args(voice_key()),
                Command::new("modes")
                    .about("Read styles, supported languages and phoneme inventory")
                    .args(voice_key()),
                Command::new("import")
                    .about("Import a Flat voice package into the configured registry")
                    .arg(input().required(true)),
                Command::new("context")
                    .about("Import an existing portable model context for an installed voice")
                    .args(voice_key())
                    .arg(input().required(true)),
                project_cmd(
                    "assign",
                    "Assign an installed voice to a track or group reference",
                    tuning([
                        option("voice", "Catalog database ID").required(true),
                        option("revision", "Voice revision"),
                        language(),
                    ]),
                ),
            ],
        ),
        family(
            "dictionary",
            "Query native dictionaries and manage project pronunciations",
            [
                Command::new("list").about("List configured language dictionaries"),
                Command::new("lookup")
                    .about("Resolve a word through the engine language frontend")
                    .args([
                        language().required(true),
                        option("word", "Word or reading to resolve").required(true),
                    ]),
                project_cmd("project", "List the custom dictionaries in an SVP", []),
                project_cmd(
                    "set",
                    "Create a project dictionary, set an entry and optionally assign it",
                    tuning([
                        name().required(true),
                        language().required(true),
                        option("word", "Word to override"),
                        option("phones", "Space-separated phones"),
                        flag("assign", "Assign to the selected instance"),
                    ]),
                ),
                project_cmd(
                    "delete",
                    "Delete a project dictionary and clear its assignments",
                    [name().required(true)],
                ),
            ],
        ),
        family(
            "config",
            "Edit shared resource paths and inference defaults",
            [
                Command::new("show").about("Show current synthesis settings and source path"),
                Command::new("set")
                    .about("Set synthesis configuration without starting the GUI")
                    .args(
                        [
                            "databases",
                            "sv1-databases",
                            "flat-databases",
                            "flat-models",
                            "dictionaries",
                        ]
                        .map(|key| {
                            option(key, "Existing resource directory or supported archive path")
                        }),
                    )
                    .args([
                        choice("compute-backend", "Default inference backend", BACKENDS),
                        option("compute-adapter", "Default WebGPU adapter ID, or auto"),
                    ]),
            ],
        ),
    ]
}
fn voice_key() -> [Arg; 2] {
    [
        positional("voice", "Catalog database ID"),
        option("revision", "Specific voice revision"),
    ]
}
