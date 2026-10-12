#!/usr/bin/env bash
# Build all desktop/CLI platforms on native builders, from Linux or macOS.
set -euo pipefail
cd "$(dirname "$0")/.."
ref=${1:-$(git branch --show-current)}
output=${2:-build/host-release}
release=${3:-}
run=$(gh api --method POST repos/norifm/nori/actions/workflows/host-release.yml/dispatches \
  -H 'X-GitHub-Api-Version: 2026-03-10' -f ref=master -f "inputs[source]=$ref" -f "inputs[release]=$release" -F return_run_details=true --jq .workflow_run_id)
gh run watch "$run" --repo norifm/nori --exit-status
mkdir -p "$output"
gh run download "$run" --repo norifm/nori --dir "$output"
echo "Desktop and CLI packages: $output"
