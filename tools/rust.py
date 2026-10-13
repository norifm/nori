#!/usr/bin/env python3
"""Build, run, or test native Rust targets with the experimental Bazel backend."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run", "test"))
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument("-p", "--package", action="append")
    selection.add_argument("--workspace", action="store_true")
    parser.add_argument("-j", "--jobs", type=int, choices=range(1, 5), default=4)
    argv = sys.argv[1:]
    split = argv.index("--") if "--" in argv else len(argv)
    args = parser.parse_args(argv[:split])
    forwarded = argv[split + 1:]
    if args.command == "build" and forwarded:
        parser.error("build does not accept program arguments")
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"], cwd=ROOT))
    packages = {p["name"]: p for p in metadata["packages"] if p["id"] in metadata["workspace_members"]}
    selected = list(packages) if args.workspace else args.package
    if args.command == "run" and len(selected) != 1:
        parser.error("run requires one package")
    targets = []
    for name in selected:
        if name not in packages:
            parser.error(f"unknown workspace crate: {name}")
        package = packages[name]
        folder = Path(package["manifest_path"]).parent.relative_to(ROOT)
        if args.command == "test":
            targets.append(f"//{folder}:all")
        else:
            native = [t for t in package["targets"] if t["kind"][0] in ("bin", "lib", "proc-macro", "staticlib", "cdylib")]
            if args.command == "run":
                native = [t for t in native if t["kind"][0] == "bin"]
                if len(native) != 1:
                    parser.error(f"{name} must have one binary to run")
            targets += [f"//{folder}:" + t["name"] for t in native]
    subprocess.run([sys.executable, str(ROOT / "tools/bazel/generate.py")], cwd=ROOT, check=True)
    client = "desktop" if set(selected) <= {"nori-cli", "nori-desktop"} else "tests"
    if args.command == "test" and client == "desktop":
        client += "-tests"
    command = [str(ROOT / "tools/check.sh"), "bazel", args.command,
               f"--//tools/bazel:client={client}", f"--jobs={args.jobs}", *targets]
    if args.command == "test":
        command += ["--test_arg=" + argument for argument in forwarded]
    elif args.command == "run" and forwarded:
        command += ["--", *forwarded]
    raise SystemExit(subprocess.run(command, cwd=ROOT).returncode)


if __name__ == "__main__":
    main()
