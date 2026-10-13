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


def confirm_cross_tests(allowed):
    print("The Linux cross-builder can compile macOS packages, but cannot run Mac rendering tests or execute the Mac apps.", flush=True)
    if allowed:
        return
    if not sys.stdin.isatty():
        raise SystemExit("Use --allow-untested-macos to acknowledge these skipped checks on a worker.")
    if input("Build macOS packages with these checks skipped? [y/N] ").strip().lower() not in ("y", "yes"):
        raise SystemExit("macOS cross-build canceled")


def macos_container(workspace, output, target_platform, jobs, package_only):
    run("docker", "build", "--build-arg", f"NORI_UID={os.getuid()}", "--build-arg", f"NORI_GID={os.getgid()}",
        "-f", ROOT / "tools/host-release/Dockerfile.macos", "-t", "nori-macos-build", ROOT / "tools/host-release")
    output.parent.mkdir(parents=True, exist_ok=True)
    run("docker", "run", "--rm", "--init",
        "--mount", f"type=bind,source={workspace},target=/workspace,readonly",
        "--mount", f"type=bind,source={ROOT},target=/tooling,readonly",
        "--mount", f"type=bind,source={output.parent},target=/output",
        "--mount", "type=volume,source=nori-macos-cache,target=/cache",
        "--env", "CARGO_HOME=/cache/cargo", "--env", "CARGO_TARGET_DIR=/cache/target",
        "--workdir", "/workspace", "nori-macos-build",
        "python3", "/tooling/tools/host-release/build.py", "--cross-macos", "--platform", target_platform,
        "--workspace", "/workspace", "--output", "/output", "--jobs", str(jobs),
        *(["--package-only"] if package_only else []))


def cross_macos(workspace, output, target_platform, jobs, package_only):
    arch = target_platform.split("-", 1)[1]
    target = f"{arch}-apple-darwin"
    key = target.replace("-", "_")
    environment = dict(os.environ, RUSTC_WRAPPER="", NORI_MACOS_TARGET=target)
    environment.update({f"CARGO_TARGET_{key.upper()}_LINKER": "macos-cc",
                        f"CC_{key}": "macos-cc", f"CXX_{key}": "macos-cxx", f"AR_{key}": "llvm-ar"})
    binaries = Path(environment["CARGO_TARGET_DIR"]) / target / "release"
    if not package_only:
        run("cargo", "build", "--locked", "--release", f"-j{jobs}", "--target", target,
            "-p", "nori-cli", "-p", "nori-desktop", cwd=workspace, env=environment)
    version = re.search(r'^version = "([^"]+)"', (workspace / "Cargo.toml").read_text(), re.M).group(1)
    run(sys.executable, ROOT / "tools/host-release/package.py", "--binaries", binaries,
        "--version", version, "--system", "macos", "--arch", arch, "--output", output, env=environment)
    run(sys.executable, ROOT / "tools/host-release/check.py", "--checksums-only", output, env=environment)


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
        run("pulseaudio", "--start", "--exit-idle-time=-1", env=environment)
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
    flatpak_environment = []
    if arch != ARCHES[platform.machine()]:
        flatpak_environment = ["--env", "FLATPAK_BWRAP=/tooling/tools/host-release/emulated-bwrap.py"]
    run("docker", "run", "--rm", "--init", "--privileged", "--platform", docker_platform,
        "--mount", f"type=bind,source={workspace},target=/workspace,readonly",
        "--mount", f"type=bind,source={ROOT},target=/tooling,readonly",
        "--mount", f"type=bind,source={output.parent},target=/output",
        "--mount", f"type=volume,source={cache},target=/cache",
        "--mount", f"type=volume,source=nori-host-{arch}-home,target=/home/build",
        "--env", "CARGO_HOME=/cache/cargo", "--env", "CARGO_TARGET_DIR=/cache/target",
        "--env", "SCCACHE_DIR=/cache/sccache", *flatpak_environment, "--workdir", "/workspace", image, *command)


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
    parser.add_argument("--docker-macos", action="store_true", help="cross-compile macOS in Docker, including on a Mac")
    parser.add_argument("--allow-untested-macos", action="store_true", help="acknowledge skipped macOS execution checks on Linux")
    parser.add_argument("--preflight", action="store_true", help="check requirements and confirm skipped checks before building")
    parser.add_argument("--cross-macos", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    selected = args.platforms or list(PLATFORMS)
    if args.native and not args.platforms:
        parser.error("--native requires --platform")
    cross_platforms = {p for p in selected if p.startswith("macos-") and
                       (args.docker_macos or platform.system() != "Darwin" or
                        ARCHES[platform.machine()] == "x86_64" and p.endswith("aarch64"))}
    if not args.cross_macos and cross_platforms:
        if args.native:
            parser.error("this Mac target needs the Linux Docker cross-builder; omit --native")
        confirm_cross_tests(args.allow_untested_macos)
    if not args.native and not args.cross_macos and (cross_platforms or any(p.startswith("linux-") for p in selected)) and not shutil.which("docker"):
        parser.error("local cross-platform packages need Docker")
    if args.preflight:
        return
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
            if args.cross_macos:
                cross_macos(workspace, target_output, target_platform, args.jobs, args.package_only)
            elif target_platform in cross_platforms:
                macos_container(workspace, target_output, target_platform, args.jobs, args.package_only)
            elif args.native or target_platform.startswith("macos-"):
                native(workspace, target_output, target_platform, args.jobs, not args.no_test, args.package_only, target_dir)
            else:
                container(workspace, target_output, target_platform, args.jobs, not args.no_test, args.package_only)
    print(f"Desktop and CLI packages: {output}")


if __name__ == "__main__":
    main()
