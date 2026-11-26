"""Build the static RynthV site using the locked Rust and wasm-bindgen versions."""

import argparse
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
import urllib.request
from uuid import uuid4
import zipfile


ROOT = Path(__file__).resolve().parents[2]
WEB = ROOT / "packaging" / "web"
TARGET = "wasm32-unknown-unknown"
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
    "style.css", "main.js", "strings.js", "browser.js", "documents.js",
    "worker.js", "worker-bridge.js", "cpu-pool.js", "cpu-worker.js", "cpu-worker-bridge.js",
    "opfs.js", "audio.js", "pcm-worklet.js",
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


def bundle_digest(bundle):
    digest = hashlib.sha256()
    names = sorted(path.relative_to(bundle).as_posix() for path in bundle.rglob("*") if path.is_file())
    for name in names:
        digest.update(name.encode("utf-8") + b"\0")
        with (bundle / name).open("rb") as asset:
            digest.update(hashlib.file_digest(asset, "sha256").digest())
    return digest.hexdigest()


def prune_output(directory, keep):
    for entry in directory.iterdir():
        if entry.name in keep:
            continue
        if entry.resolve() != entry:
            raise SystemExit(f"Expected a generated output at {entry}")
        if entry.is_dir():
            shutil.rmtree(entry)
        else:
            entry.unlink()


def publish_site(site, destination, bundle_path):
    bundle_output = destination / bundle_path
    if bundle_output.resolve() != bundle_output:
        raise SystemExit(f"Expected the generated asset directory at {bundle_output}")
    bundle_output.parent.mkdir(parents=True, exist_ok=True)
    if bundle_output.exists() and bundle_digest(bundle_output) != bundle_path.name:
        shutil.rmtree(bundle_output)
    if not bundle_output.exists():
        (site / bundle_path).rename(bundle_output)
    (site / "index.html").replace(destination / "index.html")


@contextmanager
def staging_directory(dist):
    # Renamed files retain their Windows ACLs, so inherit permissions from dist.
    staging = dist / f"site-{uuid4().hex}"
    staging.mkdir()
    try:
        yield staging
    finally:
        shutil.rmtree(staging)


def package_site(bindgen, wasm, debug, dist):
    cache = ROOT / "target" / "web-fonts"
    cache.mkdir(parents=True, exist_ok=True)
    site_output = dist / "web"
    archive_output = dist / "rynthv-web.zip"
    if dist.resolve() != dist:
        raise SystemExit(f"Expected the generated output directory at {dist}")
    dist.mkdir(parents=True, exist_ok=True)

    with staging_directory(dist) as staging:
        bundle = staging / "bundle"
        flags = () if debug else ("--remove-name-section", "--remove-producers-section")
        run(bindgen, "--target", "web", "--no-typescript", "--out-dir", bundle, *flags, wasm)
        for name in STATIC:
            shutil.copyfile(WEB / name, bundle / name)
        for name, (source, digest) in ASSETS.items():
            shutil.copyfile(asset_path(cache, name, source, digest), bundle / name)
        shutil.copyfile(ROOT / "assets" / "branding" / "app-icon.png", bundle / "app-icon.png")

        # Relative imports keep the page, Worker and AudioWorklet in the same release.
        site = staging / "web"
        bundle_path = Path("assets") / bundle_digest(bundle)
        assets = site / bundle_path
        assets.parent.mkdir(parents=True)
        bundle.rename(assets)
        index = (WEB / "index.html").read_text(encoding="utf-8")
        (site / "index.html").write_text(
            index.replace("__WEB_ASSETS__", bundle_path.as_posix()), encoding="utf-8", newline="\n",
        )

        archive_path = staging / archive_output.name
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for path in sorted(site.rglob("*")):
                if path.is_file():
                    archive.write(path, path.relative_to(site).as_posix())

        publish_site(site, site_output, bundle_path)
        archive_path.replace(archive_output)
        prune_output(site_output, {"assets", "index.html"})
        prune_output(site_output / bundle_path.parent, {bundle_path.name})
    print(f"Static site: {site_output}")
    print(f"Deployment archive: {archive_output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Run compile and strict Clippy checks only")
    parser.add_argument("--debug", action="store_true", help="Package an unoptimized development build")
    parser.add_argument("--webgpu-renderer", action="store_true", help="Use WebGPU for the editor canvas")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist", help="Directory for the site and deployment archive")
    args = parser.parse_args()
    features = "webgpu-renderer" if args.webgpu_renderer else "web"
    build_args = ("--locked", "--no-default-features", "--features", features, "--lib", "--target", TARGET)
    cargo = os.environ.get("CARGO", "cargo")
    env = os.environ.copy()
    target_dir = Path(env.get("CARGO_TARGET_DIR", ROOT / "target" / "wasm"))
    if not target_dir.is_absolute():
        target_dir = ROOT / target_dir
    env["CARGO_TARGET_DIR"] = str(target_dir)
    if "CARGO_ENCODED_RUSTFLAGS" in env:
        flags = env["CARGO_ENCODED_RUSTFLAGS"]
        env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(filter(None, (flags, "-C", "target-feature=+simd128")))
    else:
        flags = env.get("RUSTFLAGS", env.get("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS", ""))
        env["RUSTFLAGS"] = f"{flags} -C target-feature=+simd128".strip()
    if args.check:
        run(cargo, "check", *build_args, env=env)
        run(cargo, "clippy", *build_args, "--", "-D", "warnings", env=env)
        return
    bindgen = bindgen_path()
    if not args.debug:
        env["CARGO_PROFILE_RELEASE_STRIP"] = "debuginfo"
    run(cargo, "build", *build_args, *(() if args.debug else ("--release",)), env=env)
    wasm = target_dir / TARGET / ("debug" if args.debug else "release") / "rynthv.wasm"
    dist = args.output_dir
    if not dist.is_absolute():
        dist = ROOT / dist
    package_site(bindgen, wasm, args.debug, dist)


if __name__ == "__main__":
    main()
