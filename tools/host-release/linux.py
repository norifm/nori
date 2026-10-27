#!/usr/bin/env python3
"""Build desktop AppImage and Flatpak bundles from native release binaries."""
import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP_ID = "dev.nori.desktop"
# GNOME's runtime: it carries WebKitGTK, which the sign-in window uses.
RUNTIME = "50"


def run(*args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def appimage_executable(image, directory):
    executable = directory / f"{image.name}.elf"
    shutil.copy2(image, executable)
    # AppImage's marker collides with the ELF mask used by Rosetta.
    with executable.open("r+b") as file:
        file.seek(8)
        file.write(bytes(3))
    return executable


def package(binaries, output, version, arch, parts):
    """Builds `parts` of the desktop's Linux packages for `arch`: the AppImage (this machine's architecture
    only, as it is run to check it) and the Flatpak (any architecture: Flatpak builds for another one, and
    runs it in its own sandbox under binfmt emulation, which bwrap cannot set up when it is emulated itself)."""
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nori-linux-") as temporary:
        work = Path(temporary)
        icon = work / "nori.png"
        run("convert", ROOT / "app/src/main/res/drawable-xxxhdpi/nori_mark.png", "-resize", "512x512", icon)
        desktop = work / f"{APP_ID}.desktop"
        desktop.write_text("[Desktop Entry]\nType=Application\nName=nori\nExec=nori-desktop\nIcon=nori\nTerminal=false\nCategories=AudioVideo;Audio;Player;\n")
        made = []
        if "appimage" in parts:
            made.append(appimage(binaries, output, version, arch, work, icon, desktop))
        if "flatpak" in parts:
            made.append(flatpak(binaries, output, version, arch, work, icon, desktop))
        sums = output / f"SHA256SUMS-linux-{arch}"
        existing = sums.read_text() if sums.exists() else ""
        names = {p.name for p in made}
        existing = "".join(line + "\n" for line in existing.splitlines() if line.split("  ", 1)[1] not in names)
        sums.write_text(existing + "".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in made))


def appimage(binaries, output, version, arch, work, icon, desktop):
    app = work / "AppDir"
    (app / "usr/bin").mkdir(parents=True)
    shutil.copy2(binaries / "nori-desktop", app / "usr/bin/nori-desktop")
    shutil.copy2(icon, app / "nori.png")
    shutil.copy2(desktop, app / desktop.name)
    licenses = app / "usr/share/licenses/nori"
    licenses.mkdir(parents=True)
    shutil.copy2(ROOT / "LICENSE", licenses / "LICENSE")
    # No libraries are bundled: the sign-in window's WebKitGTK and the GTK and GLib under it must be the
    # system's, and any older copy of a library they share (libmount, libsystemd) breaks them on newer systems.
    (app / "AppRun").symlink_to("usr/bin/nori-desktop")
    (app / ".DirIcon").symlink_to("nori.png")
    env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1", ARCH=arch)
    tool = work / "appimagetool.AppImage"
    run("curl", "-fL", f"https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-{arch}.AppImage", "-o", tool)
    tool.chmod(0o755)
    image = output / f"nori-desktop-{version}-linux-{arch}.AppImage"
    run(appimage_executable(tool, work), app, image, env=env)
    result = subprocess.run([str(appimage_executable(image, work)), "--appimage-extract-and-run", "--help"], capture_output=True, text=True)
    assert "usage: nori-desktop" in result.stdout + result.stderr, result.stderr
    return image


def flatpak(binaries, output, version, arch, work, icon, desktop):
    # Another architecture's app runs under binfmt emulation, whose syscalls are this machine's: its sandbox
    # goes without the syscall filter Flatpak makes for the app's architecture.
    if arch != platform.machine():
        os.environ["FLATPAK_BWRAP"] = str(ROOT / "tools/host-release/emulated-bwrap.py")
    run("flatpak", "remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo")
    run("flatpak", "install", "--user", "--noninteractive", f"--arch={arch}", "flathub", f"org.gnome.Platform//{RUNTIME}", f"org.gnome.Sdk//{RUNTIME}")
    bundle = work / "flatpak"
    run("flatpak", "build-init", f"--arch={arch}", bundle, APP_ID, "org.gnome.Sdk", "org.gnome.Platform", RUNTIME)
    files = bundle / "files"
    (files / "bin").mkdir()
    for client in ("desktop", "cli"):
        shutil.copy2(binaries / f"nori-{client}", files / "bin" / f"nori-{client}")
    applications = files / "share/applications"
    applications.mkdir(parents=True)
    (applications / f"{APP_ID}.desktop").write_text(desktop.read_text().replace("Icon=nori", f"Icon={APP_ID}"))
    icons = files / "share/icons/hicolor/512x512/apps"
    icons.mkdir(parents=True)
    shutil.copy2(icon, icons / f"{APP_ID}.png")
    licenses = files / "share/licenses/nori"
    licenses.mkdir(parents=True)
    shutil.copy2(ROOT / "LICENSE", licenses / "LICENSE")
    run("flatpak", "build-finish", "--command=nori-desktop", "--share=network", "--share=ipc", "--socket=wayland", "--socket=fallback-x11", "--socket=pulseaudio", "--device=dri", "--own-name=org.mpris.MediaPlayer2.nori.*", bundle)
    repo = work / "repo"
    branch = next((kind for kind in ("beta", "nightly") if f"-{kind}." in version), "stable")
    run("flatpak", "build-export", f"--arch={arch}", repo, bundle, branch)
    artifact = output / f"nori-desktop-{version}-linux-{arch}.flatpak"
    run("flatpak", "build-bundle", f"--arch={arch}", "--runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo", repo, artifact, APP_ID, branch)
    run("flatpak", "install", "--user", "--noninteractive", artifact)
    result = subprocess.run(["flatpak", "run", f"--arch={arch}", "--command=nori-cli", APP_ID, "--help"], capture_output=True, text=True)
    assert "usage: nori-cli" in result.stdout + result.stderr, result.stderr
    result = subprocess.run(["flatpak", "run", f"--arch={arch}", APP_ID, "--help"], capture_output=True, text=True)
    assert "usage: nori-desktop" in result.stdout + result.stderr, result.stderr
    return artifact


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", choices=("x86_64", "aarch64"), default=platform.machine())
    parser.add_argument("--only", choices=("appimage", "flatpak"))
    args = parser.parse_args()
    package(args.binaries.resolve(), args.output, args.version, args.arch, [args.only] if args.only else ["appimage", "flatpak"])
