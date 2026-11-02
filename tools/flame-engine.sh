#!/usr/bin/env bash
# Flamegraphs of the engine's perf bench cases (crates/engine/tests/perf_bench.rs), on the host.
#
#   tools/flame-engine.sh [--heap] <case[,case]> [minutes=4] [out=build/flame/<case>]
#
# Cases: plain equalizer eq10 compressor mp3 flac speed silence gain gaineq change crossfade automix, and
# stage: a plugin's sound stage through wasmtime (perf_stage.rs).
# CPU (default): samples the engine's threads (nori-*, not the test making the music) with tools/flame.py:
# flame.svg, stacks.folded, profile.json.gz. --heap: where the engine allocates, by bytes (a stack per
# 64 KB a thread allocates): heap-<case>.svg and <case>.folded.
set -euo pipefail
cd "$(dirname "$0")/.."
heap=
if [ "${1:-}" = --heap ]; then heap=1; shift; fi
case=${1:?case, e.g. automix}
minutes=${2:-4}
out=${3:-build/flame/$case}
mkdir -p "$out"
bin=$(cargo test --profile profiling -j4 -p nori-engine --test engine --no-run --message-format=json 2>/dev/null \
    | python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "engine" and m.get("executable"): print(m["executable"])' | tail -1)
[ -n "$bin" ] || { cargo test --profile profiling -j4 -p nori-engine --test engine --no-run; exit 1; }
test=engine::perf_report
[ "$case" = stage ] && test=engine::perf_stage
run=("$bin" --exact "$test" --ignored --nocapture --test-threads=1)
if [ -z "$heap" ]; then
    NORI_PERF=$case NORI_PERF_MINUTES=$minutes python3 tools/flame.py -o "$out" --thread nori- -- "${run[@]}"
    exit
fi
NORI_PERF=$case NORI_PERF_MINUTES=$minutes NORI_HEAP_STACKS=$out "${run[@]}" | grep 'perf:' || true
for f in "$out"/*.folded; do
    [ "$(basename "$f")" = stacks.folded ] && continue
    name=$(basename "$f" .folded)
    if command -v inferno-flamegraph >/dev/null; then
        inferno-flamegraph --title "allocated bytes: $name" --countname bytes --colors mem <"$f" >"$out/heap-$name.svg"
    fi
    python3 tools/flame.py --folded "$f" --unit bytes
done
