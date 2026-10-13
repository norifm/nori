#!/usr/bin/env bash
set -euo pipefail
mkdir -p /cache/cargo /cache/target /cache/sccache
chown build:build /cache /cache/cargo /cache/target /cache/sccache
mkdir -p /run/dbus
dbus-daemon --system --fork
exec runuser -u build -- "$@"
