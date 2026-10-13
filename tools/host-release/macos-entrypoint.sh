#!/usr/bin/env bash
set -euo pipefail
mkdir -p /cache/cargo /cache/target
chown build:build /cache /cache/cargo /cache/target
exec runuser -u build -- "$@"
