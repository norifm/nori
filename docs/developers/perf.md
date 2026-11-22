# Performance

This page shows how to measure what nori costs and how to make it cheaper. It covers the engine on the
host, CPU and heap profiles, the perf build on a phone, and the adb tools. For the desktop app's window,
memory and GPU, see [Desktop profiling](desktop-performance.md). For the iPod app's battery, see
[iPod internals](ipod-internals.md). The performance rules in AGENTS.md apply to every change.

## The method

1. Pick one resource: CPU, memory, disk, network or wakeups. On a phone, wakeups and radio time cost
   battery even when the CPU time is low.
2. Estimate before you measure. 44.1 kHz stereo is about 350 KB/s of f32. So a 10 s burst is 3.5 MB,
   and it should take milliseconds.
3. Measure with a bench or a profile. Reading the code is not a measurement.
4. Take the widest bar in the profile. Make it smaller, or prove it is the floor. Then take the next one.
5. Change the design, not the symptom. Do not add a cache to hide a slow path, or a flag that steers
   other code.
6. Keep the output equal. For a pure speedup, compare the output bit for bit before and after. Where
   the output may change, say why and check it against a reference.
7. Measure again with the same bench, and keep the bench.

Run benches alone. Numbers from a machine that runs something else, such as a build or another bench,
are noise.

## Measure the engine on the host

`tools/perf-host.sh` compares the engine of this checkout with another revision, on this machine:

```sh
tools/perf-host.sh                # against the latest release tag, two runs each
tools/perf-host.sh v0.9.4 3       # against that revision, three runs each
```

It plays minutes of music through the engine on the test clock, with no phone or emulator. It checks out
the other revision in a scratch worktree, and gives it the bench's files if it is older than them. The
two sides run one after the other, never at once.

The bench is the ignored test `crates/engine/tests/perf_bench.rs`. Each case prints one line per run:

```text
perf: plain      music  4.00 min | wakes/min     ... | cpu ms/min ... | allocs/min ... | alloc KB/min ... | peak MB ... | sound <digest>
```

Read the figures per minute of music:

- **wakes/min** is how often the engine thread woke. While music plays, the engine should sleep for
  seconds between bursts.
- **cpu ms/min** is the CPU time of every thread except the test thread.
- **allocs/min** and **alloc KB/min** count the allocations. The audio path must not allocate.
- **peak MB** is the heap's peak.
- **sound** is a digest of what the test card heard.

Under `== sound`, the script says for each case whether the two revisions sound "the same", or the
output "differs". A revision whose report has no digest is "not compared". Only a comparison made on
one machine means anything.

### Run the bench directly

```sh
cargo test --release -p nori-engine --test engine perf_report -- --ignored --nocapture --test-threads=1
```

The cases are `plain`, `equalizer`, `eq10`, `compressor`, `mp3`, `flac`, `opus`, `vorbis`, `aac`,
`alac`, `speed`, `silence`, `gain`, `gaineq`, `change`, `crossfade`, `automix` and `automix-dj`. The
codec cases need FFmpeg. These variables change a run:

| Variable | Effect |
| --- | --- |
| `NORI_PERF=plain,automix` | Runs only these cases |
| `NORI_PERF_MINUTES=10` | Plays each case this long, for a profiler |
| `NORI_WAKES=1` | Prints each sleep's length and the runs of short sleeps |
| `NORI_HEAP_STACKS=<dir>` | Writes where each case allocated to `<dir>/<case>.folded` |

A run of short sleeps under `NORI_WAKES=1` is a timer that fires on a guess that does not come true.

## Profile the CPU and the heap

`tools/flame.py` records any command with samply. It needs samply, and inferno for the SVG
(`cargo install inferno`). Build what it runs with `--profile profiling`, which is release code with line
tables.

```sh
python3 tools/flame.py -o build/flame/x -- <command>                # a command that ends
python3 tools/flame.py -o build/flame/x --for 60 -- <command>       # an app that runs until quit
python3 tools/flame.py --profile build/flame/x/profile.json.gz --since 15 --until 55 --thread <name>
python3 tools/flame.py --profile build/flame/x/profile.json.gz --per-second
python3 tools/flame.py --folded stacks.folded --unit bytes          # the table from heap stacks
```

