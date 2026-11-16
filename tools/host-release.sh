#!/usr/bin/env bash
# The same desktop/CLI builders run locally, in Docker, and on CI workers.
# They run in build/release-python, made from requirements.txt whenever it changes; uv downloads the
# Python for it, else a python3 of 3.10 or later is used.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
env="$root/build/release-python"
requirements="$root/tools/host-release/requirements.txt"
# A venv keeps its Python in bin/, or in Scripts/ on Windows.
venv_python() { if [ -d "$env/Scripts" ]; then echo "$env/Scripts/python.exe"; else echo "$env/bin/python"; fi; }
if ! cmp -s "$requirements" "$env/requirements.txt"; then
  rm -rf "$env"
  if command -v uv >/dev/null; then
    uv venv --quiet --python 3.12 "$env"
    uv pip install --quiet --python "$(venv_python)" -r "$requirements"
  else
    python=$(command -v python3 || command -v python) \
      && "$python" -c 'import sys; sys.exit(sys.version_info < (3, 10))' \
      || { echo "the release builders need uv or Python 3.10+" >&2; exit 1; }
    "$python" -m venv "$env"
    "$(venv_python)" -m pip install --quiet --disable-pip-version-check -r "$requirements"
  fi
  cp "$requirements" "$env/requirements.txt"
fi
exec "$(venv_python)" "$root/tools/host-release/build.py" "$@"
