#!/usr/bin/env bash
set -euo pipefail
compiler=clang
if [[ "$0" == *cxx ]]; then compiler=clang++; fi
exec "$compiler" -target "$NORI_MACOS_TARGET" -isysroot "$SDKROOT" \
  -mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET" -fuse-ld=lld "$@"
