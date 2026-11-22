# Building

This page shows how to build each nori client from source: Android, the desktop app, the terminal app,
the web client and the iPod app. It also covers the release, nightly and perf builds. Run every command
from the repository's root. To run the tests, see [Testing](testing.md).

## Prerequisites

Every build needs these tools:

- Rust stable, installed with rustup.
- sccache: `cargo install --locked sccache`. `.cargo/config.toml` sends every compile through it, so
  Cargo fails without it. To build without it, set `RUSTC_WRAPPER=` (empty), as CI does.
- FFmpeg 8 or newer on the `PATH`, for the Rust tests and the local server. Without FFmpeg, the tests
  that need encoded songs print "ffmpeg is not installed: skipped" and pass.

Each client needs more. Install only what you build:

| Client | You also need |
| --- | --- |
| Android | The Android SDK and NDK, a JDK 17 or newer, `cargo-ndk`, the Rust Android targets |
| Desktop and terminal, macOS | Xcode's command line tools |
| Desktop and terminal, Windows | The MSVC build tools (Visual Studio's C++ workload) |
| Desktop and terminal, Linux | The packages below |
| Web client | The tools in [Build the web client](#build-the-web-client) |
| iPod app, Linux release packages | Docker |

For Android, add the targets and `cargo-ndk`:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
```

On Ubuntu or Debian, the desktop and terminal apps need these packages. They are the names that
`tools/host-release/Dockerfile` installs. WebKitGTK is for the sign-in window.

```sh
sudo apt install build-essential pkg-config clang libasound2-dev libfontconfig1-dev libfreetype6-dev \
  libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
```

Optional tools: `cargo-nextest` for `tools/check.sh rust`, and `bazelisk` for the Bazel backend. On a
Mac, `brew install cargo-nextest bazelisk` installs both.

### Never use more than four jobs

The build machine runs out of memory above four parallel jobs. `.cargo/config.toml` sets `jobs = 4`, and
`gradle.properties` sets four Gradle workers. When you pass `-j` yourself, pass `-j4` or less.

## Build and test the Rust workspace

```sh
cargo build -j4 --workspace
cargo test -j4 --workspace            # every Rust test, doctests included
cargo test -j4 -p nori-player         # one crate
```

The plugin host's tests build a plugin to WebAssembly. Add its target once:
`rustup target add wasm32-wasip2`. Without it, a test fails with "the fixture builds (rustup target add
wasm32-wasip2)".

### Cargo profiles

The workspace defines these profiles in `Cargo.toml`:

| Profile | Used by | What it is |
| --- | --- | --- |
| `dev` | `cargo build`, `cargo test` | Line tables for the workspace, no debug info for dependencies |
| `release` | `cargo run --release`, debug APKs | Optimized and incremental, so an edit rebuilds in seconds |
| `profiling` | `tools/flame.py` | `release` with line tables, so stacks have names |
| `dist` | Release, perf and nightly APKs, desktop packages, the iPod app | Fat LTO, one codegen unit, stripped |
| `web` | `web/build.sh` | `dist` with `opt-level = "z"` and `panic = "abort"` |

- `dist` takes minutes to link.
- Tests build nori-player, nori-engine, nori-android, nori-ios and every dependency at opt-level 2. Debug
  assertions and overflow checks stay on.
- nori-stretch is always at opt-level 3, because it is many times slower unoptimized.
- `dist` builds most code for size (`opt-level = "s"`) and the hot crates at opt-level 3.
- `dist` keeps `panic = "unwind"`, so a panic in the core comes out in Kotlin as an exception.
- To get a debugger's variables, set `CARGO_PROFILE_DEV_DEBUG=true` for that session.

## Build the Android app

### Build the debug APK and install it on the emulator

You need the Android prerequisites and an Android Virtual Device (AVD). Create the AVD in Android
Studio's Device Manager first.

1. Start the emulator headless: `tools/emulator.sh <avd> 5554`.

   The script prints `emulator-5554 ready` when the emulator booted. If it says the emulator did not
   boot, read `build/emulators/emulator-5554.log`.

2. Build the debug APK: `./gradlew :app:assembleDebug`.

   If Gradle says "SDK location not found", set `ANDROID_HOME`, or `sdk.dir` in `local.properties`.
   If it says "NDK not found", set `ANDROID_NDK_HOME` to the NDK's folder. Without that variable, Gradle
   takes the newest NDK under `$ANDROID_HOME/ndk`.

3. Install it: `adb install -r android/app/build/outputs/apk/debug/app-debug.apk`.

   If adb says `INSTALL_FAILED_UPDATE_INCOMPATIBLE`, a copy signed with another key is installed. Remove
   it with `adb uninstall dev.nori.music`, then install again.

4. Open the app: `tools/app.sh launch`.

5. Sign in. For a local server, start `tools/dev-server.sh` and sign in to `http://10.0.2.2:4533` as
   `admin` with the password `admin`. [Testing](testing.md) describes the local server.

A debug build compiles the Rust core for the host's emulator ABI: `arm64-v8a` on Apple Silicon and
other arm64 hosts, `x86_64` elsewhere. It uses the `release` Cargo profile.

To build through the shared queue instead, run `tools/check.sh android`. To check only that the Kotlin
compiles, run `./gradlew :app:compileDebugKotlin -PrustTargets=arm64-v8a`.

### Start several emulators

`tools/emulator.sh <avd> <port>` starts an existing arm64 AVD with host graphics and four CPUs. Give each
emulator its own even port and its own AVD, because an AVD runs only once at a time.
`emulator -list-avds` lists the AVDs.

```sh
tools/emulator.sh first-avd 5554
tools/emulator.sh second-avd 5556
```

[Testing](testing.md) shows how the device checks share these emulators.

### Gradle properties

These properties change what the Android build makes:

| Property | Default | Effect |
| --- | --- | --- |
| `-PrustTargets=arm64-v8a` | Depends on the build type | The ABIs the Rust core is built for, comma-separated |
| `-PrustProfile=dist` | Depends on the build type | The Cargo profile of the native library |
| `-PrustFeatures=` | `neural-beats` | The core's Cargo features |
| `-PrustJniLibs=<dir>` | None | Uses a core already built, one folder per ABI, and skips Cargo |
| `-PpretendVersion=0.3.9` | None | Builds the app as an older version, to test the updater |

A debug build defaults to the host's emulator ABI and the `release` profile. Release, perf and nightly
builds default to `arm64-v8a` and the `dist` profile.

With `-PrustFeatures=` (empty), the library leaves out tract and AutoMix's beat model. AutoMix then uses
the classic beat grids.

Gradle caches the native library and the generated bindings, keyed by their sources, features, profile,
compiler, NDK and build environment. The binding inputs are the crates in `bindingCrates` in
`android/core/build.gradle.kts`. Add a crate to that list when it exports an FFI interface.

### Release APK

`tools/apk.sh` builds a release APK into `build/nori-music-<version>-<abis>.apk`:

```sh
tools/apk.sh                     # arm64-v8a, for phones
tools/apk.sh x86_64              # for an x86_64 emulator
tools/apk.sh arm64-v8a,x86_64    # both ABIs in one, bigger, APK
tools/apk.sh --install           # arm64-v8a, then install it on the connected device
```

The APK uses the release key from `keystore.properties`, or from the environment variables
`KEYSTORE_FILE`, `KEYSTORE_PASSWORD`, `KEY_ALIAS` and `KEY_PASSWORD`. Without a release key, Gradle signs
it with the debug key. That APK installs, but it cannot update a copy signed with the release key.

### Nightly build

The nightly build is a release build named "nori nightly" (`dev.nori.music.nightly`). It installs beside
the real app, with its own sign-in, settings and downloads.

```sh
./gradlew :app:assembleNightly -PrustTargets=arm64-v8a
```

Gradle signs it with the release key when there is one, so each night's build updates it in place. CI
publishes one each night: see [CI workflows](#ci-workflows).

## The perf build

The perf build is a release build with a recorder in it. It measures what the app costs on a real phone
in real use, with no computer attached. It is minified and not debuggable, like a release. It installs
beside the normal app as "nori dev" (`dev.nori.music.perf`), with its own sign-in, settings and
downloads.

```sh
./gradlew :app:assemblePerf -PrustTargets=arm64-v8a    # for a phone
./gradlew :app:assemblePerf -PrustTargets=x86_64       # for an x86_64 emulator
adb install -r android/app/build/outputs/apk/perf/app-perf.apk
```

Gradle signs the perf APK with the Android debug key. You can also copy the APK to the phone and open it
there. The recorder starts with the app, and its page is **Settings › Performance**.

[Performance](perf.md) explains how to read the recorder's figures and run a fair battery test. Keep
debug APKs out of performance comparisons.

## Build the desktop app

```sh
cargo run -j4 --release -p nori-desktop
```

This covers Windows, macOS and Linux. On Linux, install the packages in [Prerequisites](#prerequisites)
first. To drive the window from a script, build it with `--features test-control`, as
[Testing](testing.md) describes.

## Build the terminal app

```sh
cargo run -j4 --release -p nori-cli
```

The terminal app's default feature `sign-in-window` opens a provider's sign-in page in the system web
view. On a machine without a desktop, build it with `--no-default-features`. Signing in then takes
pasted cookies.

## Build the web client

The web client is `crates/web` (nori-web, built for `wasm32-unknown-unknown` with wasm-bindgen) and
`web/` (Svelte 5, TypeScript and Vite). Built natively, nori-web is an empty library, so workspace tests
and clippy skip it.

### Install the web tools

- The target: `rustup target add wasm32-unknown-unknown`.
- wasm-bindgen-cli at the version in `Cargo.lock`: `cargo install wasm-bindgen-cli --version 0.2.128`.
- `wasm-opt` (from binaryen) and `brotli`.
- Node 20 or newer with npm. `web/package-lock.json` pins the packages.
- A clang that targets wasm32, for SQLite's C code: Homebrew LLVM (`brew install llvm`) or the Android
  NDK's clang. `web/build.sh` finds either one. Apple's own clang has no wasm32 backend. To use another
  clang, set `CC_wasm32_unknown_unknown`.

### Build and serve it

```sh
web/build.sh                       # everything into web/dist, then a size table
web/build.sh --shell               # only the Svelte shell, over the wasm modules already built
tools/dev-server.sh                # the local Navidrome, admin/admin
python3 web/serve.py               # http://localhost:8080, with /rest/ passed to localhost:4533
```

If `web/build.sh` says "no clang with a wasm32 target", install Homebrew LLVM or set
`CC_wasm32_unknown_unknown`.

`web/serve.py` is for development only. It takes these options:

| Option | Default | Effect |
| --- | --- | --- |
| `--port` | `8080` | The port it serves on |
| `--server URL` | `http://localhost:4533` | The Subsonic server it passes `/rest/` to |
| `--log` | Off | Prints each API request |
| `--kbps N` | No limit | Sends the API's answers at most this fast |
| `--files-kbps N` | No limit | Sends the client's own files at most this fast |

`web/build.sh` builds three wasm modules, each through `wasm-opt -Oz`:

| Module | Holds |
| --- | --- |
| `core.wasm` | Sign-in and the library |
| `player.wasm` | The shared session and the engine, decoding with the browser's WebCodecs |
| `player-codecs.wasm` | The same, with Vorbis, ALAC and Opus decoded in Rust |

The page loads `player.wasm` on the first Play. It loads `player-codecs.wasm` only in a browser without
those codecs. The script then builds the shell with Vite and cuts Inter and Literata into WOFF2 in
`web/src/fonts` (`web/fonts.mjs`). It writes `.br` and `.gz` copies beside every file, and prints each
file's raw and brotli size.

The glyphs are nori's Iconoir set, in the sprite in `web/index.html`. After you use a new glyph in
`web/src`, run `python3 tools/glyphs.py <iconoir>/icons` again.

To check that the web crate compiles, without a full build:

```sh
tools/check.sh web                 # CI's wasm clippy of both modules
cargo check -j4 --target wasm32-unknown-unknown -p nori-web                        # the player module
cargo check -j4 --target wasm32-unknown-unknown -p nori-web --no-default-features  # the core module
```

[Testing](testing.md) has the browser checks.

### Deploy it

`web/dist` is static. How the page finds its server depends on where you serve it:

- From the same origin as a Subsonic API (its `/rest/ping` answers), the page signs in to that server
  only. It asks for no address.
- From anywhere else, it asks for the server's address. That server must answer CORS preflights.
  Navidrome does, and octo-fiesta does from commit `fbe3dc1`.

The page needs a secure context (https, or localhost) for its databases. A page served over https cannot
reach a server over plain http. Serve `.wasm` as `application/wasm`, and send the `.br` and `.gz` copies.
In nginx, that is `gzip_static on;`, and `brotli_static on;` with its brotli module. A server that sends
the plain files costs about three times the bytes on a first load.

A service worker keeps the shell and modules of one build. A new build installs beside the running one
and takes over at the next load, which reloads the page once.

Every push to master that touches `web/` builds the image `ghcr.io/norifm/nori-web`
(`.github/workflows/web.yml`). It holds only `web/dist`, at its root, for a server's image to copy
(`web/Dockerfile`).

## Build the iPod app

`tools/ipod.sh build` builds the iPod app in Docker. Docker is the only thing the machine needs. With an
iPod on USB, `tools/ipod.sh` also installs and runs it. [iPod internals](ipod-internals.md) covers the
setup, the device and the measurements.

## Check the crates without `std`

nori-player, nori-model and nori-remote also build with their `std` feature off, for firmware on a
microcontroller. nori-sink-protocol has no `std` feature at all. This check needs the nightly toolchain
with the `riscv32imafc-unknown-none-elf` target:

```sh
cargo +nightly check -p nori-player -p nori-model -p nori-remote -p nori-sink-protocol --no-default-features --target riscv32imafc-unknown-none-elf
```

Without `std`, each crate keeps only part of itself:

| Crate | What builds without `std` |
| --- | --- |
| nori-player | The sound chain's DSP and the records that set it, with float math from libm |
| nori-model | The library records and their JSON |
| nori-remote | The frames (`wire`), what a device admits (`device`) and the LAN's proof |
| nori-sink-protocol | All of it |

## Share the machine with other worktrees

Heavy commands go through one machine-wide queue, so worktrees and agents take turns:

| Command | Runs |
| --- | --- |
| `tools/check.sh rust --workspace` | `cargo nextest run` with four build jobs and four test threads |
| `tools/check.sh rust -p <crate>` | The same, for one crate |
| `tools/check.sh rust-doc --workspace` | The doctests, which nextest skips |
| `tools/check.sh web` | CI's wasm clippy of both web modules |
| `tools/check.sh ipod` | Clippy of nori-ios alone, without the desktop parts |
| `tools/check.sh android [GRADLE OPTIONS]` | `./gradlew :app:assembleDebug` |
| `tools/check.sh bazel <command> [...]` | A Bazel command |
| `python3 tools/with-resource.py build <command>` | Any other command |

For a full Rust check with the doctests, run
`python3 tools/with-resource.py build cargo test -j4 --workspace`. A command run directly keeps the
four-job limit but skips the queue.

`tools/check.sh web` and `tools/check.sh ipod` catch what a workspace build misses. Natively, nori-web is
empty. A workspace build turns on the desktop parts of nori-host, which the iPod app leaves out.

### Worktrees and disk space

Each worktree has its own `target/` and Gradle `build/`. A shared Cargo target folder would let one
worktree overwrite another's `target/debug/nori-cli` and Android library. Cargo's lock would also run
every build one after another. The worktrees share these caches instead:

- sccache keeps compiled dependencies for every worktree, up to 30 GB (`SCCACHE_CACHE_SIZE` in
  `.cargo/config.toml`). It cannot cache incremental workspace crates or links.
- Gradle's build cache (`~/.gradle/caches`) keeps the native library, the bindings and the Kotlin
  classes. A changed `Cargo.toml` or `Cargo.lock` misses it.
- Bazel's disk cache is `~/.cache/nori/bazel`.

When you remove a worktree, its folders go with it. The main checkout's `target/` only grows, because
every profile, feature set and target keeps its own files. `cargo clean` gives the space back. So does
`cargo sweep --time 14`, from `cargo install cargo-sweep`. The largest part is often
`target/debug/incremental`, which you can delete.

### Time a development command

On macOS, `tools/measure-dev.py` times a command and samples the machine while it runs:

```sh
python3 tools/measure-dev.py --name rust-edit tools/check.sh android
python3 tools/measure-dev.py --name smoke env NORI_E2E_SERVER=local tools/device-check.sh smoke
```

It writes to `build/development-timings/<name>/`. It records the wall time, the CPU of the command's
process tree, the compiler, JVM and emulator CPU and memory, and the whole machine's activity. The
emulator and JVM figures include every matching process on the host. 100 % CPU is one core.

## Native builds with Bazel (optional)

Bazel is an optional backend for cached native builds and tests across worktrees. Cargo manifests and
`Cargo.lock` stay the source of truth. APKs, the iPod app, release packages, doctests and device checks
keep their own tools.

```sh
python3 tools/rust.py test --workspace
python3 tools/rust.py test -p nori-player -- pipeline     # arguments after -- go to the test binaries
python3 tools/rust.py build -p nori-desktop
python3 tools/rust.py run -p nori-cli
```

`tools/rust.py` writes the BUILD files from the Cargo metadata, then runs Bazel through the shared queue.
After you change a manifest, regenerate and check them. Never edit the generated BUILD files.

```sh
python3 tools/bazel/generate.py
python3 tools/bazel/generate.py --check
python3 -m unittest discover -s tools/bazel -p 'test_*.py'
tools/check.sh bazel test //crates/...
```

- Tests build with the workspace's default and development features. `-p` narrows the targets, not the
  features. Desktop and terminal builds use their shipping features.
- Other feature selections and cross-compiling do not work with Bazel.
- Bazel builds are optimized (`--compilation_mode=opt`) and are for development. Releases use Cargo.
- Native tests need the same platform libraries and FFmpeg as Cargo tests.
- Bazel caches a passing test and does not run it again. To run it anyway, add `--nocache_test_results`.
- Pass `--remote_cache` to a direct Bazel command where a remote cache exists. Remote execution is not
  set up.

## Make a release

`tools/release.sh` builds and publishes a release from this machine. It builds the signed APKs, the iPod
app, and the macOS and Linux desktop and terminal packages for both architectures. Building needs no
GitHub access. Publishing needs GitHub CLI authentication and a clean tree. It does not build Windows.

```sh
tools/release.sh                                                       # guided: asks the version, then does every step
tools/with-resource.py build tools/release.sh --build                  # build and check everything, publish nothing
tools/with-resource.py build tools/release.sh --publish --beta --live  # publish the source's version as a public beta
```

The guided release can also push the release commit and leave the build to the Full release workflow
(`release.yml`). That workflow builds Windows too, and signs with the key in the repository's secrets.

| Option | Effect |
| --- | --- |
| `--build` | Builds into `build/release-<version>/`, and publishes nothing |
| `--publish` | Builds and publishes without questions, as a draft unless `--live` |
| `--live` | With `--publish`, publishes the release instead of a draft |
| `--beta` | Makes a GitHub prerelease, offered only to users with beta updates on |
| `--from DIR` | Verifies and publishes artifacts that release workers built |
| `--abi arm64-v8a` | Builds the APK for one ABI only |
| `--jni-libs DIR` | Builds the APK around a core already built, in `DIR/<abi>/` |
| `--no-test` | Skips the tests |
| `--no-hosts` | Skips the desktop and terminal packages |
| `--no-ipod` | Skips the iPod app |
| `--check-notes` | Only checks that `CHANGELOG.md` has this version's section |
| `--docker-macos` | Builds the Mac packages in Docker, even on a Mac |
| `--allow-untested-macos` | On Linux, accepts that the Mac packages cannot run there |

How a release run behaves:

- By default it makes one APK per ABI: `arm64-v8a`, `armeabi-v7a` and `x86_64`.
- A second run for the same commit keeps what it built and tested, and does only the rest.
- A desktop platform that fails is left out. A second run adds it to the published release.
- The steps run in three lanes side by side. The first runs the tests, then builds the APK. The second
  builds the two Mac packages. The third builds the iPod app, then the two Linux packages.
- Each step logs to `build/release-<version>/logs/<step>.log`.
- A beta's notes list the changes since the previous beta. `tools/changelog.py` writes them, and
  `CHANGELOG.md` stays unchanged.

### Keep the signing key

Gradle signs the APK with `nori-release.jks` through `keystore.properties`. Git ignores both files. The key
is what lets an installed copy update. If you lose it, every user must remove the app before the next
release installs.

### What each platform gets

- macOS: ad-hoc signed packages, not notarized. The desktop app is a DMG and the terminal app an archive.
  On Apple Silicon, the Intel package is cross-compiled.
- Linux: Debian packages, AppImages and a Flatpak (GNOME 50 runtime, with `nori-cli`). They build
  in Ubuntu 22.04 Docker images. The other architecture is cross-compiled and checked under emulation.
- Windows (x86-64): a zip per client, unsigned. Only a Windows worker builds it, so CI's release and
  nightly runs have it and a local release does not.

On Linux, install Docker's binfmt handlers once:
`docker run --privileged --rm tonistiigi/binfmt --install arm64,amd64`. A Linux host builds the Mac
packages in Docker but cannot run them. It checks their signatures and the DMG's contents. It asks
before it builds them, and needs `--allow-untested-macos` when it runs unattended.

### Build only the desktop and terminal packages

```sh
tools/with-resource.py build tools/host-release.sh                                   # all platforms but Windows
tools/with-resource.py build tools/host-release.sh --source v0.7.0                   # a committed revision
tools/with-resource.py build tools/host-release.sh --platform linux-x86_64           # one platform, in Docker
tools/with-resource.py build tools/host-release.sh --native --platform linux-x86_64  # on this machine, no Docker
```

The platforms are `macos-aarch64`, `macos-x86_64`, `linux-aarch64`, `linux-x86_64` and
`windows-x86_64`. The script also takes `--workspace DIR`, `--output DIR` (default `build/host-release`),
`--jobs 1..4`, `--no-test`, `--docker-macos` and `--allow-untested-macos`.

- It runs in `build/release-python`, a Python environment made from
  `tools/host-release/requirements.txt`. It uses `uv` when installed, or else `python3` 3.10 or newer.
- A `--native` Linux build needs appimagetool first: `sudo tools/host-release/appimagetool.sh`.
- Docker keeps the Cargo, sccache and Flatpak caches for each architecture in named volumes.

### Publish what release workers built

Workers can hand their artifacts to one publisher. Put the APKs, the IPA and every desktop package with
its checksum manifest in one folder, and keep the host subfolders. Then, from the matching source
checkout, run:

```sh
tools/release.sh --build --from build/workflow-release           # verify; no compiler or signing key needed
tools/release.sh --publish --from build/workflow-release --live  # publish without rebuilding
```

The publisher checks every checksum. It requires every selected platform before it tags.

## CI workflows

The workflows are in `.github/workflows/`:

| Workflow | When it runs | What it does |
| --- | --- | --- |
| Checks (`checks.yml`) | Every push, any branch | Clippy, tests, the web build, the Kotlin compile, the iPod build |
| Full release (`release.yml`) | By hand | Every platform from a chosen revision, published or as an artifact |
| Nightly (`nightly.yml`) | 02:00 UTC, when master moved | Replaces the `nightly` prerelease with new builds |
| Desktop and CLI release (`host-release.yml`) | By hand, or from another workflow | The desktop and terminal packages |
| macOS rendering checks (`macos-render.yml`) | By hand | The desktop's Metal pixel tests, on a Mac runner |
| Web image (`web.yml`) | Pushes to master that touch `web/` | The `ghcr.io/norifm/nori-web` image |
| Caches (`caches.yml`) | After the others, and daily | Deletes caches no job restores |
| Backfill Linux bundles, Backfill macOS disk images | By hand | Adds bundles or DMGs to an older release |

Checks runs only the jobs a push can affect. [Testing](testing.md) says how it picks the Rust tests.

The Full release workflow takes a `source` revision. With `publish` on, it creates the GitHub release,
and `draft` off makes it public. Signing needs the secrets `NORI_RELEASE_KEYSTORE_BASE64` and
`NORI_KEYSTORE_PROPERTIES`.

The nightly build's version is `<x.y.z>-nightly.<commit time>`, set by `tools/bump-version.sh` without a
commit. Its files, the apps and the release title carry that version. The updater in nori nightly reads that
title to offer the next night's build (the nightly channel in `update.rs`). Stable and beta builds never
see it. A release published after it would push it down the list. The workflow then makes the nightly
again, over the same commit and files, so it stays first.

### Mac builds

The Mac builds run on Apple Silicon, on the `macos-26` runner. CI checks the Intel package under
Rosetta. The system draws the window buttons and other chrome in the Liquid Glass look only for apps
linked against the macOS 26 SDK or newer. The command `vtool -show-build` shows the SDK. So a release never builds
on an older runner. The `macos_on_linux` input builds them on Linux instead and skips running them. Those
packages link an older SDK and keep the older look.

### Caches

The workflows cache Rust dependencies (`Swatinem/rust-cache`, one key per job and platform), Gradle and
npm. Only master saves caches, and every branch reads them, so branch pushes do not evict them from
GitHub's 10 GB. The release link (fat LTO) is never cached.

A new `Cargo.lock`, Rust version or apt package list saves a new cache. `caches.yml` then deletes the
cache it replaced. It deletes a branch's caches a day after their last use.

The test job always builds the whole workspace and runs only the affected crates' tests
(`tools/affected.py --tests`). A build of only some crates resolves the dependencies' features
differently, and would rebuild them instead of using the cache.
