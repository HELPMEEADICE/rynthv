"""Build and analyze a two-part song using the installed RynthV command."""
import argparse
import json
from pathlib import Path
import subprocess


def cli(*args):
    process = subprocess.run(
        ["rynthv", "-cli", "--non-interactive", "--json", "--quiet", *map(str, args)],
        stdout=subprocess.PIPE,
        encoding="utf-8",
        check=False,
    )
    reply = json.loads(process.stdout)
    if process.returncode or not reply["ok"]:
        raise RuntimeError(reply.get("error", reply))
    return reply["result"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("voice_id")
    parser.add_argument("--revision", type=int)
    parser.add_argument("--backing", type=Path)
    parser.add_argument("--compute-backend", choices=["auto", "cpu", "webgpu"], default="cpu")
    args = parser.parse_args()

    directory = args.directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    assets = Path(__file__).resolve().parent
    project = directory / "song.svp"
    leads = cli("project", "new", project, "--bpm", 128, "--time-signature", "4/4")
    lead = leads["tracks"][0]["id"]
    cli("track", "set", project, "--track", lead, "--name", "Lead")
    harmony = cli("track", "add", project, "--name", "Harmony")["edit"]["id"]

    voice = ["--voice", args.voice_id, "--language", "japanese"]
    if args.revision is not None:
        voice += ["--revision", args.revision]
    for track in (lead, harmony):
        cli("voice", "assign", project, "--track", track, *voice)
        cli("note", "import", project, "--track", track, "--input", assets / "notes.json")
        cli("lyrics", "fill", project, "--track", track, "--language", "japanese",
            "--text", "あ い う え")
    cli("note", "transpose", project, "--track", harmony, "--semitones", -5)
    cli("track", "set", project, "--track", harmony, "--volume", -6, "--pan", 0.2)
    cli("parameter", "curve", project, "--track", lead, "--name", "tension",
        "--input", assets / "tuning.json")

    lead_note = cli("note", "list", project, "--track", lead)["notes"][0]["id"]
    cli("phoneme", "set", project, "--note", lead_note, "--value", "a")
    cli("phoneme", "export", project, "--track", lead,
        "--output", directory / "pronunciation.json")
    if args.backing:
        cli("audio", "import", project, "--input", args.backing.resolve(), "--name", "Backing")

    backend = ("--compute-backend", args.compute_backend)
    preview = directory / "preview.wav"
    cli("render", project, "--track", lead, "--from", 0, "--to", 2, "--unit", "beat",
        "--output", preview, "--sample-rate", 48000, "--bit-depth", 32, *backend)
    analysis = cli("audio", "analyze", preview, "--f0")
    (directory / "preview-analysis.json").write_text(
        json.dumps(analysis, ensure_ascii=False, indent=2) + "\n", encoding="utf-8",
    )
    result = cli("render", project, "--output", directory / "song.wav",
                 "--sample-rate", 48000, "--channels", 2, "--bit-depth", 32, *backend)
    cli("render", project, "--stems", "--output-dir", directory / "stems", *backend)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
