"""Build the static RynthV site using the locked Rust and wasm-bindgen versions."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[2]
WEB = ROOT / "packaging" / "web"
TARGET = "wasm32-unknown-unknown"
FEATURES = ("--locked", "--no-default-features", "--features", "web", "--lib", "--target", TARGET)
FONT_ROOT = "https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004"
ASSETS = {
    "NotoSansCJKsc-Regular.otf": (
        "Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf",
        "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b",
    ),
    "OFL-NotoSansCJK.txt": (
        "LICENSE",
        "6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2",
    ),
}
STATIC = (
    "index.html", "style.css", "main.js", "browser.js", "documents.js",
    "worker.js", "worker-bridge.js", "opfs.js", "audio.js", "pcm-worklet.js",
)


def run(*args, env=None):
    subprocess.run(args, cwd=ROOT, check=True, env=env)


def asset_path(directory, name, source, digest):
    path = directory / name
    if not path.exists():
        with urllib.request.urlopen(f"{FONT_ROOT}/{source}", timeout=120) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise SystemExit(f"Upstream checksum mismatch for {name}")
        path.write_bytes(data)
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise SystemExit(f"Cached asset checksum mismatch: {path}")
    return path


def bindgen_path():
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    version, = (package["version"] for package in lock["package"] if package["name"] == "wasm-bindgen")
    executable = os.environ.get("WASM_BINDGEN", "wasm-bindgen")
    try:
        actual = subprocess.check_output((executable, "--version"), cwd=ROOT, text=True).strip()
    except FileNotFoundError:
        actual = "missing"
    if actual != f"wasm-bindgen {version}":
        raise SystemExit(f"Expected wasm-bindgen {version} (found {actual}). Run: cargo install --locked wasm-bindgen-cli --version {version}")
    return executable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Run compile and strict Clippy checks only")
    parser.add_argument("--debug", action="store_true", help="Package an unoptimized development build")
    args = parser.parse_args()
    cargo = os.environ.get("CARGO", "cargo")
    env = os.environ.copy()
    target_dir = Path(env.get("CARGO_TARGET_DIR", ROOT / "target" / "wasm"))
    if not target_dir.is_absolute():
        target_dir = ROOT / target_dir
    env["CARGO_TARGET_DIR"] = str(target_dir)
    env["CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS"] = "-C target-feature=+simd128"
    if args.check:
        run(cargo, "check", *FEATURES, env=env)
        run(cargo, "clippy", *FEATURES, "--", "-D", "warnings", env=env)
        return
    bindgen = bindgen_path()
    if not args.debug:
        env["CARGO_PROFILE_RELEASE_STRIP"] = "debuginfo"
    run(cargo, "build", *FEATURES, *(() if args.debug else ("--release",)), env=env)
    wasm = target_dir / TARGET / ("debug" if args.debug else "release") / "rynthv.wasm"
    cache = ROOT / "target" / "web-fonts"
    cache.mkdir(parents=True, exist_ok=True)
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="site-", dir=target_dir) as staging:
        site = Path(staging)
        flags = () if args.debug else ("--remove-name-section", "--remove-producers-section")
        run(bindgen, "--target", "web", "--no-typescript", "--out-dir", site, *flags, wasm)
        for name in STATIC:
            shutil.copyfile(WEB / name, site / name)
        for name, (source, digest) in ASSETS.items():
            shutil.copyfile(asset_path(cache, name, source, digest), site / name)
        shutil.copyfile(ROOT / "assets" / "branding" / "app-icon.png", site / "app-icon.png")
        shutil.copytree(site, dist / "web", dirs_exist_ok=True)
        with zipfile.ZipFile(dist / "rynthv-web.zip", "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for path in sorted(site.rglob("*")):
                if path.is_file():
                    archive.write(path, path.relative_to(site).as_posix())
    print(f"Static site: {dist / 'web'}")
    print(f"Deployment archive: {dist / 'rynthv-web.zip'}")


if __name__ == "__main__":
    main()
