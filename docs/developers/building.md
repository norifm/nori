# Building

This page shows how to build each nori client from source, and the release, nightly and perf builds.
Run every command from the repository's root. To run the tests, see [Testing](testing.md).

## Prerequisites

Every build needs these tools:

- Rust stable, installed with rustup.
- sccache: `cargo install --locked sccache`. `.cargo/config.toml` sends every compile through it, so
  Cargo fails without it. To build without it, set `RUSTC_WRAPPER=` (empty), as CI does.
- FFmpeg 8 or newer on the `PATH`, for the Rust tests and the local server.

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

On Ubuntu or Debian, the desktop and terminal apps need these packages, the ones that
`tools/host-release/Dockerfile` installs:

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

`dist` takes minutes to link. It keeps `panic = "unwind"`, so a panic in the core comes out in Kotlin as
an exception. To get a
debugger's variables, set `CARGO_PROFILE_DEV_DEBUG=true` for that session.

## Build the Android app

### Build the debug APK and install it on the emulator

You need the Android prerequisites and an Android Virtual Device (AVD), made in Android Studio's Device
Manager.

1. Start the emulator headless: `tools/emulator.sh <avd> 5554`.

   The script prints `emulator-5554 ready` when the emulator booted. If it says the emulator did not
   boot, read `build/emulators/emulator-5554.log`.

2. Build the debug APK: `./gradlew :app:assembleDebug`.

   If Gradle says "SDK location not found", set `ANDROID_HOME`, or `sdk.dir` in `local.properties`.
   If it says "NDK not found", set `ANDROID_NDK_HOME` to the NDK's folder.

3. Install it: `adb install -r android/app/build/outputs/apk/debug/app-debug.apk`.

   If adb says `INSTALL_FAILED_UPDATE_INCOMPATIBLE`, a copy signed with another key is installed. Remove
   it with `adb uninstall dev.nori.music`, then install again.

4. Open the app: `tools/app.sh launch`.

5. Sign in. For a local server, start `tools/dev-server.sh` and sign in to `http://10.0.2.2:4533` as
   `admin` with the password `admin`.

A debug build compiles the Rust core for the host's emulator ABI (`arm64-v8a` on arm64 hosts, `x86_64`
elsewhere), with the `release` Cargo profile. To build through the shared queue instead, run
`tools/check.sh android`. To check only that the Kotlin compiles, run
`./gradlew :app:compileDebugKotlin -PrustTargets=arm64-v8a`.

### Start several emulators

Give each emulator its own AVD and its own even port. An AVD runs only once at a time, and
`emulator -list-avds` lists them.

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
| `-PrustFeatures=` | `neural-beats` | The core's Cargo features, empty to leave out the beat model |
| `-PrustJniLibs=<dir>` | None | Uses a core already built, one folder per ABI, and skips Cargo |
| `-PpretendVersion=0.3.9` | None | Builds the app as an older version, to test the updater |

A debug build defaults to the host's emulator ABI and the `release` profile. Release, perf and nightly
builds default to `arm64-v8a` and the `dist` profile.

Gradle caches the native library and the generated bindings. The binding inputs are the crates in
`bindingCrates` in `android/core/build.gradle.kts`: add a crate there when it exports an FFI interface.

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
it with the debug key. That APK cannot update a copy signed with the release key.

### Nightly build

The nightly build is a release build named "nori nightly" (`dev.nori.music.nightly`). It installs beside
the real app, with its own sign-in, settings and downloads. CI publishes one each night.

```sh
./gradlew :app:assembleNightly -PrustTargets=arm64-v8a
```

## The perf build

The perf build is a release build with a recorder in it. It measures what the app costs on a real phone
in real use, with no computer attached. It installs beside the normal app as "nori dev"
(`dev.nori.music.perf`), with its own sign-in, settings and downloads. Gradle signs it with the debug key.

```sh
./gradlew :app:assemblePerf -PrustTargets=arm64-v8a    # for a phone
./gradlew :app:assemblePerf -PrustTargets=x86_64       # for an x86_64 emulator
adb install -r android/app/build/outputs/apk/perf/app-perf.apk
```

The recorder starts with the app, and its page is **Settings › Performance**. [Performance](perf.md)
explains how to read its figures and run a fair battery test. Keep debug APKs out of performance
comparisons.

## Build the desktop app

```sh
cargo run -j4 --release -p nori-desktop
```

