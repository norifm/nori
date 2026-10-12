#!/usr/bin/env python3
"""Create and verify a macOS disk image from a native desktop binary."""
import argparse
import hashlib
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def package(binary, output, version, arch):
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    artifact = output / f"nori-desktop-{version}-macos-{arch}.dmg"
    with tempfile.TemporaryDirectory(prefix="nori-dmg-") as temporary:
        stage = Path(temporary) / "stage"
        env = dict(os.environ, NORI_APP_DIR=str(stage), NORI_DESKTOP_BINARY=str(binary.resolve()), NORI_VERSION=version)
        subprocess.run([str(ROOT / "tools/desktop-app.sh"), "--no-build"], env=env, check=True)
        (stage / "nori.app/Contents/Resources/LICENSE").write_bytes((ROOT / "LICENSE").read_bytes())
        subprocess.run(["codesign", "--force", "--sign", "-", str(stage / "nori.app")], check=True)
        environment = Path(temporary) / "python"
        subprocess.run([sys.executable, "-m", "venv", str(environment)], check=True)
        python = str(environment / "bin/python")
        subprocess.run([python, "-m", "pip", "install", "--disable-pip-version-check", "dmgbuild==1.6.7", "Pillow==11.3.0"], check=True)
        layout = str(ROOT / "tools/host-release/dmg-layout.py")
        subprocess.run([python, layout, "build", str(stage), str(artifact)], check=True)
        subprocess.run([python, layout, "verify", str(artifact)], check=True)
    verify(artifact)
    return artifact


def verify(artifact):
    subprocess.run(["hdiutil", "verify", str(artifact)], check=True)
    with tempfile.TemporaryDirectory(prefix="nori-dmg-check-") as temporary:
        subprocess.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", temporary, str(artifact)], check=True)
        try:
            root = Path(temporary)
            assert (root / "Applications").readlink() == Path("/Applications")
            subprocess.run(["codesign", "--verify", "--strict", str(root / "nori.app")], check=True)
            result = subprocess.run([str(root / "nori.app/Contents/MacOS/nori"), "--help"], capture_output=True, text=True)
            assert "usage: nori-desktop" in result.stdout + result.stderr, result.stderr
        finally:
            subprocess.run(["hdiutil", "detach", temporary], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    arch = {"arm64": "aarch64", "x86_64": "x86_64"}[platform.machine()]
    artifact = package(args.binary, args.output, args.version, arch)
    sums = args.output / f"SHA256SUMS-macos-{arch}"
    existing = sums.read_text() if sums.exists() else ""
    existing = "".join(line + "\n" for line in existing.splitlines() if line.split("  ", 1)[1] != artifact.name)
    sums.write_text(existing + f"{hashlib.sha256(artifact.read_bytes()).hexdigest()}  {artifact.name}\n")
