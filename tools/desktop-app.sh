#!/usr/bin/env bash
# Builds the desktop client as a macOS app, ~/Applications/nori.app (Launchpad and Spotlight list it): the
# release binary, the icon from docs/brand/nori-macos.png, an Info.plist, an ad-hoc signature.
#   tools/desktop-app.sh            build and install
#   tools/desktop-app.sh --no-build bundle the binary already in target/release
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
app="${NORI_APP_DIR:-$HOME/Applications}/nori.app"
version=${NORI_VERSION:-$(grep -oE '^version = "[^"]+"' "$root/Cargo.toml" | head -1 | cut -d'"' -f2)}

[ "${1:-}" = --no-build ] || (cd "$root" && cargo build -j4 --release -p nori-desktop)

python3 "$root/tools/host-release/app.py" \
  --binary "${NORI_DESKTOP_BINARY:-$root/target/release/nori-desktop}" \
  --directory "$(dirname "$app")" --version "$version"
echo "$app (version $version)"