This covers Windows, macOS and Linux. On Linux, install the packages in [Prerequisites](#prerequisites)
first.

## Build the terminal app

```sh
cargo run -j4 --release -p nori-cli
```

On a machine without a desktop, build it with `--no-default-features`. That leaves out the
`sign-in-window` feature, so signing in to a provider takes pasted cookies.

## Build the web client

The web client is `crates/web` (nori-web, built for `wasm32-unknown-unknown` with wasm-bindgen) and
`web/` (Svelte 5, TypeScript and Vite). Built natively, nori-web is an empty library, so workspace tests
and clippy skip it.

### Install the web tools

- The target: `rustup target add wasm32-unknown-unknown`.
- wasm-bindgen-cli at the version in `Cargo.lock`: `cargo install wasm-bindgen-cli --version 0.2.128`.
- `wasm-opt` (from binaryen) and `brotli`.
- Node 20 or newer with npm.
- A clang that targets wasm32, for SQLite's C code: Homebrew LLVM (`brew install llvm`) or the Android
  NDK's clang. Apple's own clang has no wasm32 backend. To use another clang, set
  `CC_wasm32_unknown_unknown`.

### Build and serve it

```sh
web/build.sh                       # everything into web/dist, then a size table
web/build.sh --shell               # only the Svelte shell, over the wasm modules already built
tools/dev-server.sh                # the local Navidrome, admin/admin
python3 web/serve.py               # http://localhost:8080, with /rest/ passed to localhost:4533
```

If `web/build.sh` says "no clang with a wasm32 target", install Homebrew LLVM or set
`CC_wasm32_unknown_unknown`.

`web/serve.py` is for development only. It also takes `--port`, `--server URL` (the Subsonic server for
`/rest/`), `--log`, and `--kbps N` and `--files-kbps N` to slow the API or the client's own files.

`web/build.sh` builds three wasm modules. `core.wasm` holds sign-in and the library. `player.wasm` holds
the session and the engine, decoding with WebCodecs. `player-codecs.wasm` is the same with Vorbis, ALAC
and Opus decoded in Rust, for browsers without them. The page loads a player module at the first Play.

The glyphs are nori's Iconoir set, in the sprite in `web/index.html`. After you use a new glyph in
`web/src`, run `python3 tools/glyphs.py <iconoir>/icons` again.

To check that the web crate compiles, without a full build:

```sh
tools/check.sh web                 # CI's wasm clippy of both modules
cargo check -j4 --target wasm32-unknown-unknown -p nori-web                        # the player module
cargo check -j4 --target wasm32-unknown-unknown -p nori-web --no-default-features  # the core module
```

### Deploy it

`web/dist` is static. How the page finds its server depends on where you serve it:

- From the same origin as a Subsonic API (its `/rest/ping` answers), the page signs in to that server
  only.
- From anywhere else, it asks for the server's address. That server must answer CORS preflights.
  Navidrome does, and octo-fiesta does from commit `fbe3dc1`.

The page needs a secure context (https, or localhost), and a page served over https cannot reach a
server over plain http. Serve `.wasm` as `application/wasm`, and send the `.br` and `.gz` copies that
the build writes beside every file. In nginx, that is `gzip_static on;`, and `brotli_static on;` with its
brotli module. Without them, a first load costs about three times the bytes.

A service worker keeps the shell and modules of one build. A new build takes over at the next load,
which reloads the page once.

Every push to master that touches `web/` builds the image `ghcr.io/norifm/nori-web`. It holds only
`web/dist`, at its root, for a server's image to copy (`web/Dockerfile`).

## Build the iPod app

`tools/ipod.sh build` builds the iPod app in Docker, the only thing the machine needs. With an iPod on
USB, `tools/ipod.sh` also installs and runs it. [iPod internals](ipod-internals.md) covers the setup.

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

`tools/check.sh web` and `tools/check.sh ipod` catch what a workspace build misses. Natively, nori-web is
empty, and a workspace build turns on the desktop parts of nori-host that the iPod app leaves out.

### Worktrees and disk space

Each worktree has its own `target/` and Gradle `build/`, so builds in two worktrees neither overwrite
each other's binaries nor wait on one Cargo lock. They share these caches:

- sccache keeps compiled dependencies, up to 30 GB (`SCCACHE_CACHE_SIZE` in `.cargo/config.toml`). It
  cannot cache incremental workspace crates or links.
- Gradle's build cache (`~/.gradle/caches`) keeps the native library, the bindings and the Kotlin
  classes. A changed `Cargo.toml` or `Cargo.lock` misses it.
- Bazel's disk cache is `~/.cache/nori/bazel`.

The main checkout's `target/` only grows. `cargo clean` or `cargo sweep --time 14` (from
`cargo install cargo-sweep`) gives the space back. The largest part is often `target/debug/incremental`,
which you can delete.

