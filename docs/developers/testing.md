# Testing

Applies to the nori repository at 0.7 (master is 0.7.1-beta). Commands run from the repository's root.
The rules for writing tests are in [AGENTS.md](../../AGENTS.md#tests).

Everything that decides how music plays, the queue, the library, lyrics, remote control and settings is
Rust, tested by `cargo test`, on a virtual clock where time matters. Device checks cover only what
Rust cannot reach: the platform's audio output, media3, the media session, focus, routing, JNI, the
service, the screens and a force stop.

## Tiers

| Tier | Covers | Who runs it |
| --- | --- | --- |
| `cargo test -j4 --workspace` | every Rust test, doctests included; the plugin host's build a plugin to wasm (`rustup target add wasm32-wasip2` once) | every change |
| `./gradlew :core:testDebugUnitTest :app:testDebugUnitTest` | the Kotlin unit tests, no device | changes to Kotlin |
| `tools/smoke.sh` | launch, play, controls, a queue edit, an AutoMix transition, the equalizer, offload, the notification, offline playback, crashes; about two minutes | every change that reaches Android |
| `tools/audio-e2e.sh --only …`, `tools/feature-e2e.sh --only …` | the sections a change touches | the change's author |
| `tools/audio-e2e.sh`, `tools/feature-e2e.sh` in full | every device check | once per batch |

For the shared queue, nextest, Bazel and running several emulators, see
[Building](building.md#faster-development-checks).

## Rust tests

### What each crate's tests cover

| Crate | Checked |
| --- | --- |
| player | the sound chain sample by sample (decoders, ReplayGain, equalizer, compressor, limiter, speed, silence skipping), AutoMix analysis on synthetic songs of known tempo, key and structure, the planner (never panicking over any stored row: `automix/plan_fuzz.rs`), the mixer, and the whole player on a simulated output (`sim`, `tests/pipeline`): gapless joins, crossfades, levels, controls |
| engine | the player on its own thread, on the virtual clock of `tests/common`: loading, fetching ahead, the stream cache, offload on a simulated chip (`paths.rs`), radio, tempo, stretched mixes (`stretch.rs`), estimated transcode lengths and their 416 (`estimated.rs`, `transcode.rs`), streams that hang (`hung.rs`), never playing silent (`silent.rs`), one fetch per song (`one_fetch.rs`), settings changed while playing (`replan.rs`), albums kept gapless (`album.rs`), jam guests listening along (`along.rs`), pushed and late-decoded songs for the web (`pushed.rs`, `driven.rs`, `later.rs`), the core's queue (`core.rs`) |
| core | the FFI surface over real SQLite: index and search, smart playlists, mixes, history, lyrics, covers, AutoEQ, the Subsonic client against a fake transport, transfers, the car's tree; whole flows against an in-memory server (`tests/scenario.rs`); remote control and jams over an in-memory relay (`tests/remote.rs`); listens kept by profile (`tests/scrobble.rs`); Kotlin twins (`tests/twins.rs`) |
| host | the shared session: queue saving, profiles and jam guest profiles, signing out, autofill and radio (`refill_tests.rs`), remote control and volume (`remote.rs`), LAN jams and pairing (`tests/lan_jam.rs`) |
| lyrics | every format, the services' answers, trust and fitting, the race between services, sync against synthetic sung songs (`sync.rs`) |
| covers | decoders against Pillow's output, the scaler, disk and memory caches, the loader |
| look | colors from covers (the AndroidX Palette port), themes, seek bar and lyric pacing |
| settings | the store; `tests/stored_format.rs` holds the stored keys and encodings to a golden copy (`NORI_BLESS=1` only for an intended change) |
| library, queue, transfers, devices, perf | pages, menus, queue rules, the download table, AutoEQ and device profiles, the perf report |
| android | the AudioTrack writer on a simulated track (`track.rs`, `track/air.rs`), every JNI door against its Kotlin `external fun` (`lib.rs`), async JNI calls keeping their receiver (`receiver_tests.rs`) |
| ios | `IosOutput` on a simulated sink, the session behind the C ABI on `WavOutput`, and the doors the screens read |
| cli, desktop | screens (ratatui `TestBackend`; Slint's testing backend on virtual time) |
| testdir | every file under a crate's `tests/` is in a test binary (`tests/registered.rs`); AGENTS.md's Layout table matches the crates and its paths exist (`tests/agents_map.rs`) |

`[profile.test.package.…]` in Cargo.toml builds nori-player, nori-engine, nori-android, nori-ios and
every dependency at opt-level 2 for tests, with debug assertions and overflow checks on. Other
workspace crates, nori-core among them, are unoptimized, so their tests keep their data small.

Kotlin twins: `tools/twins.sh` runs the Kotlin originals in `crates/core/testdata/twins/*.kt` on the
JVM and writes their answers next to them; `tests/twins.rs` holds the Rust versions to them. Run it again
after changing either side.

### The engine's virtual clock

- Time moves only when the test moves it (`Rig::run`, `wait_for`, `until`). Never `thread::sleep`.
  Something that wakes the engine outside a command goes through `Virtual::woke_engine`.
- The clock stands still while the engine waits for bytes and while a loader takes bytes in (up to
  `BYTES_WAIT`, 2 s of real time without progress; 20 ms while a server waits on the clock through
  `Virtual::wait_until`). The engine lets its loaders finish before reading what they hold
  (`let_loaders_run`), so an in-memory server acts as an instant network under any scheduling.
- The loaders' waits (stalls, retries) come from the clock's `nori_engine::Waits`, which the virtual
  clock shortens. A server paced on the clock stops it at each time it waits for.
- Read what the test card heard, not the status.
- Songs are made once per binary: `music()` in `engine.rs` and `pipeline/common.rs`, `steady()` in
  `levels.rs`, and the FFmpeg encodes in `paths.rs` and `estimated.rs` are cached by length and seed.
- Process-wide state (the core's queue, the active client, the AutoEQ fetch, the watch hook) gets its own
  test binary, or the test filters to what it made itself.
- A table of slow cases runs each case on its own thread (`each` in `automix/tests.rs`) or becomes one
  test per row.
- crates/engine and crates/core have `autotests = false`: a new test file needs a `[[test]]` in
  Cargo.toml or a `#[path]` module in `tests/main.rs`. A new file in `crates/player/tests/pipeline`
  needs a `mod` in its `main.rs`. testdir's `every_test_file_is_registered` names a file that is in no
  binary.

### Reading a failure

- Asserts print expected and actual, song ids, positions, and the engine's event log (`rig.events`) or
  the planner's log. Read the last event first: an `Error`, `Stopped` or `Bridge` there is the cause.
- "the engine did not go to sleep" means the engine thread is stuck or spinning, not slow: the limit is
  120 s of real time.
- A virtual-clock test that fails only under load has a real thread the clock doesn't see. Make that
  thread wait for a condition, not for time. To find one, run the test binary with `--exact NAME` in
  sixteen parallel loops and compare logs: on the virtual clock, every run prints the same.

### Reference renders

- `crates/engine/tests/common/reference.rs` renders offline what the card should hear when the sound
  changes mid-play (equalizer, compressor, speed, silence skipping), switched at the frame the engine
  logged. The `*_changes_seamlessly*` tests in `engine.rs` compare the card with it sample for sample,
  in crossfades, stretched mixes, after seeks and while paused. `reference::clicks` finds jumps where no
  exact render exists.
- `crates/android/src/track/air.rs` models the Android mixer: periods, volume ramps, output latency,
  Bluetooth's late and jittery timestamps. It checks that every frame is heard once, with no gap, level
  step or click, when the track is resized, paused or jumps, for periods from 5 to 40 ms and Bluetooth.

### The host perf report

`tools/perf-host.sh [rev] [runs]` plays minutes of music through the engine on the test clock, for this
checkout and another revision (the latest release tag by default), and prints per minute of music the
engine's wakes, the CPU time and the allocations, for plain playback, the equalizer and crossfades. The
bench is the ignored test `crates/engine/tests/perf_bench.rs`; the script gives an older revision its
files. Only a comparison on the same machine means anything. On a phone, use
[the perf build](building.md#the-perf-build).

## Kotlin unit tests

`./gradlew :core:testDebugUnitTest :app:testDebugUnitTest` (no device) covers what the Kotlin keeps:

- formatting, frame fades, cover placement, overlays, song lists, selection and setting keys;
- queue edits, drag and undo (`QueueEditsTest`);
- the player's cover across fast skips and out-of-order loads (`CoverTurnTest`, `CoverFetchTest`);
- the stream bridge handing available bytes to Rust before the next network read (`RustBodyTest`), and
  telling a read past the end from a dropped network (`PastEndTest`);
- releasing the player after a long pause (`LongPauseTest`);
- a song's lyrics following only that song's playhead (`ForSongTest`);
- release code reaching the test bridge only through `TestDriver` (`TestBridgeTest`);
- the perf self test's logic (`SelfTestLogicTest`);
- `InitOrderTest`: fails when a field a constructor's thread reads is declared after the line that
  starts that thread.

## Device checks

### Running them

```sh
tools/smoke.sh                                  # the real server in ~/.music.pass
NORI_E2E_SERVER=local tools/smoke.sh            # tools/dev-server.sh's Navidrome, through tools/lying-proxy.py
tools/audio-e2e.sh --list                       # the sections
tools/audio-e2e.sh --only tuning,restart        # just those
tools/feature-e2e.sh --only lyrics-services     # an opt-in section: never part of a full run
```

- All three take `--only <section>[,<section>]` and `--list`, exit non-zero on a failure, print each
  section's start time, and use `ANDROID_SERIAL` (default `emulator-5554`). `tools/device-check.sh`
  picks a free device for them ([Building](building.md#devices)).
- They need a debug build: they drive it with `tools/app.sh` (`open`, `play`, `set`, `do`, `state`),
  which reads only its own request's reply. `python3 tools/test-app.py` checks that without a device.
- `tools/e2e-lib.sh` has the waits: `wait_for <field> <value> <timeout>` polls `tools/app.sh state`
  (`!v` for anything else, `>n` and `<n` for numbers), `wait_until <timeout> <command>` waits for a
  command to succeed, and `sounds` / `silent` wait for the app's AudioTrack to start or stop. The only
  fixed waits are durations under test: 20 s paused in the background, and the ten-second burst cycle.
- Never play or stream an `ext-` item on the real server.

### The local server

`NORI_E2E_SERVER=local` runs against `tools/dev-server.sh`: a Navidrome on port 4533 (Docker or
Podman; one already answering on 4533 is used as is), with its music in the main checkout's `.dev/`,
shared by every worktree. Besides its generated albums it seeds:

| Fixture | For |
| --- | --- |
| Nori E2E / Long Album | twelve songs of 80-135 s, every third one FLAC; tracks 05 and 06 have timed LRC sidecars (the bridge, album pages, AutoMix, lyrics) |
| Nori E2E Two / Far Side | three songs of 150-190 s on another album, for crossfades between albums; "Far Song One" is the transcoded one |
| Nori Codecs / Lossless | one 20 s Apple Lossless song (M4A seeks, the web client's Rust decoders) |
| Nori Bench 1000 | a playlist of a thousand songs, for `tools/open-bench.sh` |

Each emulator signs in as its own `nori-e2e-<serial>` account, through `tools/lying-proxy.py` on port
4534 (`http://10.0.2.2:4534` from the emulator; 4536 for `emulator-5556`). The proxy:

- states a transcoded stream a quarter longer than its bytes, closes the connection short, and answers
  416 to a range past the real end, as Navidrome does when a transcode comes out smaller than its
  estimate;
- sends other songs at about 4 MB/s, so downloads can be seen running side by side;
- makes chosen songs hang, as octo-fiesta does for a provider song it cannot fetch:
  `NORI_HANG=id1,id2` (`NORI_HANG_MODE=headers|body`) at start, or
  `curl 'localhost:4534/_hang?ids=id1,id2&mode=body'` at any time; `curl localhost:4534/_hang` says how
  many requests hang now and how many the app closed.

The `remote` section resolves the terminal peer's live mDNS announcement with `dns-sd` on macOS or
Python's zeroconf on Linux. `python3 tools/test-mdns.py` checks that a stale announcement is rejected.

### What each section checks and why it needs a device

A new device check needs a line here saying why Rust cannot test it.

`tools/smoke.sh`:

| Section | Checks | Why a device |
| --- | --- | --- |
| `launch` | the app comes up and signs in to the run's server | the activity, the service |
| `play` | a song plays | a real AudioTrack |
| `controls` | pause, play, seek, next | media3's session over the real track |
| `queue` | a queue edit reaches the player | media3's timeline |
| `automix` | one AutoMix transition is heard and the next song plays out of it | the mix on a real track |
| `eq` | the equalizer on and off; the buffer is shallow while it is tuned and deep after, resized in place | `AudioTrack.setBufferSizeInFrames` |
| `offload` | offload on and off while playing | the platform's offload path |
| `notification` | the notification's play and pause | the media notification |
| `offline` | a downloaded song plays with the network off | media3's downloads and cache |
| `crashes` | no crash or ANR over the run | the platform's crash and ANR reports |

`tools/audio-e2e.sh`:

| Section | Checks | Why a device |
| --- | --- | --- |
| `play` | a song plays | a real AudioTrack |
| `transport` | media keys pause and resume; play resumes after 20 s paused in the background and with the screen off | media key routing, background limits, the screen |
| `coldkey` | a play key with the process gone resumes a saved queue; with none saved, no service starts | the system's media-button routing, media3's receiver, the foreground-service rule |
| `processing` | offload and the equalizer switched while playing | the platform's output |
| `crossfade` | the mix is heard on the device and the next song plays out of it | the real track |
| `eq` | the equalizer leaves without stopping the track | the real track |
| `tuning` | the buffer is shallow only while a sound page is in sight and tuned, resized in place, never reopened | `AudioTrack.setBufferSizeInFrames` and the app's lifecycle |
| `automix` | measuring starts when AutoMix is switched on, and the coming songs are measured out of the media3 cache | media3's cache read through JNI |
| `restart` | a seek while paused after a force stop sticks, and play resumes from it | a new service restoring the saved queue |
| `container-seek` | an M4A seek while paused resumes (local server) | Android's source and output after Rust's container seek |
| `transcode` | a transcode stated longer than it is plays past its real end (local server) | media3 and OkHttp on a real HTTP stack |
| `errors` | no playback errors over the run | the run's whole log |

`tools/feature-e2e.sh`:

| Section | Checks | Why a device |
| --- | --- | --- |
| `lyrics` | lyrics cross the binding; the next song's lyrics are cached ahead and arrive after an offline skip | the Android binding and the network going |
| `notification` | the notification's seal and shuffle act and redraw; its cover is drawn once and then served from disk | the media session, the cover provider's readers |
| `album-page` | the Play and Shuffle pills answer for the page's own queue; holding a song opens its menu | Compose taps and gestures |
| `bridge` | downloads stand in while the server is out of reach, and the album comes back with the network | the network really going and coming |
| `download-notification` | tapping the download notification opens the queue | the notification's intent |
| `downloads` | several songs download at once, under the notification's own id, and pick up after a force stop | media3's DownloadManager |
| `mobile-data` | a download on mobile data waits for a yes; on Wi-Fi it starts at once | ConnectivityManager |
| `car-art` | four-cover mosaics through the content provider, with one cover missing | content-provider pipes, Bitmap decoding, Canvas |
| `foryou` | favorites and mixes open as pages; starring and unstarring; a row plays | the screens |
| `dac` | with a faked USB DAC, offload stands down and the track opens in the DAC's format, bit-perfect | the platform's output devices |
| `device-sound` | an output's sound profile applies on connect, and the previous sound returns | the platform's output events |
| `sheets` | a sheet over the sideways player keeps the status bar away, before and after the shade | which window's status bar request Android obeys, and its insets |
| `queue-panel` | tapping the playing song in the queue returns to the player without restarting it | Compose taps and the panel switch |
| `remote` | the terminal client on this computer as the other device (local server): picking it, mirroring, volume, the system's output picker transferring back, a force stop rediscovering the peer, the peer's mix shown here | NsdManager, the remote media session (notification, volume keys), MediaRouter2 |
| `jam` | a jam hosted on the emulator against the octo-fiesta relay with two guests (`tools/jam-guest.py`), requests decided by tapping, ended and new invites; then the emulator as a guest of `tools/jam-host.py`: listening along with the screen off, a guest's pause, an admin's controls (`NORI_JAM_ADMINS=1`), and the guest timeline without a seek to zero | the playback service, the invite intent, media3's timeline |
| `plugins` | a repo served from this computer (orbit's package from `nori-plugins/dist`, `NORI_PLUGINS_DIST`) is added under its warning; orbit installs from it through the install and turn-on sheets, its stage joins and leaves the chain at once as it is turned on, off and removed (asked first), and its page shows its levels' values; music sounds throughout | the plugin runtime on the device's ABI, the Plugins page and its sheets, the repo fetched through OkHttp, the Android lifecycle opening the host, the stage reaching the AudioTrack player |
| `scrobble` (opt-in, local) | a qualifying listen counts once; an early skip and a pause don't | media3's track callbacks and the write queue |
| `download-albums` (opt-in, local) | downloaded albums on the phone and through Android Auto's media browser; emptying an open album page | media3's browser callbacks, Compose navigation, offline playback |
| `youtube-music` (opt-in) | the YouTube Music plugin (`nori-plugins/dist`) installed and turned on; its profile searches, plays, mixes, aligns lyrics, downloads and plays offline, shows in the car's browser and widgets, moves its queue to the terminal and back; Google's sign-in page opens and Cancel leaves no cookies | the resolved stream through media3 and OkHttp, widgets, the WebView and its CookieManager. Opt-in because YouTube can refuse on any day |
| `lyrics-services` (opt-in) | each lyrics service answers: a report, not a check | the services' health, not the app's |

`NORI_E2E_SERVER=local tools/library-sync-e2e.sh` checks that a favorite changed over the LAN reaches the
visible seal and the media session's buttons through JNI. It roots the emulator for a moment to find the
app's listener, then restores adb's previous state.

### Checked by hand

- **Updates**: which release and APK to offer is the core's (`update.rs`); the download and
  PackageInstaller are Android's. In a debug build, `tools/app.sh do "update 0.3.0"` pretends to be
  0.3.0 and shows the banner; its button opens the release page. To install in place, build an older
  release with `./gradlew :app:assembleRelease -PpretendVersion=0.3.9 -PrustTargets=x86_64`, install
  it, then About > Check for updates > Update.
- **The car**: the tree, search and voice requests are the core's (`car.rs`). A debug build walks them
  through a media browser with `tools/app.sh do "car root"`, `"car tree <id>"`, `"car search <words>"`,
  `"car play <id>"` and `"car voice <words>[|artist|album|playlist|genre|song]"`, logging each row under
  the tag `noricar`. `adb shell content read --uri content://dev.nori.music.carart/c/<cover id>` reads a
  picture. Android Auto itself is checked on a phone with the Desktop Head Unit.

## Desktop

Desktop UI regressions run on Slint's testing backend with virtual time (`cargo test -j4 -p
nori-desktop`): collections, shelves, covers released offscreen, menus, panels, animation only where
something changed. Playback and queue decisions stay in the Rust virtual-clock tests.

Rendering tests that need a GPU:

- macOS: `cargo test -j4 -p nori-desktop compositor::tests:: -- --nocapture` (Metal), also the
  "macOS rendering checks" workflow.
- Linux: `DISPLAY=:0 cargo test -j4 -p nori-desktop layers_resize_without_losing_pixels_or_blur --
  --ignored` checks every pixel through live resizes, repaints, cover ownership, mosaics and the blur.
  Run it on the native driver; VirtualGL can leave enlarged regions empty.

To drive a running desktop client, build it with the `test-control` feature (Linux and macOS). Without
`--control-socket` it starts no listener.

```sh
cargo build -j4 -p nori-desktop --features test-control
export NORI_DESKTOP_SOCKET=<private test directory>/control.sock
target/debug/nori-desktop --data <test data directory> --control-socket "$NORI_DESKTOP_SOCKET"
python3 tools/desktop-app.py state
python3 tools/desktop-app.py open fullscreen
python3 tools/desktop-app.py panel queue
python3 tools/desktop-app.py do pause
```

| Command | Takes |
| --- | --- |
| `open` | `home`, `search`, `albums`, `artists`, `playlists`, `songs`, `settings`, `equalizer`, `fullscreen` |
| `panel` | `none`, `queue`, `lyrics`, `devices` |
| `do` | `play`, `pause`, `toggle`, `next`, `previous`, `close_fullscreen` |

Each instance needs its own socket and data directory; an existing socket is never replaced. The
socket is owner-only (0600) and removed on a normal exit. The reply is the UI's actual state; playback
requests finish asynchronously, so wait for the state to change before asserting. Run unattended
checks in Xvfb with a private audio sink. On Linux, check interactive window resizing under a window
manager on a 1920 × 1080 Xvfb screen. CPU, heap and GPU measurements are in
[Desktop performance](desktop-performance.md).

## The web client

What the web client plays is the engine's and the session's, tested in Rust: pushed bytes
(`pushed.rs`, `driven.rs`), decoders that answer later as WebCodecs does (`later.rs`), and
`crates/host`. The browser checks cover only what Rust cannot reach: the worker, the AudioWorklet, the
browser's decoders, OPFS, streaming `fetch` and the pages.

```sh
npm --prefix web/check install
python3 web/serve.py                                     # with tools/dev-server.sh running
node web/check/check.mjs http://localhost:8080/          # every phase
node web/check/check.mjs http://localhost:8080/ play,seekbar
```

The phases: `browse`, `play`, `lyrics`, `skip`, `instant`, `automix`, `crossfade`, `codecs`, `speed`,
`seekbar`, `edit`, `crash`, `tabs`, `offline`, `early`, `slowstart`. They drive headless Chrome (`CHROME`
names another browser): sign-in, browsing and search, playback without the AudioWorklet running dry,
next and previous, lyrics and a tapped line heard from where it is sung, pause silent within 50 ms, a
seek heard from its place within 100 ms, the seek bar, an AutoMix transition and a crossfade with no
silence over 20 ms, an Apple Lossless song through the Rust decoders.
`THROTTLED_URL=http://localhost:8082/`, with `python3 web/serve.py --port 8082 --kbps 400` running,
also times a song's first sound at 400 kbit/s; `SLOW_URL` adds a Play during a slow load.

Headless Chrome is muted: the checks read what the AudioWorklet played. They cannot check audio with
a locked screen, Safari's or Firefox's codecs, or real latency.

With the relay (`tools/jam-server.sh`, then `python3 web/serve.py --port 8081 --server
http://localhost:5274`):

```sh
node web/check/jam.mjs http://localhost:8081/                 # Play On and a jam between two browser profiles
node web/check/shots.mjs http://localhost:8081/ <directory>   # screenshots of both layouts in Day, Night, Midnight and two seasons
```

## The iPod

Rust covers `IosOutput` on a simulated sink (the granted format, the latency sum, shallow switching, a
route change, a failed reopen, no allocation on the render path), the session behind the C ABI on
`WavOutput` (two downloaded songs through the controls, queue edits, the save on background, a memory
warning), remote control on the virtual clock, and the doors the screens read (`cargo test -p
nori-ios`).

On the device:

- AURemoteIO itself: a sine from the engine's test tone through the jack, with no underrun for a minute
  at 93 ms, at 10 ms, and across a Bluetooth route change. Rust cannot see the audio unit.
- Bonjour announcing and finding, the render timestamps' host time, the volume view and the lock screen
  while another device plays: iOS's.
- UIKit's drawing (the card and queue sliding, the word fill with CoreText, a dragged queue row), looked
  at by eye.

`tools/ipod.sh run` shows the process and its log. Battery and CPU are `tools/ipod-bench.sh`
(unplugged, SSH over Wi-Fi), never the host.

## Tooling checks

| Command | Checks |
| --- | --- |
| `python3 tools/test-resources.py` | the shared queue: exclusion, nesting, device pools and cancellation |
| `python3 tools/test-app.py` | `tools/app.sh`'s reply matching and shell quoting |
| `python3 tools/test-mdns.py` | the remote check takes a live peer, not a cached announcement |
| `python3 tools/test-release.py` | a release handoff prepares without a compiler or signing key, needs every selected platform, and fails on a changed artifact |
| `python3 -m unittest discover -s tools/bazel -p 'test_*.py'` | the Bazel BUILD generator |
| `actionlint .github/workflows/*.yml` | the workflow definitions |
| `python3 tools/host-release/check.py <output>` | on macOS: the signatures of Mac packages built on Linux, and that they run (`--checksums-only` skips running them) |

Workspace tests encode their fixtures with FFmpeg 8 or newer. FFmpeg 4 writes different AAC edit lists
and cannot make the gapless fixtures.
