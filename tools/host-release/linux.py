#!/usr/bin/env python3
"""Build desktop AppImage and Flatpak bundles from native release binaries."""
import argparse
import hashlib
import os
import platform
import shutil
import sys
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP_ID = "dev.nori.desktop"
RUNTIME = "25.08"


def run(*args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def package(binaries, output, version):
    arch = {"x86_64": "x86_64", "aarch64": "aarch64"}[platform.machine()]
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nori-linux-") as temporary:
        work = Path(temporary)
        app = work / "AppDir"
        (app / "usr/bin").mkdir(parents=True)
        shutil.copy2(binaries / "nori-desktop", app / "usr/bin/nori-desktop")
        icon = app / "nori.png"
        run("convert", ROOT / "app/src/main/res/drawable-xxxhdpi/nori_mark.png", "-resize", "512x512", icon)
        desktop = app / f"{APP_ID}.desktop"
        desktop.write_text("[Desktop Entry]\nType=Application\nName=nori\nExec=nori-desktop\nIcon=nori\nTerminal=false\nCategories=AudioVideo;Audio;Player;\n")
        licenses = app / "usr/share/licenses/nori"
        licenses.mkdir(parents=True)
        shutil.copy2(ROOT / "LICENSE", licenses / "LICENSE")
        deploy = work / "linuxdeploy.AppImage"
        run("curl", "-fL", f"https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-{arch}.AppImage", "-o", deploy)
        deploy.chmod(0o755)
        env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1", ARCH=arch)
        libdir = Path("/usr/lib") / {"x86_64": "x86_64-linux-gnu", "aarch64": "aarch64-linux-gnu"}[arch]
        libraries = ("libasound.so.2", "libxkbcommon.so.0", "libwayland-client.so.0", "libwayland-cursor.so.0", "libwayland-egl.so.1")
        args = [arg for library in libraries for arg in ("--library", libdir / library)]
        plugins = (libdir / "alsa-lib/libasound_module_pcm_pulse.so", libdir / "alsa-lib/libasound_module_ctl_pulse.so")
        args += [arg for plugin in plugins for arg in ("--library", plugin)]
        run(deploy, "--appdir", app, "--executable", app / "usr/bin/nori-desktop", "--desktop-file", desktop, "--icon-file", icon, *args, env=env)
        (app / "usr/share/alsa").mkdir(parents=True)
        (app / "usr/share/alsa/nori.conf").write_text("pcm.!default { type pulse }\nctl.!default { type pulse }\n")
        plugin_dir = app / "usr/lib/alsa-lib"
        plugin_dir.mkdir()
        for plugin in plugins:
            shutil.copy2(plugin, plugin_dir / plugin.name)
        launcher = app / "AppRun"
        launcher.unlink()
        launcher.write_text('#!/bin/sh\nroot="$(dirname "$(readlink -f "$0")")"\n'
                            'export LD_LIBRARY_PATH="$root/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n'
                            'export ALSA_CONFIG_PATH="$root/usr/share/alsa/nori.conf"\n'
                            'export ALSA_PLUGIN_DIR="$root/usr/lib/alsa-lib"\n'
                            'exec "$root/usr/bin/nori-desktop" "$@"\n')
        launcher.chmod(0o755)
        tool = work / "appimagetool.AppImage"
        run("curl", "-fL", f"https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-{arch}.AppImage", "-o", tool)
        tool.chmod(0o755)
        image = output / f"nori-desktop-{version}-linux-{arch}.AppImage"
        run(tool, app, image, env=env)
        result = subprocess.run([str(image), "--appimage-extract-and-run", "--help"], capture_output=True, text=True)
        assert "usage: nori-desktop" in result.stdout + result.stderr, result.stderr
        audio_env = dict(os.environ, LD_LIBRARY_PATH=str(app / "usr/lib"), ALSA_CONFIG_PATH=str(app / "usr/share/alsa/nori.conf"), ALSA_PLUGIN_DIR=str(app / "usr/lib/alsa-lib"))
        run(sys.executable, "-c", "import ctypes; lib = ctypes.CDLL('libasound.so.2'); lib.snd_strerror.restype = ctypes.c_char_p; pcm = ctypes.c_void_p(); result = lib.snd_pcm_open(ctypes.byref(pcm), b'default', 0, 0); assert result == 0, lib.snd_strerror(result); lib.snd_pcm_close(pcm)", env=audio_env)

        run("flatpak", "remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo")
        run("flatpak", "install", "--user", "--noninteractive", "flathub", f"org.freedesktop.Platform//{RUNTIME}", f"org.freedesktop.Sdk//{RUNTIME}")
        bundle = work / "flatpak"
        run("flatpak", "build-init", bundle, APP_ID, "org.freedesktop.Sdk", "org.freedesktop.Platform", RUNTIME)
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
        run("flatpak", "build-export", repo, bundle, "beta" if "-beta." in version else "stable")
        branch = "beta" if "-beta." in version else "stable"
        artifact = output / f"nori-desktop-{version}-linux-{arch}.flatpak"
        run("flatpak", "build-bundle", "--runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo", repo, artifact, APP_ID, branch)
        run("flatpak", "install", "--user", "--noninteractive", artifact)
        result = subprocess.run(["flatpak", "run", "--command=nori-cli", APP_ID, "--help"], capture_output=True, text=True)
        assert "usage: nori-cli" in result.stdout + result.stderr, result.stderr
        result = subprocess.run(["flatpak", "run", APP_ID, "--help"], capture_output=True, text=True)
        assert "usage: nori-desktop" in result.stdout + result.stderr, result.stderr
        sums = output / f"SHA256SUMS-linux-{arch}"
        existing = sums.read_text() if sums.exists() else ""
        names = {image.name, artifact.name}
        existing = "".join(line + "\n" for line in existing.splitlines() if line.split("  ", 1)[1] not in names)
        sums.write_text(existing + "".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in (image, artifact)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    package(args.binaries.resolve(), args.output, args.version)
