#!/usr/bin/env bash
# One heavy host check at a time across worktrees; test selection stays Cargo's.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/.."
case "${1:-}" in
  rust) shift; exec python3 "$here/with-resource.py" build cargo nextest run --build-jobs 4 --test-threads 4 "$@" ;;
  rust-doc) shift; exec python3 "$here/with-resource.py" build cargo test -j4 --doc "$@" ;;
  web)
    # CI's wasm clippy, both modules; the NDK's clang builds the wasm C parts.
    cc=$(ls -d "$HOME"/Library/Android/sdk/ndk/*/toolchains/llvm/prebuilt/*/bin/clang 2>/dev/null | sort -V | tail -1)
    if [ -n "$cc" ]; then export CC_wasm32_unknown_unknown="$cc" AR_wasm32_unknown_unknown="$(dirname "$cc")/llvm-ar"; fi
    for f in "" --no-default-features; do
      python3 "$here/with-resource.py" build cargo clippy -j4 --target wasm32-unknown-unknown -p nori-web --all-targets $f -- -D warnings
    done ;;
  # The iPod's host, without the desktop parts a workspace build turns on.
  ipod) exec python3 "$here/with-resource.py" build cargo clippy -j4 -p nori-ios --all-targets -- -D warnings ;;
  android) shift; exec python3 "$here/with-resource.py" build ./gradlew :app:assembleDebug "$@" ;;
  bazel) shift; exec python3 "$here/with-resource.py" build bazel "$@" ;;
  *) echo "usage: check.sh rust [--workspace|-p CRATE ...]|rust-doc [...]|web|ipod|android [GRADLE OPTIONS]|bazel COMMAND [...]" >&2; exit 2 ;;
esac