To time a development command on macOS, run it through `tools/measure-dev.py`. It writes the wall time
and the CPU and memory of the command, the compiler, the JVM and the emulator to
`build/development-timings/<name>/`:

```sh
python3 tools/measure-dev.py --name rust-edit tools/check.sh android
```

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
tools/check.sh bazel test //crates/...
```

- Tests build with the workspace's default and development features. `-p` narrows the targets, not the
  features. Other feature selections and cross-compiling do not work.
- Bazel builds are optimized and are for development. Releases use Cargo.
- Bazel caches a passing test and does not run it again. To run it anyway, add `--nocache_test_results`.

## Make a release

`tools/release.sh` builds and publishes a release from this machine. It builds the signed APKs, the iPod
app, and the macOS and Linux desktop and terminal packages for both architectures. Publishing needs GitHub CLI
authentication and a clean tree. It does not build Windows.

```sh
tools/release.sh                                                       # guided: asks the version, then does every step
tools/with-resource.py build tools/release.sh --build                  # build and check everything, publish nothing
tools/with-resource.py build tools/release.sh --publish --beta --live  # publish the source's version as a public beta
```

The guided release can also push the release commit and leave the build to the Full release workflow
(`release.yml`), which builds Windows too.

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

- By default it makes one APK per ABI: `arm64-v8a`, `armeabi-v7a` and `x86_64`.
- A second run for the same commit keeps what it built and tested, and does only the rest. A desktop
  platform that failed is added then.
- Each step logs to `build/release-<version>/logs/<step>.log`.
- A beta's notes list the changes since the previous beta (`tools/changelog.py`). `CHANGELOG.md` stays
  unchanged.

### Keep the signing key

Gradle signs the APK with `nori-release.jks` through `keystore.properties`. Git ignores both files. If
you lose the key, every user must remove the app before the next release installs.

### What each platform gets

- macOS: ad-hoc signed packages, not notarized. The desktop app is a DMG and the terminal app an archive.
- Linux: Debian packages, AppImages and a Flatpak, built in Ubuntu 22.04 Docker images. The other
  architecture is cross-compiled and checked under emulation.
- Windows (x86-64): a zip per client, unsigned. Only CI's release and nightly runs build it.

On Linux, install Docker's binfmt handlers once:
`docker run --privileged --rm tonistiigi/binfmt --install arm64,amd64`. A Linux host builds the Mac
packages in Docker but cannot run them, so it checks only their signatures and the DMG's contents.

### Build only the desktop and terminal packages

```sh
tools/with-resource.py build tools/host-release.sh                                   # all platforms but Windows
tools/with-resource.py build tools/host-release.sh --source v0.7.0                   # a committed revision
tools/with-resource.py build tools/host-release.sh --platform linux-x86_64           # one platform, in Docker
tools/with-resource.py build tools/host-release.sh --native --platform linux-x86_64  # on this machine, no Docker
```

The platforms are `macos-aarch64`, `macos-x86_64`, `linux-aarch64`, `linux-x86_64` and
`windows-x86_64`. The output goes to `build/host-release`, and `--help` lists the other options. The
script needs `uv` or `python3` 3.10 or newer. A `--native` Linux build needs appimagetool first:
`sudo tools/host-release/appimagetool.sh`.

### Publish what release workers built

Put the APKs, the IPA and every desktop package with its checksum manifest in one folder, and keep the
host subfolders. Then, from the matching source checkout, run:

```sh
tools/release.sh --build --from build/workflow-release           # verify; no compiler or signing key needed
tools/release.sh --publish --from build/workflow-release --live  # publish without rebuilding
```

The publisher checks every checksum, and requires every selected platform before it tags.

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

The nightly's version is `<x.y.z>-nightly.<commit time>`, and nori nightly's updater reads it from the
release title. Stable and beta builds never see it. When a release is published after it, the workflow
makes the nightly again so it stays first in the list.

### Mac builds

The Mac builds run on Apple Silicon, on the `macos-26` runner, and CI checks the Intel package under
Rosetta. Only an app linked against the macOS 26 SDK or newer gets the Liquid Glass window chrome. So a release
never builds on an older runner. The `macos_on_linux` input builds them on Linux instead, with an
older SDK and look.

### Caches

The workflows cache Rust dependencies (`Swatinem/rust-cache`, one key per job and platform), Gradle and
npm. Only master saves caches, and every branch reads them, so branch pushes do not evict them from
GitHub's 10 GB. The release link (fat LTO) is never cached. `caches.yml` deletes a replaced cache, and a
branch's caches a day after their last use.

The test job always builds the whole workspace and runs only the affected crates' tests
(`tools/affected.py --tests`). A build of only some crates would resolve the dependencies' features
differently and miss the cache.
