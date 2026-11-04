#!/usr/bin/env python3
"""Deletes the GitHub Actions caches no job restores any more, so the repository stays under its 10 GB
and GitHub never evicts a cache a job still needs.

On master, a cache is kept only while it is its family's newest: a new Cargo.lock, Rust or package list
saves another, and the one before is never restored again. A branch's caches are deleted a day after
they were last used. Gradle and npm keep their own.

  tools/ci-caches.py [--dry-run]     needs gh, signed in with actions:write
"""
import json
import subprocess
import sys
from datetime import datetime, timedelta, timezone

MASTER = "refs/heads/master"
# Caches their own actions look after.
OWN = ("gradle-", "node-cache-")
BRANCH_UNUSED = timedelta(days=1)


def family(key):
    """What a cache replaces: the key without its hashes. rust-cache ends a key with the Rust
    environment's hash and the lockfiles' hash; the apt action with the hash of the .debs."""
    parts = key.split("-")
    if key.startswith("v0-rust-"):
        return "-".join(parts[:-2])
    if key.startswith("apt-"):
        return "-".join(parts[:-1])
    return None


def stale(caches, now):
    """The caches to delete."""
    newest = {}
    gone = []
    for c in sorted(caches, key=lambda c: c["createdAt"], reverse=True):
        if c["key"].startswith(OWN):
            continue
        if c["ref"] != MASTER:
            if now - datetime.fromisoformat(c["lastAccessedAt"]) > BRANCH_UNUSED:
                gone.append(c)
            continue
        f = family(c["key"])
        if f is None:
            continue
        if f in newest:
            gone.append(c)
        else:
            newest[f] = c
    return gone


def main():
    dry = "--dry-run" in sys.argv[1:]
    out = subprocess.check_output(["gh", "cache", "list", "--limit", "1000", "--json", "id,key,ref,sizeInBytes,createdAt,lastAccessedAt"])
    gone = stale(json.loads(out), datetime.now(timezone.utc))
    for c in gone:
        print(f"{'would delete' if dry else 'deleting'} {c['key']} ({c['ref']}, {c['sizeInBytes'] / 1e6:.0f} MB)")
        if not dry:
            subprocess.run(["gh", "cache", "delete", str(c["id"])], check=False)
    print(f"{sum(c['sizeInBytes'] for c in gone) / 1e9:.2f} GB in {len(gone)} caches")


if __name__ == "__main__":
    main()