It writes `flame.svg`, `stacks.folded` and `profile.json.gz` to the output folder. It prints the
functions that take the most CPU, by self time and by total time. `samply load profile.json.gz` opens
the profile in the Firefox profiler.

Inlined helpers show as their caller's self time. To split them, mark them `#[inline(never)]` for one
run, then remove the marks.

`tools/flame-engine.sh` runs flame.py over the engine's bench cases:

```sh
tools/flame-engine.sh automix,mp3 [minutes] [out]       # CPU of the engine's threads
tools/flame-engine.sh --heap automix [minutes] [out]    # where the engine allocates, by bytes
```

It also takes the case `stage`, a plugin's sound stage through wasmtime. The output goes to
`build/flame/<case>` by default.

### Profile the shipping build

`release` is not what ships. `dist` adds fat LTO, builds most code for size and the hot crates at
opt-level 3. Check a change that depends on inlining or the optimization level with `--profile dist`.
Release builds have no LTO, so a non-generic `pub fn` that a hot loop calls in another crate needs
`#[inline]`.

## Benches

Each bench is an ignored test. Run it with `--release`, `--ignored` and `--nocapture`:

| What it measures | Command |
| --- | --- |
| A plugin's sound stage | `cargo test --release -p nori-engine --test engine perf_stage -- --ignored --nocapture` |
| Library index, sync, search, prune | `cargo test --release -p nori-db --test perf -- --ignored --nocapture` |
| Cover decoding and colors | `cargo test --release -p nori-covers --test perf -- --ignored --nocapture` |
| AutoMix analysis of a minute | `cargo test --release -p nori-player --test analysis_perf -- --ignored --nocapture` |
| Time stretch, the port against the C++ | `cargo test --release -p nori-stretch --test perf -- --ignored --nocapture` |
| A plugin decider, one call and a shuffle of 1000 songs | `cargo test --release -p nori-plugins --test deciders decider_calls_cost -- --ignored --nocapture` |
| A plugin's audio stream | `cargo test --release -p nori-plugins --lib sink::tests::a_minute_sent -- --ignored --nocapture` |

These tests print a digest of their output, to compare before and after a change:

| Output | Command |
| --- | --- |
| The time stretch | `cargo test --release -p nori-stretch --test compare digest -- --ignored --nocapture` |
| FLAC, MP3, ALAC, Vorbis and AAC decoding | `cargo test --release -p nori-engine --test engine decoders::digest -- --ignored --nocapture` |
| Opus decoding, by mode, rate and frame length | `cargo test --release -p nori-engine --test engine decoders::opus_digest -- --ignored --nocapture` |

`decoders::digest` prints a digest per shape of song. `decoders::digest_alac` prints one codec's.

## Measure on a phone with the perf build

The perf build measures what the app costs on a real phone in real use, with no computer attached.
[Building](building.md) shows how to build and install it. Its recorder starts with the app, and its page
is **Settings › Performance**.

### Run a fair battery test

1. Press **Start fresh** before a series of runs.
2. Unplug the phone for the whole run.
3. Keep the same playlist, volume, output, network and download state for every run.
4. Turn the screen off, and leave the phone untouched for 1 to 2 hours per stretch.
5. Make sure the normal nori app is not playing at the same time.
6. After each run, press **Share report**.
7. Do three runs of each variant. Compare the mAh/h of "Screen off, playing".

### How the recorder splits time

The recorder cuts the app's life into stretches. Each stretch is spent in one of these states:

- Screen off, playing. This is the figure that matters most.
- Screen off, paused. This should be close to nothing.
- Screen on, playing, player open.
- Screen on, playing, other page.
- Screen on, playing, another app.
- Screen on, paused.
- Charging. The recorder leaves it out of every battery figure.

- A stretch also ends when a setting that changes the cost changes. These settings are the equalizer,
  AutoMix, crossfade, offload, hi-res and bit-perfect.
- Each stretch lists those settings, and whether offload was wanted.
- The recorder drops stretches under 3 seconds, and keeps the rest for 14 days.
- The stretch under way shows as "Now". It is lost if the process dies.
- **Start fresh** forgets every stretch.

### What each figure means

