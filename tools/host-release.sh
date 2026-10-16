#!/usr/bin/env bash
# The same desktop/CLI builders run locally, in Docker, and on CI workers.
# They run in build/release-python, made from requirements.txt whenever it changes; uv downloads the
# Python for it, else a python3 of 3.10 or later is used.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
env="$root/build/release-python"
requirements="$root/tools/host-release/requirements.txt"
if ! cmp -s "$requirements" "$env/requirements.txt"; then
  rm -rf "$env"
  if command -v uv >/dev/null; then
    uv venv --quiet --python 3.12 "$env"
    uv pip install --quiet --python "$env/bin/python" -r "$requirements"
  else
    python3 -c 'import sys; sys.exit(sys.version_info < (3, 10))' \
      || { echo "the release builders need uv or Python 3.10+" >&2; exit 1; }
    python3 -m venv "$env"
    "$env/bin/python" -m pip install --quiet --disable-pip-version-check -r "$requirements"
  fi
  cp "$requirements" "$env/requirements.txt"
fi
exec "$env/bin/python" "$root/tools/host-release/build.py" "$@"
