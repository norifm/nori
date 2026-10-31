#!/usr/bin/env bash
# The web client into web/dist: core.wasm (nori-web without the engine: sign-in and the library),
# player.wasm (with it: the shared session and the engine, loaded on the first Play, decoding with the
# browser's WebCodecs) and player-codecs.wasm (the same with Vorbis, ALAC and Opus decoded in Rust, for a
# browser without them), each with its wasm-bindgen glue and run through wasm-opt -Oz; then the shell
# (Svelte 5, type-checked with svelte-check and built with Vite; web/package.json pins them). Prints
# each piece's raw and brotli size, the first paint's and the full load's. Every file gets .br and .gz
# twins for the server to send.
# Needs: the wasm32-unknown-unknown target, wasm-bindgen-cli matching Cargo.lock, wasm-opt (binaryen),
# brotli, node 20 or later with npm, and a clang that targets wasm32 for SQLite's C (Homebrew LLVM, or
# the Android NDK's; Apple's clang has no wasm32 backend).
# `web/build.sh --shell` builds the shell again over the modules already in web/dist/pkg.
set -euo pipefail
shell_only=false
[ "${1:-}" = "--shell" ] && shell_only=true
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/.."
dist="$here/dist"

if [ -z "${CC_wasm32_unknown_unknown:-}" ]; then
  for cc in "$(brew --prefix llvm 2>/dev/null)/bin/clang" "$(ls -d "$HOME"/Library/Android/sdk/ndk/*/toolchains/llvm/prebuilt/*/bin/clang 2>/dev/null | sort -V | tail -1)"; do
    if [ -x "$cc" ] && "$cc" --print-targets 2>/dev/null | grep -q wasm32; then
      export CC_wasm32_unknown_unknown="$cc" AR_wasm32_unknown_unknown="$(dirname "$cc")/llvm-ar"
      break
    fi
  done
fi
$shell_only || [ -n "${CC_wasm32_unknown_unknown:-}" ] || { echo "no clang with a wasm32 target: brew install llvm" >&2; exit 1; }

# SQLite as the core uses it (FTS5, JSON): sqlite-wasm-rs's extras it never asks for left out (R-tree,
# sessions, the introspection tables, UTF-16, column metadata), and its C optimized for size.
export CFLAGS_wasm32_unknown_unknown="${CFLAGS_wasm32_unknown_unknown:-} -USQLITE_ENABLE_RTREE -USQLITE_ENABLE_SESSION \
  -USQLITE_ENABLE_PREUPDATE_HOOK -USQLITE_ENABLE_BYTECODE_VTAB -USQLITE_ENABLE_DBPAGE_VTAB -USQLITE_ENABLE_DBSTAT_VTAB \
  -USQLITE_ENABLE_STMTVTAB -USQLITE_ENABLE_COLUMN_METADATA -USQLITE_ENABLE_UNKNOWN_SQL_FUNCTION -USQLITE_ENABLE_OFFSET_SQL_FUNC \
  -USQLITE_ENABLE_MATH_FUNCTIONS -USQLITE_ENABLE_API_ARMOR -USQLITE_ENABLE_UNLOCK_NOTIFY -DSQLITE_OMIT_PROGRESS_CALLBACK \
  -DSQLITE_OMIT_COMPLETE -DSQLITE_OMIT_GET_TABLE -DSQLITE_OMIT_TCL_VARIABLE -DSQLITE_OMIT_UTF16 -DSQLITE_DQS=0 \
  -DSQLITE_DEFAULT_MEMSTATUS=0 -DSQLITE_MAX_EXPR_DEPTH=0 -Oz"

# A file's size as sent: its brotli twin's, or its own for a font (already compressed).
sent() { wc -c < "$([ -f "$1.br" ] && echo "$1.br" || echo "$1")"; }
size() { printf '%-22s %9d raw %8d br\n' "$1" "$(wc -c < "$2")" "$(sent "$2")"; }

