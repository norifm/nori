#!/usr/bin/env bash
# The web client into web/dist: core.wasm (nori-web, wasm-bindgen glue, wasm-opt -Oz), the TypeScript
# shell compiled with tsc, and the static files; prints each piece's raw and brotli size.
# Needs: the wasm32-unknown-unknown target, wasm-bindgen-cli matching Cargo.lock, wasm-opt (binaryen),
# brotli, node (npx tsc), and a clang that targets wasm32 for SQLite's C (Homebrew LLVM, or the
# Android NDK's; Apple's clang has no wasm32 backend).
set -euo pipefail
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
[ -n "${CC_wasm32_unknown_unknown:-}" ] || { echo "no clang with a wasm32 target: brew install llvm" >&2; exit 1; }

# NORI_WEB_CORE_ONLY=1: the module without the engine, to measure it; the page needs the engine, so
# only core.wasm is made.
features=()
[ "${NORI_WEB_CORE_ONLY:-}" = 1 ] && features=(--no-default-features)
python3 "$root/tools/with-resource.py" build cargo build -j4 --profile web --target wasm32-unknown-unknown -p nori-web ${features[@]+"${features[@]}"}

rm -rf "$dist"
mkdir -p "$dist/pkg" "$here/src/pkg"
wasm-bindgen --target web --out-dir "$dist/pkg" --out-name core "$root/target/wasm32-unknown-unknown/web/nori_web.wasm"
mv "$dist/pkg/core.d.ts" "$here/src/pkg/core.d.ts"
rm -f "$dist/pkg/core_bg.wasm.d.ts"
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types \
  "$dist/pkg/core_bg.wasm" -o "$dist/pkg/core.wasm"
rm "$dist/pkg/core_bg.wasm"
sed -i.bak 's/core_bg\.wasm/core.wasm/g' "$dist/pkg/core.js" && rm "$dist/pkg/core.js.bak"
size() { printf '%-22s %9d raw %8d br\n' "$1" "$(wc -c < "$2")" "$(brotli -q 11 -c "$2" | wc -c)"; }
size "core.wasm" "$dist/pkg/core.wasm"
[ "${NORI_WEB_CORE_ONLY:-}" = 1 ] && exit 0
(cd "$here" && npx -y -p typescript@5.6.3 tsc -p tsconfig.json)
cp "$here/index.html" "$here/style.css" "$dist/"
size "core.js (glue)" "$dist/pkg/core.js"
size "worker.js" "$dist/worker.js"
size "worklet.js" "$dist/worklet.js"
size "app.js" "$dist/app.js"
size "index.html" "$dist/index.html"
size "style.css" "$dist/style.css"
