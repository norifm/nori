#!/usr/bin/env bash
# The same desktop/CLI builders run locally, in Docker, and on CI workers.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$root/tools/host-release/build.py" "$@"
