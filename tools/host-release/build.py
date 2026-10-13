#!/usr/bin/env python3
"""Build and verify desktop/CLI packages locally or on one native worker."""
import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PLATFORMS = ("macos-aarch64", "macos-x86_64", "linux-aarch64", "linux-x86_64")
ARCHES = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}


def run(*args, **kwargs):
    print("+ " + " ".join(str(arg) for arg in args), flush=True)
    subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def native(workspace, output, target_platform, jobs, test, package_only, target_dir):
    system, arch = target_platform.split("-", 1)
    current = {"Darwin": "macos", "Linux": "linux"}[platform.system()]
    if system != current or (system == "linux" and arch != ARCHES[platform.machine()]):
        raise SystemExit(f"{target_platform} requires a matching native worker")
    environment = dict(os.environ)
    target_dir = Path(environment.get("CARGO_TARGET_DIR", target_dir)).resolve()
    environment["CARGO_TARGET_DIR"] = str(target_dir)
    cross = arch != ARCHES[platform.machine()]
    target = f"{arch}-apple-darwin" if cross else None
    cargo_target = ["--target", target] if target else []
    binaries = target_dir / target / "release" if target else target_dir / "release"
    if not package_only:
        if target:
            run("rustup", "target", "add", target)
        if test:
            run("cargo", "test", "--locked", "--release", f"-j{jobs}", *cargo_target,
                "-p", "nori-cli", "-p", "nori-desktop", "--", "--test-threads=1", cwd=workspace, env=environment)
        run("cargo", "build", "--locked", "--release", f"-j{jobs}", *cargo_target,
            "-p", "nori-cli", "-p", "nori-desktop", cwd=workspace, env=environment)
    version = re.search(r'^version = "([^"]+)"', (workspace / "Cargo.toml").read_text(), re.M).group(1)
    run(sys.executable, ROOT / "tools/host-release/package.py", "--binaries", binaries,
        "--version", version, "--arch", arch, "--output", output, env=environment)
    if system == "linux":
        run("pulseaudio", "--start", env=environment)
        run("dbus-run-session", "--", sys.executable, ROOT / "tools/host-release/linux.py",
            "--binaries", binaries, "--version", version, "--output", output, env=environment)
    run(sys.executable, ROOT / "tools/host-release/check.py", output, env=environment)


def container(workspace, output, target_platform, jobs, test, package_only):
    arch = target_platform.split("-", 1)[1]
    docker_platform = {"aarch64": "linux/arm64", "x86_64": "linux/amd64"}[arch]
    image = f"nori-host-build:{arch}"
    cache = f"nori-host-{arch}-cache"
    run("docker", "build", "--platform", docker_platform, "--build-arg", f"NORI_UID={os.getuid()}",
        "--build-arg", f"NORI_GID={os.getgid()}", "-t", image, ROOT / "tools/host-release")
    output.mkdir(parents=True, exist_ok=True)
    command = ["python3", "/tooling/tools/host-release/build.py", "--workspace", "/workspace", "--native",
               "--platform", target_platform, "--output", "/output", "--jobs", str(jobs)]
    if not test:
        command.append("--no-test")
    if package_only:
        command.append("--package-only")
    run("docker", "run", "--rm", "--init", "--privileged", "--platform", docker_platform,
        "--mount", f"type=bind,source={workspace},target=/workspace,readonly",
        "--mount", f"type=bind,source={ROOT},target=/tooling,readonly",
        "--mount", f"type=bind,source={output.parent},target=/output",
        "--mount", f"type=volume,source={cache},target=/cache",
        "--mount", f"type=volume,source=nori-host-{arch}-home,target=/home/build",
        "--env", "CARGO_HOME=/cache/cargo", "--env", "CARGO_TARGET_DIR=/cache/target",
        "--env", "SCCACHE_DIR=/cache/sccache", "--workdir", "/workspace", image, *command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", action="append", choices=PLATFORMS, dest="platforms")
    parser.add_argument("--native", action="store_true", help="build only on the current worker, without Docker")
    parser.add_argument("--workspace", type=Path, default=ROOT, help="source checkout to compile")
    parser.add_argument("--source", help="build a committed revision instead of the working tree")
    parser.add_argument("--output", type=Path, default=ROOT / "build/host-release")
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=min(4, int(os.environ.get("CARGO_BUILD_JOBS", "4"))))
    parser.add_argument("--no-test", action="store_true")
    parser.add_argument("--package-only", action="store_true", help="package already built binaries and verify them")
    args = parser.parse_args()
    selected = args.platforms or list(PLATFORMS)
    if args.native and not args.platforms:
        parser.error("--native requires --platform")
    if platform.system() != "Darwin" and any(p.startswith("macos-") for p in selected):
        parser.error("the full local release needs a Mac; select Linux platforms on Linux workers")
    if not args.native and any(p.startswith("linux-") for p in selected) and not shutil.which("docker"):
        parser.error("local Linux packages need Docker")
    output = args.output.resolve()
    workspace = args.workspace.resolve()
    target_dir = workspace / "target"
    with tempfile.TemporaryDirectory(prefix="nori-host-source-") as directory:
        if args.source:
            archive = subprocess.check_output(["git", "archive", args.source], cwd=workspace)
            workspace = Path(directory)
            run("tar", "-xf", "-", "-C", workspace, input=archive)
        for target_platform in selected:
            target_output = output / target_platform
            if args.native or target_platform.startswith("macos-"):
                native(workspace, target_output, target_platform, args.jobs, not args.no_test, args.package_only, target_dir)
            else:
                container(workspace, target_output, target_platform, args.jobs, not args.no_test, args.package_only)
    print(f"Desktop and CLI packages: {output}")


if __name__ == "__main__":
    main()
