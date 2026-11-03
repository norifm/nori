# Performance

How nori is measured and made faster. The rules in AGENTS.md (Performance) still apply; this page is
the method and the tools.

## The loop

1. Pick a resource: CPU, memory, disk, network or wakeups. On a phone, wakeups and radio time cost
   battery even when CPU time is low.
2. Measure it with a bench or a profile, never by reading code alone. Estimate first: 44.1 kHz stereo
   is about 350 KB/s of f32, so a 10 s burst is 3.5 MB and should take milliseconds.
3. Take the widest bar in the profile. Make it smaller, or prove it is the floor; then take the next.
4. Change the design, not the symptom: no caches added to hide a slow path, no flags steering other
   code. A faster design beats a cached slow one.
5. Keep results equal. For a pure speedup, compare the output bit for bit before and after (dump it
   from a test); where the output may change, say why and check it against a reference.
6. Measure again with the same bench, and keep the bench.

## Rules for the data

- Lay data out for how it is read: a flat `Vec` indexed by position over a map keyed by strings;
  struct of arrays when a loop reads one field of many items.
- A bitset (`Vec<u64>`), not a `Vec<bool>`. A slice or an inline array, not a `Vec` built per call.
- Nothing allocates per buffer or per sample. Branch on a mode once per buffer, not per sample, so the
  loop can vectorize. `%` and `/` per sample block vectorization; walk the ring in runs instead.
- A non-generic `pub fn` in a hot loop in another crate needs `#[inline]` (release builds have no LTO).
- Stores that are not API responses are SQLite columns, not JSON. A sort that SQLite does itself uses
  a key column it compares bytewise (`nori_db::natural_key`), not a collation calling back into Rust.
  A migration from an older store lives in `crates/db/src/upgrade.rs`, so it can be deleted later.
- State lives in the struct that owns it, not in a static.
- Work happens once: one decode feeds every consumer that can share it, and one analysis per song.

## Tools

| what | command |
|---|---|
| CPU flame graph of any command | `python3 tools/flame.py -o build/flame/x -- <command>` |
| An app that runs until quit, for a while | `python3 tools/flame.py -o build/flame/x --for 60 -- <command>` |
| A window of an earlier profile; CPU each second by thread | `python3 tools/flame.py --profile build/flame/x/profile.json.gz --since 15 --until 55 [--thread name]`; `--per-second` |
| The desktop client through every page, idle or playing | `tools/desktop-tour.py SOCKET [rounds] [pause] [album id]`, [Desktop profiling](desktop-performance.md) |
| Engine CPU, per case | `tools/flame-engine.sh automix,mp3 [minutes] [out]` |
| Engine heap, by call site | `tools/flame-engine.sh --heap automix [minutes] [out]` |
| Engine wakes, CPU, allocations per minute | `cargo test --release -p nori-engine --test engine perf_report -- --ignored --nocapture --test-threads=1` |
| What keeps the engine awake | the same with `NORI_WAKES=1`: sleep lengths and runs of short sleeps |
| This checkout against a release | `tools/perf-host.sh [rev]` |
| A plugin's sound stage | `cargo test --release -p nori-engine --test engine perf_stage -- --ignored --nocapture` |
| Library index, sync, search, prune | `cargo test --release -p nori-db --test perf -- --ignored --nocapture` |
| Cover decode and colors | `cargo test --release -p nori-covers --test perf -- --ignored --nocapture` |
| AutoMix analysis of a minute | `cargo test --release -p nori-player --test analysis_perf -- --ignored --nocapture` |
| Time stretch, the port against the C++ | `cargo test --release -p nori-stretch --test perf -- --ignored --nocapture` |
| A plugin's decider: one call, a shuffle of 1000 songs | `cargo test --release -p nori-plugins --test deciders decider_calls_cost -- --ignored --nocapture` |
| A plugin's audio stream | `cargo test --release -p nori-plugins --lib sink::tests::a_minute_sent -- --ignored --nocapture` |
| Android, one device | `tools/perf-suite.sh <serial> <server-url>`; frames: `tools/open-bench.sh` |

`tools/flame.py` records with samply, writes `flame.svg`, the folded stacks and the profile, and prints
the top functions by self and by total time; `--folded` reads stacks already made (`--unit bytes` for
heap stacks). Build what it runs with `--profile profiling` (release code with line tables). Inlined
helpers show as their caller's self time: mark them `#[inline(never)]` for one run to split it, then
remove the marks.

Numbers from a machine running something else (a build, another bench) are noise: run benches alone.
`release` is not what ships: `dist` adds fat LTO and builds the cold code for size, with the hot crates
at O3. Check a change that depends on inlining or optimization level with `--profile dist`.

## What to check

- CPU: the widest bar, then the next. A loop with a branch or a division per sample; work done again
  per call that could be done once per buffer or per song; a dependency's generic path where nori uses
  one case (the stretch's frequency map, a bitwise CRC).
- Memory: peak and allocations per minute in the perf report; the heap flame graph for where.
- Disk: rows written when nothing changed (a sync that rewrites the library), indexes SQLite maintains
  through a Rust callback, reads that could come through the memory map.
- Network: requests the user did not ask for, requests that could go together while the radio is up,
  URLs that change so caches miss.
- Frames: an interface draws only what changed, once. Count frames per second while music plays
  with nothing on screen moving; a Slint animation of 0 ms still reads the animation clock and draws
  again on the next tick, so an animation that is sometimes instant is turned off with `enabled`.
- Wakeups: the engine sleeps for seconds between bursts while music plays; a run of short sleeps
  (`NORI_WAKES=1`) is a timer firing on a guess that does not come true.
- Plugins: a host call per buffer, not per sample; a plugin's events only while it listens.
- Dependencies: two versions of one crate (`cargo tree -d`), a C or C++ library a small Rust port can
  replace, a crate used for one function.

## Reading

- Brendan Gregg, the USE method: https://www.brendangregg.com/usemethod.html, and flame graphs:
  https://www.brendangregg.com/flamegraphs.html
- The Rust Performance Book: https://nnethercote.github.io/perf-book/
- Ross Bencina, real-time audio programming: http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing
- SQLite: WAL https://www.sqlite.org/wal.html, FTS5 https://www.sqlite.org/fts5.html, memory-mapped
  I/O https://www.sqlite.org/mmap.html
- Android: power https://developer.android.com/topic/performance/power, Baseline Profiles
  https://developer.android.com/topic/performance/baselineprofiles/overview, Perfetto
  https://perfetto.dev/docs/, simpleperf https://developer.android.com/ndk/guides/simpleperf
