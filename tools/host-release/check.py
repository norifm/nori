#!/usr/bin/env python3
"""Verify packaged clients load on their native platform."""
import hashlib
import platform
import plistlib
import subprocess
import sys
import tempfile
from pathlib import Path

output = Path(sys.argv[1]).resolve()
for sums in output.glob("SHA256SUMS-*"):
    for line in sums.read_text().splitlines():
        digest, name = line.split("  ", 1)
        assert hashlib.sha256((output / name).read_bytes()).hexdigest() == digest, name
for archive in output.glob("*.tar.gz"):
    with tempfile.TemporaryDirectory(prefix="nori-package-check-") as directory:
        subprocess.run(["tar", "-xzf", str(archive), "-C", directory], check=True)
        root = next(Path(directory).iterdir())
        client = "cli" if archive.name.startswith("nori-cli-") else "desktop"
        binary = root / f"nori-{client}"
        if platform.system() == "Darwin":
            if client == "desktop":
                bundle = root / "nori.app"
                info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
                binary = bundle / "Contents/MacOS" / info["CFBundleExecutable"]
                subprocess.run(["codesign", "--verify", "--strict", str(bundle)], check=True)
            else:
                subprocess.run(["codesign", "--verify", "--strict", str(binary)], check=True)
        result = subprocess.run([str(binary), "--help"], capture_output=True, text=True)
        assert f"usage: nori-{client}" in result.stdout + result.stderr, result.stderr
        print(f"Verified {archive.name}")
for deb in output.glob("*.deb"):
    subprocess.run(["dpkg-deb", "--info", str(deb)], check=True)
    dependencies = subprocess.check_output(["dpkg-deb", "--field", str(deb), "Depends"], text=True)
    assert dependencies.strip(), deb
