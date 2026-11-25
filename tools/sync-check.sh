#!/usr/bin/env bash
# Pairing and profile sync against a real sync server, the self-hosted program of the neighbouring
# nori-sync checkout (or NORI_SYNC_ROOT), with headless nori-cli instances (tools/sync-check.py).
#   tools/sync-check.sh                    the terminal client only
#   tools/sync-check.sh --android SERIAL   and the app's debug build on that device or emulator
#   tools/sync-check.sh --web URL [--shots DIR]  and the web client served there (web/check/sync.mjs)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
common=$(git -C "$here" rev-parse --path-format=absolute --git-common-dir)
sync_root="${NORI_SYNC_ROOT:-$(dirname "$(dirname "$common")")/nori-sync}"
[ -f "$sync_root/PROTOCOL.md" ] || { echo "set NORI_SYNC_ROOT to a nori-sync checkout" >&2; exit 1; }
target="${CARGO_TARGET_DIR:-$here/../target}"
python3 "$here/with-resource.py" build cargo build -j4 -p nori-cli
python3 "$here/with-resource.py" build bash -c "cd '$sync_root' && cargo build -j4 --release -p nori-sync-server"
exec python3 "$here/sync-check.py" "$target/debug/nori-cli" "$sync_root/target/release/nori-sync-server" "$@"
