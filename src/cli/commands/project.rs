use super::*;

pub(super) fn project_and_tracks() -> Vec<Command> {
    vec![
        family(
            "project",
            "Create, inspect and edit SVP projects",
            [
                project_cmd(
                    "new",
                    "Create a default SVP project",
                    [
                        option("bpm", "Initial BPM"),
                        option("time-signature", "Initial meter as numerator/denominator"),
                    ],
                ),
                project_cmd(
                    "info",
                    "Show project identifiers, duration and track summaries",
                    [],
                ),
                project_cmd(
                    "inspect",
                    "Show the full normalized SVP document and identifiers",
                    [],
                ),
                project_cmd(
                    "validate",
                    "Validate project content and check resources",
                    [],
                ),
                project_cmd(
                    "clone",
                    "Save a separate SVP copy with a new project UUID",
                    [positional("destination", "Destination SVP path")],
                ),
                project_cmd(
                    "apply",
                    "Apply an ordered JSON edit transaction to a single project",
                    [input().required(true)],
                ),
                project_cmd("open", "Open a project in a REPL or stdio session", []),
                Command::new("save")
                    .about("Save the currently open project with revision checking"),
                Command::new("status").about("Show the current session project and selection"),
                Command::new("close")
                    .about("Close the current session project")
                    .arg(flag("discard", "Discard unsaved changes")),
                Command::new("undo").about("Undo the last session edit"),
                Command::new("redo").about("Redo the last undone session edit"),
                Command::new("select")
                    .about("Set the current session track and group")
                    .args([track().required(true), group()]),
            ],
        ),
        family(
            "batch",
            "Run a single-project transaction or multiple independent jobs",
            [
                project_cmd(
                    "apply",
                    "Apply an ordered JSON edit transaction to a single project",
                    [input().required(true)],
                ),
                Command::new("run")
                    .about("Run a JSON array of independent command argument arrays")
                    .arg(input().required(true)),
            ],
        ),
        family(
            "track",
            "Edit vocal and accompaniment tracks",
            [
                project_cmd("list", "List tracks and their persistent IDs", []),
                project_cmd(
                    "add",
                    "Add a new track",
                    [
                        name().required(true),
                        flag("instrumental", "Create an empty accompaniment track"),
                    ],
                ),
                project_cmd(
                    "delete",
                    "Delete a track and remove its orphaned groups",
                    [track().required(true)],
                ),
                project_cmd(
                    "copy",
                    "Duplicate a track; non-main groups remain linked",
                    [track().required(true), name()],
                ),
                project_cmd(
                    "set",
                    "Set track name, display order or mixer controls",
                    [
                        track().required(true),
                        name(),
                        option("volume", "Gain in decibels"),
                        option("pan", "Pan from -1 (left) to 1 (right)"),
                        choice("mute", "Track mute", &["true", "false"]),
                        choice("solo", "Track solo", &["true", "false"]),
                        option("index", "One-based track order"),
                    ],
                ),
            ],
        ),
        family(
            "group",
            "Manage group definitions and timeline references",
            [
                project_cmd(
                    "list",
                    "List group references with separate definition IDs",
                    [track()],
                ),
                project_cmd(
                    "add",
                    "Create a finite group on a vocal track",
                    [track(), name().required(true), at(), duration(), unit()],
                ),
                project_cmd(
                    "delete",
                    "Remove an instance from its track",
                    [track(), group().required(true)],
                ),
                project_cmd(
                    "copy",
                    "Create a separate copy of the source definition",
                    [
                        track(),
                        group().required(true),
                        option("to-track", "Destination track"),
                        at(),
                        unit(),
                    ],
                ),
                project_cmd(
                    "link",
                    "Add a reference to the source definition",
                    [
                        track(),
                        group().required(true),
                        option("to-track", "Destination track"),
                        at(),
                        unit(),
                    ],
                ),
                project_cmd(
                    "independent",
                    "Copy a linked definition and retarget this instance",
                    [track(), group().required(true)],
                ),
                project_cmd(
                    "move",
                    "Move a reference without changing its definition",
                    [
                        track(),
                        group().required(true),
                        option("to-track", "Destination track"),
                        at(),
                        unit(),
                    ],
                ),
                project_cmd(
                    "split",
                    "Split at an interior position into linked visible ranges",
                    [track(), group().required(true), at().required(true), unit()],
                ),
                project_cmd(
                    "merge",
                    "Combine adjacent, aligned references to the same definition",
                    [
                        track(),
                        group().required(true),
                        option("with", "Following group reference UUID").required(true),
                    ],
                ),
                project_cmd(
                    "set",
                    "Set group name, instance transpose or mute",
                    [
                        track(),
                        group().required(true),
                        name(),
                        shared(),
                        option("semitones", "Absolute instance transpose offset"),
                        choice("mute", "Instance mute", &["true", "false"]),
                    ],
                ),
            ],
        ),
    ]
}