| Figure | Source |
| --- | --- |
| time | Elapsed realtime, deep sleep included |
| CPU % | The process's utime and stime (`/proc/self/stat`), in % of one core |
| wakeups/s | Voluntary context switches of the app's threads (`/proc/self/task/*/status`) |
| KB/min, GCs | Java heap allocated (`art.gc.bytes-allocated`) and collections run |
| PSS MB | Proportional memory at the end (`Debug.MemoryInfo`) |
| mAh, mAh/h | The whole phone's charge counter (`BATTERY_PROPERTY_CHARGE_COUNTER`) |
| gauge mA | The fuel gauge's current at both ends, as a cross-check only |
| °C | Battery temperature at both ends |
| frames, janky | Frames drawn on screen, those that missed their deadline, and the slowest |
| network | Bytes received and sent (`TrafficStats`) |

The battery figures are the whole phone's. CPU, wakeups, allocations and PSS are the app's alone. A
phone without a charge counter shows the drop of its battery level in %. "By state" sums the stretches
of each state, with the share of time in offload.

Under each stretch, the page shows three more parts:

- **Threads by wakeups** lists the six threads that woke most, with their wakeups/s and CPU. The Rust
  threads carry their own names: `nori-engine`, `nori-track`, `nori-load` and others.
- **Output** shows the last AudioTrack as the platform describes it, against what nori asked for. That
  is the rate, channels, encoding, buffer, performance mode, offload, route and underruns.
- **Timeline** lists up to 150 timestamped events. They include songs, changed settings, the service
  starting or ending, outputs opened or released, offload, underruns, errors and buffer switches.

Offloaded stretches also show the engine wake lock's held time, `nori-engine`'s wakeups, and the audio
chip's data requests per second.

The core (`crates/perf`) files each stretch under its state, does the arithmetic and writes the report.
Android only reads the counters (`android/app/src/perf`).

### Invariant breaks

The report starts with invariant breaks (`crates/perf/src/invariants.rs`). The recorder checks them on
events that happen anyway, never on a timer:

- An output that holds music stops advancing ("stalled", "offload-starved"), or nothing feeds it
  ("starved").
- With no output open, the position stands still while nothing downloads ("silent").
- One skip press moves more or less than one song.
- The shown song or lyrics are not the song you hear, or the seek bar strays from the engine's position.
- With AutoMix on, a queued song has no duration.
- A changed setting has not reached the engine a second later.
- A Rust thread panicked.

With each break, the recorder saves the app's own recent log: the core's and Kotlin's lines under the
`nori` tag, from the last ten minutes. It keeps the logs of the latest three breaks.

### Report, benchmarks and self test

- **Share report** sends plain text through Android's share sheet. It holds the device, the Android
  version, the build, the table by state, the latest stretches, and the benchmark results. Then come the
  app's own log, the crash buffer, the last uncaught exception and the last 400 logcat lines.
- The report is at most 60 000 characters. Its columns line up in a monospaced font.
- **Run call benchmark** times calls into the core against the same work in Kotlin.
- **Run cover benchmark** times the core's cover decoding. Browse some albums first, so covers are cached.
- Both run the same code as the debug build's `bench` and `coverbench` commands
  (`android/app/src/bench`). Their cost lands in the stretch under way.
- **Run self test** plays songs from the library, downloads first. It checks playback, the controls,
  offload, settings applied live, lyrics and covers. It takes about six minutes, and then puts back the
  settings, the queue and the position.

### What the recorder costs

Nothing ticks. The recorder reads the counters only when something changes:

- The screen, the power or playback changes.
- The app comes to the front or leaves it, or the player opens or closes.
- A setting that ends a stretch changes.
- The Performance page opens.

With the screen off and music playing, that is only a song change, which wakes the app anyway. So the
recorder adds no wakeups of its own. It collects frame timings only while the app is on screen. It reads
logcat only when you share the report or show the page's log.

## Measure Android over adb

These tools read a connected device. They all honor `ANDROID_SERIAL`.

| Tool | Measures |
| --- | --- |
| `tools/cost.sh LABEL [SECONDS]` | CPU, wakeups, frames/s, GCs/min and the busiest threads, from a debug build |
| `tools/meminfo.sh LABEL [package]` | One line of `dumpsys meminfo`, with `libnorimusic.so`'s PSS and the thread count |
| `tools/wakers.py [seconds]` | Which threads wake each of the app's threads, from a scheduler trace |
| `tools/net-check.py [seconds] [play]` | Bytes received against each song the player opened |
| `tools/open-bench.sh <package> "<label>" [times] [flings]` | Frames and janky frames of opening a page and flinging it |
| `tools/scroll.sh <package> [flings]` | Scroll jank of the list on screen |
| `tools/perf-suite.sh <serial> <server-url> [user] [password]` | The whole suite on one device, into `build/perf-results.md` |

