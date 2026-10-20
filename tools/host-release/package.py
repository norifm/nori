#!/usr/bin/env python3
"""Package native desktop and terminal binaries without rebuilding them."""
import argparse
import hashlib
import platform
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

import dmg
from app import sign

ROOT = Path(__file__).resolve().parents[2]


def package(binaries, output, version, system, arch):
    output.mkdir(parents=True, exist_ok=True)
    artifacts = []
    with tempfile.TemporaryDirectory(prefix="nori-package-") as temporary:
        work = Path(temporary)
        for client in ("desktop", "cli"):
            if system == "macos" and client == "desktop":
                artifacts.append(dmg.package(binaries / "nori-desktop", output, version, arch))
                continue
            name = f"nori-{client}-{version}-{system}-{arch}"
            stage = work / name
            stage.mkdir()
            binary = binaries / f"nori-{client}"
            shutil.copy2(binary, stage / binary.name)
            if system == "macos":
                sign(stage / binary.name)
            shutil.copy2(ROOT / "LICENSE", stage / "LICENSE")
            (stage / "README.txt").write_text(
                "nori music\nhttps://github.com/norifm/nori\n\n"
                + f"Run ./{binary.name} or install it on your PATH.\n"
                + "Desktop and terminal share ~/.local/share/nori (or $XDG_DATA_HOME/nori).\n")
            archive = output / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                tar.add(stage, arcname=name)
            artifacts.append(archive)
            if system == "linux" and (Path("/etc/debian_version").exists()):
                deb_arch = {"x86_64": "amd64", "aarch64": "arm64"}[arch]
                deb_version = version.replace("-beta.", "~beta.")
                deb = work / f"deb-{client}"
                (deb / "usr/bin").mkdir(parents=True)
                (deb / "DEBIAN").mkdir()
                shutil.copy2(binary, deb / "usr/bin" / binary.name)
                if client == "desktop":
                    applications = deb / "usr/share/applications"
                    applications.mkdir(parents=True)
                    (applications / "dev.nori.music.desktop.desktop").write_text(
                        "[Desktop Entry]\nType=Application\nName=nori\nExec=nori-desktop\nIcon=/usr/share/pixmaps/nori-desktop.png\nTerminal=false\nCategories=AudioVideo;Audio;Player;\n")
                    icon = deb / "usr/share/pixmaps"
                    icon.mkdir(parents=True)
                    shutil.copy2(ROOT / "app/src/main/res/drawable-xxxhdpi/nori_mark.png", icon / "nori-desktop.png")
                doc = deb / f"usr/share/doc/nori-{client}"
                doc.mkdir(parents=True)
                shutil.copy2(ROOT / "LICENSE", doc / "copyright")
                shutil.copy2(stage / "README.txt", doc / "README")
                metadata = work / f"metadata-{client}/debian"
                metadata.mkdir(parents=True)
                (metadata / "control").write_text(f"Source: nori-{client}\n\nPackage: nori-{client}\nArchitecture: any\n")
                deps = subprocess.check_output(["dpkg-shlibdeps", "-O", str(deb / "usr/bin" / binary.name)], cwd=metadata.parent, text=True).strip().split("=", 1)[1]
                if client == "desktop":
                    deps += ", libegl1, libgl1, libxkbcommon0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libx11-6, libxrandr2, libxi6, libxcursor1, libx11-xcb1"
                (deb / "DEBIAN/control").write_text(
                    f"Package: nori-{client}\nVersion: {deb_version}\nArchitecture: {deb_arch}\nMaintainer: nori <filipton@users.noreply.github.com>\n"
                    f"Depends: {deps}\nSection: sound\nPriority: optional\nHomepage: https://norifm.com\nDescription: nori music {client} client\n")
                artifact = output / f"nori-{client}_{version}_{deb_arch}.deb"
                subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(deb), str(artifact)], check=True)
                artifacts.append(artifact)
    sums = output / f"SHA256SUMS-{system}-{arch}"
    sums.write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in artifacts))


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binaries", type=Path, default=ROOT / "target/dist")
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--arch", choices=("aarch64", "x86_64"))
    p.add_argument("--system", choices=("macos", "linux"))
    a = p.parse_args()
    system = a.system or {"Linux": "linux", "Darwin": "macos"}[platform.system()]
    arch = a.arch or {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}[platform.machine()]
    package(a.binaries, a.output, a.version, system, arch)
