# Building

Applies to the nori repository at 0.7 (master is 0.7.1-beta). Commands run from the repository's root.
The everyday commands are in [AGENTS.md](../../AGENTS.md#commands); this page has the setup and the
details behind them.

## Requirements

| For | Needs |
| --- | --- |
| Every build | Rust stable through rustup; sccache (`cargo install --locked sccache`), since `.cargo/config.toml` sends every compile through it |
| Rust tests | FFmpeg 8 or newer on the `PATH`: the engine tests encode their fixtures with it |
| Android | Android SDK and NDK (`ANDROID_NDK_HOME`, or the newest under `$ANDROID_HOME/ndk`), a JDK 17 or newer, `rustup target add aarch64-linux-android x86_64-linux-android`, `cargo install cargo-ndk` |
| Desktop and terminal on macOS | Xcode's command line tools |
| Desktop and terminal on Linux | `build-essential pkg-config clang libasound2-dev libfontconfig1-dev libfreetype6-dev libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev` (Ubuntu names, as `tools/host-release/Dockerfile` installs them; WebKitGTK is for the sign-in window) |
| The iPod app, Linux release packages | Docker |
| Optional | `brew install cargo-nextest bazelisk` for `tools/check.sh rust` and the Bazel backend |

The web client has its own list, under [The web client](#the-web-client).

Never run Cargo with more than four jobs: the build machine runs out of memory. `.cargo/config.toml`
sets `jobs = 4` and `gradle.properties` sets four Gradle workers.

## Faster development checks

### Profiles

| Cargo profile | Used by | What it is |
| --- | --- | --- |
| `dev` | `cargo test`, `cargo build` | Line tables only for the workspace, no debug info for dependencies. Test builds compile nori-player, nori-engine, nori-android, nori-ios and every dependency at opt-level 2, with debug assertions on. nori-stretch is always at opt-level 3 |
| `release` | `cargo run --release`, debug APKs | Optimized and incremental: an edit rebuilds in seconds |
| `dist` | release, perf and preview APKs, desktop packages, the iPod app | Fat LTO, one codegen unit, stripped, `panic = "unwind"`. Takes minutes to link |
| `web` | `web/build.sh` | `dist` with `opt-level = "s"` and `panic = "abort"` |

Set `CARGO_PROFILE_DEV_DEBUG=true` for a session that needs a debugger's variables. Keep debug APKs
out of performance comparisons; use [the perf build](#the-perf-build).

Gradle properties for the Android build:

| Property | Default | Effect |
| --- | --- | --- |
| `-PrustTargets=arm64-v8a` | debug: the host's emulator ABI; release, perf, preview: `arm64-v8a` | The ABIs the Rust core is built for |
| `-PrustProfile=dist` | debug: `release`; release, perf, preview: `dist` | The Cargo profile of the native library |
| `-PrustFeatures=` | `neural-beats` | Cargo features of the core. Empty leaves out tract and the "Better beat detection" setting |
| `-PpretendVersion=0.3.9` | none | Builds the app as an older version, for testing the updater |

The native library and the generated bindings are cacheable Gradle outputs, keyed by sources, features,
profile, compiler, NDK and build environment. The binding inputs are the crates in `bindingCrates` in
`core/build.gradle.kts`; extend that list when adding an FFI crate.

### The shared queue

Heavy work goes through one machine-wide queue, so worktrees and agents take turns:

| Command | Runs |
| --- | --- |
| `tools/check.sh rust --workspace` | `cargo nextest run` with four build jobs and four test threads; `-p <crate>` for one crate |
| `tools/check.sh rust-doc --workspace` | the doctests, which nextest skips |
| `tools/check.sh android [GRADLE OPTIONS]` | `./gradlew :app:assembleDebug` |
| `tools/check.sh bazel <command> [...]` | a Bazel command |
| `python3 tools/with-resource.py build <command>` | any other command, in the same queue |

`python3 tools/with-resource.py build cargo test -j4 --workspace` is the full pre-commit Rust check,
doctests included. Direct commands keep the four-job limit but skip the queue.

### Worktrees and disk

Each worktree has its own `target/` and Gradle `build/`. A shared Cargo target directory is not used:
the e2e peer `target/debug/nori-cli` and the Android library would be overwritten by whichever worktree
built last, and Cargo's lock would serialize every build. Worktrees share these caches instead:

- sccache: compiled dependencies for every worktree, up to 30 GB (`SCCACHE_CACHE_SIZE` in
  `.cargo/config.toml`). It cannot cache incremental workspace crates or links.
- Gradle's build cache (`~/.gradle/caches`): the native library, bindings and Kotlin classes when their
  inputs match. A changed `Cargo.toml` or `Cargo.lock` misses.
- Bazel's disk cache (`~/.cache/nori/bazel`).

A removed worktree takes its directories with it. The main checkout's `target/` only grows, since every
profile, feature set and target keeps its own artifacts. `cargo clean`, or `cargo sweep --time 14` from
`cargo install cargo-sweep`, gives the space back.

### Devices

`tools/emulator.sh <avd> <port>` starts an existing arm64 AVD headless, with host graphics and four
CPUs. Use a distinct even port per emulator. Create the AVDs in Android Studio first.

```sh
tools/emulator.sh a16 5554
tools/emulator.sh a16b 5556
export NORI_E2E_SERVER=local
export NORI_E2E_DEVICES=emulator-5554,emulator-5556
tools/device-check.sh --install app/build/outputs/apk/debug/app-debug.apk smoke
tools/device-check.sh audio --only tuning,restart
```

- `tools/device-check.sh [--install APK] smoke|audio|feature [--only SECTIONS]` leases the first free
  device in `NORI_E2E_DEVICES` for the install, the checks and the cleanup. Jobs wait when every device
  is taken. `smoke.sh`, `audio-e2e.sh` and `feature-e2e.sh` run directly lease their `ANDROID_SERIAL`
  (default `emulator-5554`).
- Don't drive a leased device with `tools/app.sh` or adb from elsewhere.
- Each emulator gets its own Navidrome account (`nori-e2e-<serial>`), proxy port, terminal peer and
  logs. `emulator-5554` uses proxy port 4534 and `emulator-5556` uses 4536; `NORI_E2E_PROXY_PORT`
  sets it for other serials. The music library is shared.
- A failed check saves the accessibility tree and a screenshot under `build/e2e/<serial>/failures/`.
- `tools/ui-dump.sh` reads the visible accessibility tree through `tools/UiDump.java`, built on demand,
  without waiting for an animating player to go idle.
- Jam checks need an octo-fiesta relay on `localhost:5274` whose `/nori/jam` page offers "Open in nori".
  `tools/jam-server.sh` starts one from the octo-fiesta checkout beside this one (`NORI_OCTO_ROOT` names
  another), and says so when the relay there is outdated.

The local server and what each check covers are in [Testing](testing.md).

### Native Rust with Bazel

An optional backend for cached native builds and tests across worktrees. Cargo manifests and
`Cargo.lock` stay the source of truth; APKs, the iPod app, release packages, doctests and device checks
keep their own tools.

```sh
python3 tools/rust.py test --workspace
python3 tools/rust.py test -p nori-player -- pipeline     # arguments after -- go to the test binaries
python3 tools/rust.py build -p nori-desktop
python3 tools/rust.py run -p nori-cli
```

`tools/rust.py` regenerates the BUILD files from Cargo metadata and runs Bazel through the shared queue.
After changing a manifest:

```sh
python3 tools/bazel/generate.py                            # regenerate; never edit generated BUILD files
python3 tools/bazel/generate.py --check
python3 -m unittest discover -s tools/bazel -p 'test_*.py'
tools/check.sh bazel test //crates/...
```

- Tests build with the workspace's default and development features; `-p` narrows the targets, not the
  features. Desktop and terminal builds use their shipping features. Other feature selections and
  cross-compilation are not supported.
- Bazel builds are optimized (`--compilation_mode=opt`) and meant for development; releases use Cargo.
- Native tests need the same platform libraries and FFmpeg as Cargo tests.
- A passing test is cached and does not run again. Add `--nocache_test_results` to run it anyway.
- Pass `--remote_cache` to a direct Bazel command where a remote cache exists. Remote execution is not
  set up.

### Timing a development command

```sh
python3 tools/measure-dev.py --name rust-edit tools/check.sh android
python3 tools/measure-dev.py --name smoke env NORI_E2E_SERVER=local tools/device-check.sh smoke
```

On macOS, it writes wall time, the process tree's CPU, sampled compiler, JVM and emulator CPU and memory,
and whole-machine activity to `build/development-timings/<name>/`. Emulator and JVM figures include
every matching process on the host. 100% CPU is one core.

## The perf build

A release build with a recorder in it. It measures what the app costs on a real phone in real use
(battery, CPU, wakeups, allocations, memory, frames), with no computer attached. It is minified and not
debuggable, like a release, and installs beside the normal app as "nori dev" (`dev.nori.music.perf`)
with its own sign-in, settings and downloads.

```sh
./gradlew :app:assemblePerf -PrustTargets=arm64-v8a    # a phone
./gradlew :app:assemblePerf -PrustTargets=x86_64       # an x86_64 emulator
adb install -r app/build/outputs/apk/perf/app-perf.apk
```

The APK is signed with the Android debug key. It can also be sent to the phone and opened there. The
recorder starts with the app. Its page is Settings > Performance.

`./gradlew :app:assemblePreview -PrustTargets=arm64-v8a` builds "nori preview"
(`dev.nori.music.preview`), a release build without the recorder, for trying a change beside the real
app.

### Stretches

The recorder cuts the app's life into stretches, each spent in one state:

| State | Note |
| --- | --- |
| Screen off, playing | The figure that matters most |
| Screen off, paused | Should be close to nothing |
| Screen on, playing, player open | |
| Screen on, playing, other page | |
| Screen on, playing, another app | |
| Screen on, paused | |
| Charging | Left out of every battery figure |

A stretch also ends when a setting that changes the cost changes: equalizer, AutoMix, crossfade,
offload, hi-res or bit-perfect. Each stretch lists those settings and whether offload was wanted.
Stretches under 3 seconds are dropped. Stretches are kept for 14 days (`perf_stretches`); the stretch
under way shows as "Now" and is lost if the process dies. "Start fresh" forgets everything.

Each stretch records:

| Figure | Source |
| --- | --- |
| time | elapsed realtime, deep sleep included |
| CPU % | the process's utime + stime (`/proc/self/stat`), in % of one core |
| wakeups/s | voluntary context switches of the app's threads (`/proc/self/task/*/status`) |
| KB/min, GCs | Java heap allocated (`art.gc.bytes-allocated`) and collections run |
| PSS MB | proportional memory at the end (`Debug.MemoryInfo`) |
| mAh, mAh/h | the whole phone's charge counter (`BATTERY_PROPERTY_CHARGE_COUNTER`); phones without it show the level's drop in % |
| gauge mA | the fuel gauge's current at both ends, a cross-check only |
| °C | battery temperature at both ends |
| frames, janky | frames drawn while on screen and those that missed their deadline, with the slowest |
| network | bytes received and sent (`TrafficStats`) |

Under each stretch:

- **Threads by wakeups**: the six threads that woke most, with wakeups/s and CPU. The Rust threads carry
  their own names (`nori-engine`, `nori-track`, `nori-load`, ...).
- **Output**: the last AudioTrack as the platform describes it against what was asked: rate, channels,
  encoding, buffer, performance mode, offload, route, underruns. Offloaded stretches also show the
  engine wake lock's held time, `nori-engine`'s wakeups and the chip's data requests per second.
- **Timeline**: up to 150 timestamped events: songs (format and source), settings changed, the player
  service starting or ending, outputs opened or released, offload entered or left, underruns, errors and
  the shallow buffer switching.

"By state" sums the stretches of each state, with the share of time really offloaded. Battery figures
are the whole phone's; CPU, wakeups, allocations and PSS are the app's.

The core (crates/perf) files each stretch under its state, does the arithmetic and writes every word
on the page and in the report; Android only reads the counters (`app/src/perf`).

### Invariant breaks

The report starts with invariant breaks (crates/perf/src/invariants.rs), checked on events that happen
anyway, never on a timer:

- an output holding music stops advancing ("stalled", "offload-starved") or goes unfed ("starved");
- with no output open, the place stands still with nothing downloading ("silent");
- one skip press moves more or less than one song;
- the shown song or lyrics are not the heard song's, or the seek bar strays from the engine's place;
- with AutoMix on, a queued song has no duration;
- a changed setting has not reached the engine a second later;
- a Rust thread panicked.

With each break, the app's own recent log (the core's and Kotlin's lines under the `nori` tag, the last
ten minutes, 500 to 5000 lines) is saved, for the latest three breaks.

### What it costs

Nothing ticks. The counters are read only when something changes: the screen, the power, playback, the
app coming to the front or leaving, the player opening or closing, a setting above, or the Performance
page opening. With the screen off and music playing, that is only a song change, which wakes the app
anyway, so the recorder adds no wakeups of its own. Frame timings are collected only while the app is on
screen. The log is read from logcat only when the report is shared or the page's log is shown.

### A fair battery test

1. Unplugged for the whole run.
2. The same playlist, volume, output, network and download state for every run.
3. Screen off, 1 to 2 hours per stretch, the phone untouched.
4. Three runs of each variant; compare the mAh/h of "Screen off, playing".
5. The normal nori app not playing at the same time.

Press "Start fresh" before a series and share the report after each run.

### Report, benchmarks and self test

- **Share report**: plain text through Android's share sheet: device, Android version, build, the
  table by state, the latest stretches with their output and timeline, benchmark results, then the
  app's own log, the crash buffer, the last uncaught exception and the process's last 400 logcat lines
  (60 000 characters at most). Columns are aligned for a monospaced font.
- **Run call benchmark** and **Run cover benchmark**: the same code as the debug build's `bench` and
  `coverbench` test commands (`app/src/bench`). They time crossings into the core against the same
  work in Kotlin, and the core's cover decoding ([Writing a client](writing-a-client.md)). Browse some
  albums first so covers are cached. Their cost lands in the stretch under way.
- **Run self test**: plays songs from the library (downloads first) and checks playback, the controls,
  offload, settings applied live, lyrics and covers. It takes about six minutes and puts settings,
  queue and place back afterwards.

### Measuring over adb

| Tool | Measures |
| --- | --- |
| `tools/cost.sh LABEL [SECONDS]` | the app as it is on screen: CPU, wakeups, frames/s, GCs/min and the six busiest threads (debug build) |
| `tools/meminfo.sh LABEL [package]` | one line of `dumpsys meminfo`: total PSS, Java and native heap, code, stack, graphics, `libnorimusic.so`'s PSS, threads |
| `tools/open-bench.sh <package> "<label>" [times] [flings]` | frames and janky frames of opening a page and flinging it, such as the seeded "Nori Bench 1000" playlist from Library > Playlists. Use a perf or release build |
| `tools/scroll.sh <package> [flings]` | scroll jank of the list on screen |

All honor `ANDROID_SERIAL`. For the engine's cost on the host, see
[the host perf report](testing.md#the-host-perf-report).

## The web client

Applies to master; the web client is not in a release yet. It is `crates/web` (nori-web, built for
`wasm32-unknown-unknown` with wasm-bindgen; natively an empty library, so workspace tests and clippy
skip it) and `web/` (Svelte 5, TypeScript, Vite).

### What it needs

- `rustup target add wasm32-unknown-unknown`
- wasm-bindgen-cli at the version in `Cargo.lock`: `cargo install wasm-bindgen-cli --version 0.2.128`
- `wasm-opt` (binaryen), `brotli`, node 20 or newer with npm (versions pinned in `web/package-lock.json`)
- a clang that targets wasm32, for SQLite's C: Homebrew LLVM (`brew install llvm`) or the Android
  NDK's, which `web/build.sh` finds itself. Apple's clang has no wasm32 backend. Or set
  `CC_wasm32_unknown_unknown`.

### Building and serving

```sh
web/build.sh                                # everything into web/dist, then a size table
web/build.sh --shell                        # only the Svelte shell, over the wasm modules already built
tools/dev-server.sh                         # the local Navidrome, admin/admin
python3 web/serve.py                        # http://localhost:8080, /rest/ passed to localhost:4533
python3 web/serve.py --log --kbps 400       # print each API request; answers at 400 kbit/s at most
```

`web/serve.py` also takes `--port`, `--server URL` and `--files-kbps` (throttles the client's own files
too). It is for development only.

`web/build.sh` builds three modules, each through `wasm-opt -Oz`:

| Module | Holds |
| --- | --- |
| `core.wasm` | sign-in and the library |
| `player.wasm` | the shared session and the engine, decoding with the browser's WebCodecs; loaded on the first Play |
| `player-codecs.wasm` | the same with Vorbis, ALAC and Opus decoded in Rust, for browsers without them |

It then builds the shell with Vite, copies Inter and Literata from the Android app into `web/src/fonts`
(not in git) and prints each piece's raw and brotli size. SQLite is built without the extras the core
never uses (`CFLAGS_wasm32_unknown_unknown` in the script).

The glyphs are nori's Iconoir set, written into `web/index.html`'s sprite for the names `web/src`
uses. After using a new glyph, run `python3 tools/glyphs.py <iconoir>/icons` again.

### Deploying

`web/dist` is static. Serve it from the same origin as the Subsonic API; the page signs its own
requests, so no CORS headers are needed. Serve `.wasm` as `application/wasm`, precompressed with
brotli in production. A service worker keeps the shell and modules of one build. A new build installs
beside the running one and takes over at the next load, which reloads the page once.

To check that it compiles without building it:

```sh
cargo check -j4 --target wasm32-unknown-unknown -p nori-web                        # the player module
cargo check -j4 --target wasm32-unknown-unknown -p nori-web --no-default-features  # the core module
```

Browser checks are in [Testing](testing.md#the-web-client).

## Releases

`tools/release.sh` builds and publishes a release from this machine: the signed APK, the iPod app,
and macOS and Linux desktop and terminal packages for both architectures. Building needs no GitHub
access; publishing needs GitHub CLI authentication and a clean tree. Windows is not built.

```sh
tools/release.sh                                                       # guided: asks the version, then every step
tools/with-resource.py build tools/release.sh --build                  # build and check everything; publish nothing
tools/with-resource.py build tools/release.sh --publish --beta --live  # publish the version in the source as a public beta
```

| Option | Effect |
| --- | --- |
| `--build` | build into `build/release-<version>/`; nothing leaves the machine |
| `--publish` | build and publish without questions; a draft unless `--live` |
| `--beta` | a GitHub prerelease, offered only to users with beta updates on; notes list the changes since the previous beta |
| `--from DIR` | verify and publish artifacts built by release workers |
| `--abi arm64-v8a` | an APK for phones only |
| `--no-test`, `--no-hosts`, `--no-ipod` | skip the tests, the desktop and terminal packages, or the iPod app |
| `--docker-macos` | build the Mac packages in Docker, even on a Mac |
| `--allow-untested-macos` | acknowledge, on Linux, that the Mac packages cannot be run there |

- Running again for the same commit keeps what was built and tested and does only the rest. A desktop
  platform that fails is left out; running again adds it to the published release.
- The steps run in three lanes side by side: tests then the APK; the two Mac packages; the iPod app then
  the two Linux packages. Each step logs to `build/release-<version>/logs/<step>.log`.
- The APK is signed with `nori-release.jks` through `keystore.properties` (both gitignored). Keep the
  key: without it, installed copies cannot update.
- On Apple Silicon, the Mac packages use the installed SDK and the Intel one is cross-compiled. Linux
  packages build in Ubuntu 22.04 Docker images; the other architecture is cross-compiled and checked
  under emulation. On Linux, install Docker's binfmt handlers once:
  `docker run --privileged --rm tonistiigi/binfmt --install arm64,amd64`.
- A Linux host builds the Mac packages in Docker and cannot run them. It checks signatures and the DMG's
  contents, asks before building, and needs `--allow-untested-macos` when unattended.
- Mac packages are ad-hoc signed, not notarized: a DMG for the desktop, an archive for the terminal.
  Linux gets Debian packages, AppImages and a Flatpak (Freedesktop 25.08 runtime, `nori-cli`
  included).

### Desktop and terminal packages alone

```sh
tools/with-resource.py build tools/host-release.sh                                   # all four platforms
tools/with-resource.py build tools/host-release.sh --source v0.7.0                   # a committed revision
tools/with-resource.py build tools/host-release.sh --platform linux-x86_64           # one platform, in Docker
tools/with-resource.py build tools/host-release.sh --native --platform linux-x86_64  # on this worker, no Docker
```

It also takes `--workspace DIR`, `--output DIR` (default `build/host-release`), `--jobs 1..4`,
`--no-test`, `--docker-macos` and `--allow-untested-macos`. It runs in `build/release-python`, a
Python environment made from `tools/host-release/requirements.txt` on first use (with `uv`, or
`python3` 3.10 or newer). Docker keeps Cargo, sccache and Flatpak caches per architecture in named
volumes.

### Workflows and handoffs

| Workflow | Does |
| --- | --- |
| Full release (`release.yml`) | Android, iPod, all four desktop platforms and the workspace tests from a chosen revision; uploads a `release` artifact, or publishes with `publish` (and `draft` off for a public one). Signing needs the secrets `NORI_RELEASE_KEYSTORE_BASE64` and `NORI_KEYSTORE_PROPERTIES` |
| Desktop and CLI release (`host-release.yml`) | the desktop and terminal packages; uploads to a release when given its tag. Also callable from other workflows |
| macOS rendering checks (`macos-render.yml`) | the desktop's Metal pixel tests on a Mac runner |
| Backfill Linux bundles, Backfill macOS disk images | add bundles or DMGs to an older release from its archives, without rebuilding |

Mac builds in the workflows run on Apple Silicon; the Intel package is checked under Rosetta. The
`macos_on_linux` input builds them on Linux instead and skips running them.

Workers can hand their artifacts to one publisher. Put the APK, the IPA and every desktop package
with its checksum manifest (keeping host subdirectories) in one directory, then, from the matching
source checkout:

```sh
tools/release.sh --build --from build/workflow-release           # verify; no compiler or signing key needed
tools/release.sh --publish --from build/workflow-release --live  # publish without rebuilding
```

The publisher checks every checksum and requires every selected platform before it tags.
