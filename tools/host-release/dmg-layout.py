#!/usr/bin/env python3
"""Draw the installation guide and store its Finder window layout."""
import subprocess
import sys
import tempfile
import datetime
import hashlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
from ds_store import DSStore


def background(path):
    image = Image.new("RGB", (1280, 720), "#f7f5f0")
    draw = ImageDraw.Draw(image)
    font = "/System/Library/Fonts/Supplemental/Arial.ttf"
    if not Path(font).exists():
        font = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
    draw.text((640, 100), "Install nori", font=ImageFont.truetype(font, 52), fill="#242c29", anchor="mm")
    draw.text((640, 610), "Drag nori to Applications", font=ImageFont.truetype(font, 32), fill="#59645e", anchor="mm")
    draw.line([(550, 340), (726, 340)], fill="#427a65", width=12)
    draw.line([(692, 306), (726, 340), (692, 374)], fill="#427a65", width=12, joint="curve")
    image.save(path.with_name("background@2x.png"))
    image.resize((640, 360), Image.Resampling.LANCZOS).save(path)


def build(stage, artifact):
    import dmgbuild

    backdrop = stage.parent / "background.png"
    background(backdrop)
    dmgbuild.build_dmg(str(artifact), "nori", settings={
        "format": "UDZO",
        "files": [str(stage / "nori.app")],
        "symlinks": {"Applications": "/Applications"},
        "icon": str(stage / "nori.app/Contents/Resources/nori.icns"),
        "background": str(backdrop),
        "window_rect": ((200, 160), (640, 360)),
        "default_view": "icon-view",
        "icon_size": 128,
        "text_size": 16,
        "icon_locations": {"nori.app": (160, 170), "Applications": (480, 170)},
        "show_toolbar": False,
        "show_sidebar": False,
        "show_status_bar": False,
        "show_tab_view": False,
        "show_pathbar": False,
    })


def verify(artifact):
    with tempfile.TemporaryDirectory(prefix="nori-layout-check-") as mount:
        subprocess.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mount, str(artifact)], check=True)
        try:
            root = Path(mount)
            with DSStore.open(str(root / ".DS_Store"), "r") as store:
                assert store["."]["icvp"]["iconSize"] >= 128
                assert store["."]["icvp"]["backgroundType"] == 2
                assert store["nori.app"]["Iloc"][0] < store["Applications"]["Iloc"][0]
                assert not store["."]["bwsp"]["ShowSidebar"]
            assert {p.name for p in root.iterdir() if not p.name.startswith(".")} == {"nori.app", "Applications"}
        finally:
            subprocess.run(["hdiutil", "detach", mount], check=True)


def build_linux(stage, artifact):
    from mac_alias import Alias, VolumeInfo, TargetInfo

    backdrop = stage / ".background/background.png"
    backdrop.parent.mkdir()
    background(backdrop)
    now = datetime.datetime.now(datetime.timezone.utc)
    alias = Alias(volume=VolumeInfo("nori", now, b"H+", 5, 0, b"\0\0", posix_path="/Volumes/nori"),
                  target=TargetInfo(0, "background.png", 0, 0, now, b"\0" * 4, b"\0" * 4,
                                    folder_name=".background", posix_path="/.background/background.png"))
    with DSStore.open(str(stage / ".DS_Store"), "w+") as store:
        store["."]["vSrn"] = ("long", 1)
        store["."]["icvl"] = ("type", b"icnv")
        store["."]["bwsp"] = {"WindowBounds": "{{200, 160}, {640, 360}}", "ShowSidebar": False,
                               "ShowToolbar": False, "ShowStatusBar": False, "ShowPathbar": False,
                               "ShowTabView": False}
        store["."]["icvp"] = {"viewOptionsVersion": 1, "backgroundType": 2,
                               "backgroundImageAlias": alias.to_bytes(), "iconSize": 128.0,
                               "textSize": 16.0, "arrangeBy": "none", "gridSpacing": 100.0,
                               "gridOffsetX": 0.0, "gridOffsetY": 0.0, "labelOnBottom": True,
                               "showIconPreview": True, "showItemInfo": False,
                               "scrollPositionX": 0.0, "scrollPositionY": 0.0}
        store["nori.app"]["Iloc"] = (160, 170)
        store["Applications"]["Iloc"] = (480, 170)
    image = stage.parent / "volume.hfs"
    size = sum(path.stat().st_size for path in stage.rglob("*") if path.is_file())
    with image.open("wb") as disk:
        disk.truncate(max(64 * 1024 * 1024, size * 2))
    subprocess.run(["mkfs.hfsplus", "-v", "nori", str(image)], check=True)
    subprocess.run(["hfsplus", str(image), "addall", str(stage)], check=True)
    for path in stage.rglob("*"):
        mode = format(path.stat().st_mode & 0o777, "o")
        subprocess.run(["hfsplus", str(image), "chmod", mode, "/" + str(path.relative_to(stage))], check=True)
    subprocess.run(["hfsplus", str(image), "symlink", "/Applications", "/Applications"], check=True)
    listing = subprocess.check_output(["hfsplus", str(image), "ls", "/nori.app/Contents/MacOS"], text=True)
    executable = next(line for line in listing.splitlines() if line.endswith(" nori"))
    assert int(executable.split()[0], 8) & 0o111 == 0o111, "DMG executable permissions"
    subprocess.run(["dmg", "build", str(image), str(artifact)], check=True, stdout=subprocess.DEVNULL)
    extracted = stage.parent / "verified.hfs"
    subprocess.run(["dmg", "extract", str(artifact), str(extracted)], check=True, stdout=subprocess.DEVNULL)
    roundtrip = stage.parent / "roundtrip"
    roundtrip.mkdir()
    subprocess.run(["hfsplus", str(extracted), "extractall", "/", str(roundtrip)], check=True, stdout=subprocess.DEVNULL)
    for path in stage.rglob("*"):
        if path.is_file():
            contents = (roundtrip / path.relative_to(stage)).read_bytes()
            assert hashlib.sha256(contents).digest() == hashlib.sha256(path.read_bytes()).digest(), path
    result = (roundtrip / "Applications").read_bytes()
    assert result == b"/Applications", "DMG Applications shortcut"
    print(f"Verified DMG contents: {artifact.name}; macOS execution checks skipped")


if __name__ == "__main__":
    if sys.argv[1] == "build":
        build(Path(sys.argv[2]), Path(sys.argv[3]))
    elif sys.argv[1] == "verify":
        verify(Path(sys.argv[2]))
    elif sys.argv[1] == "build-linux":
        build_linux(Path(sys.argv[2]), Path(sys.argv[3]))
    else:
        raise SystemExit("usage: dmg-layout.py build STAGE DMG | verify DMG")
