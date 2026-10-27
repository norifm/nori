#!/usr/bin/env python3
"""Create the desktop app bundle on macOS or a Linux cross-builder."""
import argparse
import hashlib
import platform
import plistlib
import shutil
import subprocess
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def verify_signature(binary, app=None):
    data = binary.read_bytes()
    magic, cpu, _, kind, commands = struct.unpack_from("<5I", data)
    assert magic == 0xFEEDFACF and cpu in (0x1000007, 0x100000C) and kind == 2, "macOS executable"
    at = 32
    signature = None
    for _ in range(commands):
        command, size = struct.unpack_from("<2I", data, at)
        if command == 0x1D:
            offset, length = struct.unpack_from("<2I", data, at + 8)
            signature = data[offset:offset + length]
        at += size
    assert signature is not None, "Mach-O code signature"
    magic, length, count = struct.unpack_from(">3I", signature)
    assert magic == 0xFADE0CC0 and length <= len(signature), "code signature superblob"
    directories = 0
    for index in range(count):
        _, offset = struct.unpack_from(">2I", signature, 12 + index * 8)
        if struct.unpack_from(">I", signature, offset)[0] != 0xFADE0C02:
            continue
        directory = signature[offset:]
        _, length, _, flags, hashes, _, special, slots, limit, hash_size, hash_type, _, page = struct.unpack_from(">9I4B", directory)
        assert flags & 2, "ad-hoc signature"
        algorithm = {1: "sha1", 2: "sha256", 3: "sha256", 4: "sha384"}[hash_type]
        page_size = 1 << page
        assert slots == (limit + page_size - 1) // page_size, "code signature page count"
        assert hashes + slots * hash_size <= length, "code signature hash table"
        for slot in range(slots):
            payload = data[slot * page_size:min((slot + 1) * page_size, limit)]
            digest = hashlib.new(algorithm, payload).digest()[:hash_size]
            assert digest == directory[hashes + slot * hash_size:hashes + (slot + 1) * hash_size], f"code signature page {slot}"
        if app:
            for slot, name in ((1, "Info.plist"), (3, "_CodeSignature/CodeResources")):
                assert special >= slot, f"signed {name}"
                digest = hashlib.new(algorithm, (app / "Contents" / name).read_bytes()).digest()[:hash_size]
                assert digest == directory[hashes - slot * hash_size:hashes - (slot - 1) * hash_size], f"code signature {name}"
        directories += 1
    assert directories, "signed code directory"


def sign(path):
    command = ["codesign", "--force", "--sign", "-"] if platform.system() == "Darwin" else ["rcodesign", "sign"]
    subprocess.run([*command, str(path)], check=True)
    if platform.system() != "Darwin":
        binary = path / "Contents/MacOS/nori" if path.is_dir() else path
        verify_signature(binary, path if path.is_dir() else None)


def bundle(binary, directory, version):
    app = directory / "nori.app"
    if app.exists():
        shutil.rmtree(app)
    executable = app / "Contents/MacOS/nori"
    resources = app / "Contents/Resources"
    executable.parent.mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(binary, executable)
    icon = ROOT / "docs/brand/nori-macos.png"
    if platform.system() == "Darwin":
        import tempfile
        with tempfile.TemporaryDirectory(prefix="nori-icon-") as temporary:
            iconset = Path(temporary) / "nori.iconset"
            iconset.mkdir()
            for size in (16, 32, 128, 256, 512):
                for scale in (1, 2):
                    name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
                    subprocess.run(["sips", "-z", str(size * scale), str(size * scale), str(icon), "--out", str(iconset / name)], check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(resources / "nori.icns")], check=True)
    else:
        from PIL import Image
        with Image.open(icon) as image:
            image.save(resources / "nori.icns", format="ICNS")
    info = {
        "CFBundleName": "nori", "CFBundleDisplayName": "nori",
        "CFBundleIdentifier": "dev.nori.desktop", "CFBundleExecutable": "nori",
        "CFBundleIconFile": "nori", "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version.split("-", 1)[0],
        "CFBundleVersion": version.replace("-beta.", "b").replace("-nightly.", "d"), "LSMinimumSystemVersion": "11.0",
        "LSApplicationCategoryType": "public.app-category.music", "NSHighResolutionCapable": True,
    }
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    shutil.copy2(ROOT / "LICENSE", resources / "LICENSE")
    sign(app)
    return app


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    print(bundle(args.binary, args.directory, args.version))
