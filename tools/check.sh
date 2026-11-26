#!/usr/bin/env bash
# One heavy host check at a time across worktrees; test selection stays Cargo's.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/.."
case "${1:-}" in
  rust) shift; exec python3 "$here/with-resource.py" build cargo nextest run --build-jobs 4 --test-threads 4 "$@" ;;
  # Tests and clippy of the crates this worktree's change reaches (tools/affected.py), against where it
  # left master; the whole workspace when the change reaches every crate.
  changed)
    base=$(git merge-base origin/master HEAD)
    crates=$( { git diff --name-only "$base"; git ls-files --others --exclude-standard; } | python3 "$here/affected.py")
    # shellcheck disable=SC2086 # one word per flag
    python3 "$here/with-resource.py" build cargo nextest run --build-jobs 4 --test-threads 4 ${crates:---workspace}
    # shellcheck disable=SC2086
    exec python3 "$here/with-resource.py" build cargo clippy -j4 ${crates:---workspace} --all-targets -- -D warnings ;;
  clippy) shift; exec python3 "$here/with-resource.py" build cargo clippy -j4 --all-targets "$@" -- -D warnings ;;
  rust-doc) shift; exec python3 "$here/with-resource.py" build cargo test -j4 --doc "$@" ;;
  web)
    # CI's wasm clippy, both modules; the NDK's clang builds the wasm C parts.
    sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$(sed -n 's/^sdk\.dir=//p' local.properties 2>/dev/null)}}"
    cc=$(ls -d "${sdk:-$HOME/Library/Android/sdk}"/ndk/*/toolchains/llvm/prebuilt/*/bin/clang 2>/dev/null | sort -V | tail -1 || true)
    if [ -n "$cc" ]; then export CC_wasm32_unknown_unknown="$cc" AR_wasm32_unknown_unknown="$(dirname "$cc")/llvm-ar"; fi
    for f in "" --no-default-features; do
      python3 "$here/with-resource.py" build cargo clippy -j4 --target wasm32-unknown-unknown -p nori-web --all-targets $f -- -D warnings
    done ;;
  # The iPod's host, without the desktop parts a workspace build turns on.
  ipod) exec python3 "$here/with-resource.py" build cargo clippy -j4 -p nori-ios --all-targets -- -D warnings ;;
  android) shift; exec python3 "$here/with-resource.py" build ./gradlew :app:assembleDebug "$@" ;;
  bazel) shift; exec python3 "$here/with-resource.py" build bazel "$@" ;;
  *) echo "usage: check.sh changed|rust [--workspace|-p CRATE ...]|clippy [--workspace|-p CRATE ...]|rust-doc [...]|web|ipod|android [GRADLE OPTIONS]|bazel COMMAND [...]" >&2; exit 2 ;;
esac
