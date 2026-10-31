#!/usr/bin/env python3
"""The crates a change can affect, as `-p` flags for cargo: those it touches and all that depend on them.

Reads changed paths on stdin. Prints nothing when the whole workspace needs running: no paths, a list
cut short, or a change every crate sees.
"""
import json
import subprocess
import sys

# GitHub's compare lists this many files at most.
LISTED = 300
# Seen by every crate.
EVERYWHERE = ("Cargo.", "vendor/", "wit/", ".cargo/", ".config/", ".github/", "tools/affected.py")
# Read by one crate's build or tests from outside it.
READ_BY = {
    "core/": "nori-android",
    "AGENTS.md": "nori-testdir",
    "app/src/main/assets/": "nori-settings",
    "app/src/main/res/font/": "nori-desktop",
    "docs/brand/": "nori-desktop",
}
# Its tests look at every crate.
ALWAYS = "nori-testdir"


def workspace():
    """Each member's directory, and who depends on whom: `{name: those depending on it}`."""
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version=1", "--no-deps"]))
    root = meta["workspace_root"].rstrip("/") + "/"
    dirs = {p["manifest_path"].removeprefix(root).removesuffix("Cargo.toml"): p["name"] for p in meta["packages"]}
    names = set(dirs.values())
    users = {n: set() for n in names}
    for p in meta["packages"]:
        for d in p["dependencies"]:
            if d["name"] in names:
                users[d["name"]].add(p["name"])
    return dirs, users


def affected(paths, dirs, users):
    """The crates to run, or None for all of them."""
    if not paths or len(paths) >= LISTED or any(p.startswith(EVERYWHERE) for p in paths):
        return None
    touched = set()
    for path in paths:
        touched |= {name for d, name in dirs.items() if path.startswith(d)}
        touched |= {name for prefix, name in READ_BY.items() if path.startswith(prefix)}
    pending, seen = list(touched), set(touched)
    while pending:
        for user in users[pending.pop()] - seen:
            seen.add(user)
            pending.append(user)
    return seen | {ALWAYS}


if __name__ == "__main__":
    crates = affected([line.strip() for line in sys.stdin if line.strip()], *workspace())
    if crates is not None:
        print(" ".join(f"-p {c}" for c in sorted(crates)))