if $shell_only; then
  [ -f "$dist/pkg/player.wasm" ] || { echo "no modules in $dist/pkg: build them first (web/build.sh)" >&2; exit 1; }
  find "$dist" -mindepth 1 -maxdepth 1 ! -name pkg -exec rm -rf {} +
else
  rm -rf "$dist"
fi
mkdir -p "$dist/pkg" "$here/src/pkg"

# One module: `name` built with cargo's `flags`.
module() {
  local name="$1"; shift
  python3 "$root/tools/with-resource.py" build cargo build -j4 --profile web --target wasm32-unknown-unknown -p nori-web "$@"
  wasm-bindgen --target web --out-dir "$dist/pkg" --out-name "$name" "$root/target/wasm32-unknown-unknown/web/nori_web.wasm"
  mv "$dist/pkg/$name.d.ts" "$here/src/pkg/$name.d.ts"
  rm -f "$dist/pkg/${name}_bg.wasm.d.ts"
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types \
    "$dist/pkg/${name}_bg.wasm" -o "$dist/pkg/$name.wasm"
  rm "$dist/pkg/${name}_bg.wasm"
  sed -i.bak "s/${name}_bg\.wasm/$name.wasm/g" "$dist/pkg/$name.js" && rm "$dist/pkg/$name.js.bak"
}

if ! $shell_only; then
  module core --no-default-features
  module player
  module player-codecs --features rust-codecs
fi

(
  cd "$here"
  [ node_modules/.package-lock.json -nt package-lock.json ] || npm ci --no-audit --no-fund
  # Inter and Literata are the Android app's files, cut into WOFF2 for the shell to bundle (not kept in git here).
  node fonts.mjs
  npx svelte-check --tsconfig ./tsconfig.json --fail-on-warnings --output human
  npx vite build --logLevel warn
  npx vite build --mode decoder --logLevel warn
)

# The service worker keeps this build's shell and modules (the Rust-codecs player once fetched), under
# the hash of them all.
# Not the plugin host's generator and worker, fetched only with a plugin, nor the fonts past Latin,
# kept once a page needs them.
kept=$(cd "$dist" && ls index.html decoder.js $(ls assets/* | grep -Ev 'js-component-bindgen|/(install|runner|hls)-|-rest-') pkg/core.js pkg/core.wasm pkg/player.js pkg/player.wasm pkg/player-codecs.js)
build=$(cd "$dist" && cat $kept pkg/player-codecs.wasm | shasum -a 256 | cut -c1-16)
files=$(printf '%s\n' $kept | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().split()))')
sed -i.bak -e "s|__BUILD__|$build|" -e "s|__FILES__|$files|" "$dist/sw.js" && rm "$dist/sw.js.bak"
echo "build $build"
# Every file beside its brotli and gzip twins, for a server to send as they are.
find "$dist" -type f ! -name '*.woff2' ! -name '*.br' ! -name '*.gz' | while read -r f; do
  [ "$f.br" -nt "$f" ] || brotli -q 11 -f -k "$f"
  [ "$f.gz" -nt "$f" ] || gzip -9 -f -k "$f"
done
size "core.wasm" "$dist/pkg/core.wasm"
size "core.js (glue)" "$dist/pkg/core.js"
size "player.wasm" "$dist/pkg/player.wasm"
size "player.js (glue)" "$dist/pkg/player.js"
size "player-codecs.wasm" "$dist/pkg/player-codecs.wasm"
for f in "$dist/index.html" "$dist/sw.js" "$dist/decoder.js" $(ls "$dist"/assets/* | grep -Ev '\.(br|gz)$'); do size "${f#"$dist/"}" "$f"; done
total() { printf '%-22s %9d raw %8d br\n' "$1" "$(cd "$dist" && cat $2 | wc -c)" "$(cd "$dist" && for f in $2; do sent "$f"; done | awk '{s+=$1} END {print s}')"; }
# The first paint: the page, its script and its style (the workers and the rest come after it).
total "first paint" "index.html $(cd "$dist" && grep -o 'assets/[^"]*' index.html | sort -u)"
# Everything a first visit ends up fetching.
total "full load" "$kept sw.js"