- Before `tools/wakers.py`, turn the screen off with `adb shell input keyevent 223` to see what runs
  with it off.
- `tools/net-check.py` shows a song fetched twice. Only this app should use the network meanwhile.
- `tools/open-bench.sh` needs a perf or release build. A debug build composes several times slower and
  is janky anyway. Start it on a list that shows the label first, such as the seeded "Nori Bench 1000"
  playlist in **Library › Playlists**.
- `tools/perf-suite.sh` builds and installs a release build first, and needs the phone past its lock screen. It
  expects a "Bench / Long Play" album, which `tools/dev-server.sh` does not seed.

### Profile the perf build on a phone

The perf build is profileable from adb. For its CPU, run the NDK's simpleperf:

```sh
$ANDROID_NDK_HOME/simpleperf/app_profiler.py -p dev.nori.music.perf \
  -r "-e cpu-clock -f 4000 --call-graph dwarf --duration 30" -lib target/aarch64-linux-android/dist
```

For its heap, start Perfetto's heapprofd on `dev.nori.music.perf` before you launch the app. It counts
only what is allocated after it attaches.

## What to look for

- **CPU.** Look for a loop with a branch or a division per sample. Look for work done per call that
  could be done once per buffer or per song. Look for a dependency's generic path where nori uses one case.
- **Memory.** Read the peak and the allocations per minute in the engine's perf report. The heap flame
  graph shows where they come from.
- **Disk.** Look for rows written when nothing changed, such as a sync that rewrites the library. Look
  for indexes that SQLite keeps through a Rust callback, and reads that could use the memory map.
- **Network.** Look for requests the user did not ask for, and requests that could go together while
  the radio is up. Look for URLs that change, so that caches miss.
- **Frames.** An interface draws only what changed, once. Count the frames per second while music plays
  and nothing on screen moves.
- **Wakeups.** The engine sleeps for seconds between bursts while music plays. A run of short sleeps
  under `NORI_WAKES=1` is a timer that fires too early.
- **Plugins.** A host call per buffer, not per sample. A plugin gets events only while it listens.
- **Dependencies.** Look for two versions of one crate (`cargo tree -d`), and for a crate used for one
  function. A small Rust port can replace a C or C++ library.

To change a dependency's hot loop, copy it under `vendor/`. Its `NORI.md` says what changed. Compare a
digest of its output before and after.

## Rules for fast code

- Lay data out for how it is read. Prefer a flat `Vec` indexed by position to a map keyed by strings.
  Use a struct of arrays when a loop reads one field of many items.
- Use a bitset (`Vec<u64>`), not a `Vec<bool>`. Use a slice or an inline array, not a `Vec` built per
  call.
- Allocate nothing per buffer or per sample.
- Branch on a mode once per buffer, not per sample, so the loop can vectorize.
- Do not use `%` or `/` per sample, because they block vectorization. Walk the ring in runs instead.
- Keep stores that are not API responses in SQLite columns, not JSON.
- For a sort that SQLite does itself, use a key column it compares bytewise (`nori_db::natural_key`).
  A collation that calls back into Rust is slow.
- Put a migration from an older store in `crates/db/src/upgrade.rs`, so it can be deleted later.
- Do work once. One decode feeds every consumer that can share it, and each song gets one analysis.

## Further reading

- Brendan Gregg: [the USE method](https://www.brendangregg.com/usemethod.html) and
  [flame graphs](https://www.brendangregg.com/flamegraphs.html)
- [The Rust Performance Book](https://nnethercote.github.io/perf-book/)
- Ross Bencina:
  [real-time audio programming](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing)
- SQLite: [WAL](https://www.sqlite.org/wal.html), [FTS5](https://www.sqlite.org/fts5.html) and
  [memory-mapped I/O](https://www.sqlite.org/mmap.html)
- Android: [power](https://developer.android.com/topic/performance/power),
  [Baseline Profiles](https://developer.android.com/topic/performance/baselineprofiles/overview),
  [Perfetto](https://perfetto.dev/docs/) and
  [simpleperf](https://developer.android.com/ndk/guides/simpleperf)
