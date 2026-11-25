"""Validate the native libraries packaged in one Android APK."""

import pathlib
import re
import subprocess
import sys
import tempfile
import zipfile


def output(*args):
    return subprocess.check_output(args, text=True)


def verify(apk, abi, readelf, nm):
    expected_machine = {"arm64-v8a": "AArch64", "x86_64": "Advanced Micro Devices X86-64"}[abi]
    expected_library = f"lib/{abi}/librynthv_android.so"
    with zipfile.ZipFile(apk) as archive, tempfile.TemporaryDirectory() as temporary:
        libraries = [item for item in archive.infolist() if re.fullmatch(r"lib/[^/]+/[^/]+\.so", item.filename)]
        assert any(item.filename == expected_library for item in libraries), expected_library
        for item in libraries:
            assert item.filename.startswith(f"lib/{abi}/"), item.filename
            assert item.compress_type == zipfile.ZIP_STORED, item.filename
            path = pathlib.Path(archive.extract(item, temporary))
            header = output(readelf, "-h", str(path))
            assert re.search(rf"Machine:\s+{expected_machine}\s*$", header, re.MULTILINE), header
            segments = output(readelf, "-Wl", str(path))
            loads = [line.split() for line in segments.splitlines() if line.strip().startswith("LOAD ")]
            assert loads, f"No LOAD segments in {item.filename}"
            for load in loads:
                offset, address = (int(value, 16) for value in load[1:3])
                alignment = int(load[-1], 16)
                assert alignment >= 16384 and (address - offset) % 16384 == 0, load
            if item.filename == expected_library:
                symbols = output(nm, "-D", "--defined-only", str(path))
                for symbol in (
                    "ANativeActivity_onCreate",
                    "Java_org_rynthv_app_RynthVActivity_nativeWake",
                ):
                    assert re.search(rf"\b{symbol}$", symbols, re.MULTILINE), symbol
            print(f"Verified {item.filename}: {expected_machine}, {len(loads)} aligned LOAD segments")


if __name__ == "__main__":
    verify(*sys.argv[1:])
