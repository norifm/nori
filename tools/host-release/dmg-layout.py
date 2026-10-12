#!/usr/bin/env python3
"""Draw the installation guide and store its Finder window layout."""
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
from ds_store import DSStore


def background(path):
    image = Image.new("RGB", (1280, 720), "#f7f5f0")
    draw = ImageDraw.Draw(image)
    font = "/System/Library/Fonts/Supplemental/Arial.ttf"
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


if __name__ == "__main__":
    if sys.argv[1] == "build":
        build(Path(sys.argv[2]), Path(sys.argv[3]))
    elif sys.argv[1] == "verify":
        verify(Path(sys.argv[2]))
    else:
        raise SystemExit("usage: dmg-layout.py build STAGE DMG | verify DMG")
