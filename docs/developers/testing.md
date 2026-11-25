# Testing

This page shows which tests to run for a change, how to run them, and how to write new ones. Run every
command from the repository's root. To install the tools, see [Building](building.md).

Rust holds every decision, and the Rust tests cover it, on a virtual clock where time matters. Device
checks cover only what Rust cannot reach, such as the audio output, media3, the media session, JNI
and the screens.

## Which tests to run

Run the tiers that match your change:

| Tier | Covers | Command | When to run |
| --- | --- | --- | --- |
| Rust tests | Every Rust test, doctests included | `cargo test -j4 --workspace` | Every change |
| Clippy | Every warning, as CI's errors | `cargo clippy -j4 --workspace --all-targets -- -D warnings` | Every Rust change |
| Web and iPod clippy | What a workspace build misses | `tools/check.sh web`, `tools/check.sh ipod` | Changes to core, host, engine or remote |
| Kotlin unit tests | The Kotlin logic, with no device | `./gradlew :core:testDebugUnitTest :app:testDebugUnitTest` | Kotlin changes |
| Smoke check | The Android glue, in about two minutes | `tools/smoke.sh` | Every change that reaches Android |
| Touched sections | The device sections a change touches | `tools/audio-e2e.sh --only …`, `tools/feature-e2e.sh --only …` | Before you commit an Android change |
| Full device suites | Every device check | `tools/audio-e2e.sh`, `tools/feature-e2e.sh` | Once per batch of changes |
| Web browser checks | The worker, the AudioWorklet, the pages | `node web/check/check.mjs` | Web client changes |

### What CI runs

On a push, CI runs only the jobs the push can affect. For the Rust tests, `tools/affected.py` picks the
crates you changed and the crates that depend on them. A change to `Cargo.*`, `vendor/`, `wit/`,
`.cargo/`, `.config/` or `.github/` runs every test. The Android job compiles the Kotlin and runs its unit
tests. It does not run device checks.

## Run the Rust tests

```sh
cargo test -j4 --workspace                   # everything
cargo test -j4 -p nori-engine                # one crate
cargo test -j4 -p nori-player --test pipeline   # the player end to end on a virtual clock
tools/check.sh rust --workspace              # nextest, through the shared queue
tools/check.sh rust-doc --workspace          # the doctests, which nextest skips
```

The tests need two things that are easy to miss:

- FFmpeg 8 or newer on the `PATH`, to encode the engine's fixtures. Without FFmpeg, those tests print
  "ffmpeg is not installed: skipped" and pass. FFmpeg 4 cannot make the gapless fixtures.
- The `wasm32-wasip2` target, for the plugin host's tests: `rustup target add wasm32-wasip2`. Without
  it, a test fails with "the fixture builds (rustup target add wasm32-wasip2)".

### Where the harnesses are

- `crates/player/tests/pipeline` runs the whole player on a simulated output.
- `crates/engine/tests` runs the player on its own thread, on the virtual clock in `tests/common`.
- `crates/core/tests/scenario.rs` runs whole flows against an in-memory server. `tests/remote.rs` runs
  remote control between paired devices through an in-memory sync server (`nori_sync::hub`), and jams
  over an in-memory relay.
- `crates/android/src/track.rs` tests the AudioTrack writer on a simulated track, and `lib.rs` checks
  every JNI door against its Kotlin `external fun`.
- nori-cli and nori-desktop test their screens with ratatui's `TestBackend` and Slint's testing backend.
- nori-testdir checks that every test file is in a test binary, and that AGENTS.md's Layout table
  matches the crates.

These files in `crates/engine/tests` cover the engine's harder cases:

| File | What it checks |
| --- | --- |
| `paths.rs` | Offload on a simulated audio chip |
| `stretch.rs` | Stretched mixes |
| `estimated.rs`, `transcode.rs` | Estimated transcode lengths and their 416 answers |
| `hung.rs` | Streams that hang |
| `silent.rs` | The engine never plays silence instead of music |
| `one_fetch.rs` | One fetch per song |
| `replan.rs` | Settings changed while a song plays |
| `album.rs` | Albums stay gapless |
| `along.rs` | Jam guests listening along |
| `pushed.rs`, `driven.rs`, `later.rs` | Pushed and late-decoded songs, for the web client |
| `decoders.rs` | FLAC, ALAC and Opus decoding, with a digest to compare a change against |
| `core.rs` | The core's queue |

### Kotlin twins

Some Rust code is a port of Kotlin code, and the two must give the same answers. `tools/twins.sh` runs
the Kotlin originals in `crates/*/testdata/twins/*.kt` on the JVM and writes their answers beside them.
`crates/core/tests/twins.rs` holds the Rust versions to those answers. Run `tools/twins.sh` again after
you change either side.

## Write a Rust test

AGENTS.md has the rules for tests. These are the mechanics that trip people up.

### Register a new test file

- `crates/engine` and `crates/core` set `autotests = false`. A new test file needs a `[[test]]` entry in
  `Cargo.toml`, or a `#[path]` module in `tests/main.rs`.
- A new file in `crates/player/tests/pipeline` needs a `mod` line in its `main.rs`.
- If you forget, nori-testdir's `every_test_file_is_registered` test fails and names the file.

### Keep time on the virtual clock

Engine tests run on a clock that only the test moves:

- Move time with the rig: `run`, `wait_for` and `until`. Never call `thread::sleep`.
- Anything that wakes the engine outside a command goes through `Virtual::woke_engine`.
- Assert on what the test card heard, not on the engine's status.
- The clock stands still while the engine waits for bytes, up to `BYTES_WAIT` (2 s of real time)
  without progress. An in-memory server acts as an instant network, however the threads are scheduled.
- A server paced on the clock stops it at each time it waits for (`Virtual::wait_until`).

### Keep the suite fast

- Make each test song once per binary, cached by length and seed, as `music()` in `engine.rs` does.
- Give a table of slow cases a thread per case (`each` in `automix/tests.rs`), or make one test per row.
- Tests build nori-player, nori-engine, nori-android, nori-ios and every dependency at opt-level 2.
  Other workspace crates, nori-core among them, are unoptimized, so keep their test data small.
- Process-wide state (the core's queue, the active client, the AutoEQ fetch, the watch hook) needs its
  own test binary. Otherwise, the test must filter to what it made itself.

### Change the stored settings format

`crates/settings/tests/stored_format.rs` fails when a stored key or encoding changes. If you meant the
change, run the test with `NORI_BLESS=1` to write a new golden copy.

### Read a failure

- Asserts print the expected and actual values, song ids, positions, and the engine's event log
  (`rig.events`) or the planner's log. Read the last event first. An `Error`, `Stopped` or `Bridge` there
  is the cause.
- "the engine did not go to sleep" means the engine thread is stuck or spinning, not slow. The limit is
  120 s of real time.
- A virtual-clock test that fails only under load has a real thread that the clock does not see. Make
  that thread wait for a condition, not for time.
- To find such a thread, run the test binary with `--exact NAME` in sixteen parallel loops and compare
  the logs. On the virtual clock, every run prints the same.

### Compare against a reference render

- `crates/engine/tests/common/reference.rs` renders offline what the card should hear when the sound
  changes mid-play, switching at the frame the engine logged. The `*_changes_seamlessly*` tests compare
  the card with it, sample for sample.
- `reference::clicks` finds jumps where no exact render exists.
- `crates/android/src/track/air.rs` models the Android mixer, with Bluetooth's late and jittery
  timestamps. It checks that every frame is heard once, with no gap, level step or click.

## Run the Kotlin unit tests

```sh
./gradlew :core:testDebugUnitTest :app:testDebugUnitTest
```

They need no device, and cover what the Kotlin keeps: queue edits, the player's cover, the stream
bridge, formatting and the screens' state. `InitOrderTest` checks that no field a constructor's thread
reads comes after the thread starts.

## Run the device checks

The device checks drive a debug build on an emulator or a phone. You need:

- A debug build installed, as [Building](building.md) shows. The checks drive it with `tools/app.sh`.
- A device. The scripts use `ANDROID_SERIAL`, and `emulator-5554` by default.
- A server: the local one, or a real one listed in `~/.music.pass`.

To run the smoke check against the local server:

```sh
NORI_E2E_SERVER=local tools/smoke.sh
```

The script starts the local server and its proxy when they are not running. It prints a PASS or FAIL
line per check and exits non-zero on a failure. A failed check saves the accessibility tree and a
screenshot under `build/e2e/<serial>/failures/`.

### Choose the sections

`tools/smoke.sh`, `tools/audio-e2e.sh` and `tools/feature-e2e.sh` take the same options:

```sh
tools/audio-e2e.sh --list                       # the sections
tools/audio-e2e.sh --only tuning,restart        # only these
tools/feature-e2e.sh --only lyrics-services     # an opt-in section, never part of a full run
```

### Use the real server

Without `NORI_E2E_SERVER=local`, the checks use the server in `~/.music.pass`. Its lines are the URL, a
blank line, the user name and the password. Never play or stream a provider (`ext-`) song on the real
server: the request makes the server download it.

### Run checks on several emulators

`tools/device-check.sh` leases the first free device in `NORI_E2E_DEVICES` for the install, the checks
and the cleanup. A job waits when every device is taken. Start the emulators first, as
[Building](building.md) shows.

```sh
export NORI_E2E_SERVER=local
export NORI_E2E_DEVICES=emulator-5554,emulator-5556
tools/device-check.sh --install android/app/build/outputs/apk/debug/app-debug.apk smoke
tools/device-check.sh audio --only tuning,restart
```

- `smoke.sh`, `audio-e2e.sh` and `feature-e2e.sh`, run directly, lease their `ANDROID_SERIAL`.
- Do not drive a leased device with `tools/app.sh` or adb from elsewhere.
- Each emulator gets its own Navidrome account (`nori-e2e-<serial>`), proxy port, terminal peer and logs.
  The music library is shared.
- `emulator-5554` uses proxy port 4534 and `emulator-5556` uses 4536. For other serials,
  `NORI_E2E_PROXY_PORT` sets the port.

### Helpers for device checks

- `tools/app.sh` drives the debug build: `launch`, `open`, `play`, `set`, `do` and `state`.
- `tools/e2e-lib.sh` has the waits. `wait_for <field> <value> <timeout>` polls `tools/app.sh state`.
  The value `!v` waits for anything else, and `>n` and `<n` compare numbers.
- `wait_until <timeout> <command>` waits for a command to succeed. `sounds` and `silent` wait for the
  app's AudioTrack to start or stop.
- The only fixed waits are durations under test, such as 20 s paused in the background. Never add a
  fixed sleep.
- Press media keys with `media_key` and turn the screen off with `screen_off`. With the screen off,
  `media_key` sends the key to the media session, as a headset's key arrives. The system drops a key
  from `input keyevent` then, because no window has the focus.
- Fill a text field with `fill_field` (`tools/feature-e2e.sh`) after `field_focused` passes. nori's text
  fields drop some keys when `input keyevent` sends several at once.
- Inside a time limit, such as the 5 s of an Undo, act first and check after. One reading of the
  accessibility tree takes about a second.
- `tools/ui-dump.sh` reads the visible accessibility tree. It does not wait for an animating player to
  go idle.
- The jam checks need an octo-fiesta relay on `localhost:5274`. `tools/jam-server.sh` starts one from the
  octo-fiesta checkout beside this one, or the one `NORI_OCTO_ROOT` names.
- The `remote` and `youtube-music` sections run the terminal app as the other device, in tmux.

## The local server

`NORI_E2E_SERVER=local` runs the checks against `tools/dev-server.sh`. It is a Navidrome on port 4533,
with the user `admin` and the password `admin`.

- It runs in Docker or Podman. A Navidrome already answering on port 4533 is used as it is.
- It makes its music with FFmpeg on its first run, in the main checkout's `.dev/`, shared by every
  worktree. `NORI_DEV_ROOT` names another folder.
- The emulator reaches it at `http://10.0.2.2:4533`.

Besides its generated albums, it seeds these fixtures:

| Fixture | What it holds | What uses it |
| --- | --- | --- |
| Nori E2E / Long Album | Twelve songs of 80-135 s, every third one FLAC | The bridge, album pages, AutoMix, lyrics |
| Nori E2E Two / Far Side | Three songs of 150-190 s, "Far Song One" transcoded | Crossfades between albums |
| Nori Codecs / Lossless | One 20 s Apple Lossless song | M4A seeks, the web client's Rust decoders |
| Taylor Swift / Midnights (Deluxe) | One generated song on an album with a moving cover | `web/check/plugins.mjs` |
| Pale Ink / Paper Sleeve | One song under a near-white cover | The `status-bar` section |
| Nori Bench 1000 | A playlist of a thousand songs | `tools/open-bench.sh` |

Tracks 05 and 06 of the Long Album have timed LRC lyrics beside them.

### The lying proxy

Each emulator signs in as its own `nori-e2e-<serial>` account, through `tools/lying-proxy.py`, on port
4534 for `emulator-5554`. The proxy acts as a real server does in its worst cases:

- It states a transcoded stream a quarter longer than its bytes, then closes the connection short.
- It answers 416 to a range past the real end. Navidrome does the same when a transcode comes out
  smaller than its estimate.
- It sends other songs at about 4 MB/s, so you can see downloads run side by side.
- It can make chosen songs hang, as octo-fiesta does for a provider song it cannot fetch.

To make songs hang, set `NORI_HANG=id1,id2` and `NORI_HANG_MODE=headers|body` when the proxy starts. To
change them while it runs, call it:

```sh
curl 'localhost:4534/_hang?ids=id1,id2&mode=body'   # these songs hang from now on
curl localhost:4534/_hang                           # how many requests hang now, and how many the app closed
```

## What each device check covers

Every device check needs a reason why Rust cannot test it. When you add a section, add its row here
with that reason.

### `tools/smoke.sh`

| Section | Checks | Why it needs a device |
| --- | --- | --- |
| `launch` | The app comes up and signs in | The activity, the service |
| `play` | A song plays | A real AudioTrack |
| `controls` | Pause, play, seek, next | media3's session over the real track |
| `queue` | A queue edit reaches the player | media3's timeline |
| `automix` | One AutoMix transition plays, and the next song plays out of it | The mix on a real track |
| `eq` | The equalizer on and off, with the buffer resized in place | `AudioTrack.setBufferSizeInFrames` |
| `offload` | Offload on and off while playing | The platform's offload path |
| `notification` | The notification's play and pause | The media notification |
| `offline` | A downloaded song plays with the network off | media3's downloads and cache |
| `crashes` | No crash or ANR during the run | The platform's crash and ANR reports |

### `tools/audio-e2e.sh`

| Section | Checks | Why it needs a device |
| --- | --- | --- |
| `play` | A song plays | A real AudioTrack |
| `transport` | Media keys pause and resume, also in the background and with the screen off | Media key routing, background limits |
| `coldkey` | A play key with no process resumes a saved queue | The system's media-button routing |
| `processing` | Offload and the equalizer switched while playing | The platform's output |
| `crossfade` | The mix plays, and the next song plays out of it | The real track |
| `eq` | Leaving the equalizer does not stop the track | The real track |
| `tuning` | The buffer is shallow only while a sound page is open and tuned | The app's lifecycle |
| `automix` | Measuring starts with AutoMix, from the media3 cache | media3's cache read through JNI |
| `restart` | After a force stop, the queue, its order and the place come back | A new service restoring the queue |
| `container-seek` | An M4A seek while paused resumes (local server) | Android's source after Rust's seek |
| `transcode` | A transcode stated too long plays past its real end (local server) | media3 and OkHttp on real HTTP |
| `errors` | No playback errors during the run | The run's whole log |

### `tools/feature-e2e.sh`

| Section | Checks | Why it needs a device |
| --- | --- | --- |
| `plugins` | A repo's plugin installs, its stage joins the chain, its slots show | The plugin runtime on the device's ABI |
| `lyrics` | Lyrics cross the binding, and the next song's arrive after an offline skip | The binding, the network going |
| `status-bar` | The status bar's icons after a theme change (local server) | Only the system reports the window state |
| `notification` | The session's seal and shuffle act and redraw | The media session, the cover provider |
| `album-page` | The Play and Shuffle pills, and holding a song for its menu | Compose taps and gestures |
| `bridge` | Downloads stand in while the server is out of reach | The network going and coming |
| `download-notification` | Tapping the download notification opens the queue | The notification's intent |
| `downloads` | Several downloads at once, resumed after a force stop | media3's DownloadManager |
| `mobile-data` | A download on mobile data waits for a yes | ConnectivityManager |
| `car-art` | Four-cover mosaics through the content provider | Content-provider pipes, Bitmap, Canvas |
| `foryou` | Favorites and mixes open, star and play | The screens |
| `dac` | With a faked USB DAC, the track opens bit-perfect, without offload | The platform's output devices |
| `device-sound` | An output's sound profile applies on connect | The platform's output events |
| `sheets` | A sheet over the sideways player keeps the status bar away | Android's status bar rules and insets |
| `queue-panel` | Tapping, scrolling, Clear and Undo in the queue panel | Compose, the panel, media3's list |
| `sync` | Your devices and Pair a device, both ways round, Sync this profile, removing, Reset pairing, the server going | The screens, OkHttp's WebSockets through `adb reverse` |
| `remote` | The terminal app as the other device, through every system surface | NsdManager, the remote media session, MediaRouter2 |
| `jam` | Hosting a jam with two guests, then joining one as a guest | The service, the invite intent, media3 |
| `scrobble` (opt-in, local) | A qualifying listen counts once, and an early skip does not | media3's track callbacks |
| `download-albums` (opt-in, local) | Downloaded albums on the phone and in Android Auto's browser | media3's browser callbacks |
| `youtube-music` (opt-in) | The YouTube Music plugin's profile, end to end | media3, OkHttp, the WebView |
| `lyrics-services` (opt-in) | Whether each lyrics service answers | The services' health, not the app's |

Some sections need more than the emulator:

- `plugins` serves a plugin repo from this computer. It takes orbit and the showcase from
  `nori-dev-plugins/dist` beside this checkout (or `NORI_DEV_PLUGINS_DIST`), and the official plugins
  from `nori-plugins/dist` (or `NORI_PLUGINS_DIST`).
- `sync` runs the self-hosted sync server from the `nori-sync` checkout beside this one (or
  `NORI_SYNC_ROOT`), and `nori-cli --sync` as the other device. It starts the phone in no group by
  removing the group from the stopped app's database.
- `remote` resolves the terminal app's live mDNS announcement. It uses `dns-sd` on macOS and Python's
  zeroconf on Linux. `python3 tools/test-mdns.py` checks that a stale announcement is rejected.
- `jam` needs the relay from `tools/jam-server.sh`. Its guests are `tools/jam-guest.py`, and its host is
  `tools/jam-host.py`. `NORI_JAM_ADMINS=1` makes every guest an admin.
- `youtube-music` is opt-in because YouTube can refuse on any day.

`NORI_E2E_SERVER=local tools/library-sync-e2e.sh` checks that a favorite changed over the LAN reaches the
visible seal and the media session's buttons through JNI. It roots the emulator for a moment to find the
app's listener, then restores adb's previous state.

### The sync server check

`tools/sync-check.sh` checks pairing and profile sync against a real sync server. It builds and starts the
self-hosted program of the neighbouring `nori-sync` checkout (`NORI_SYNC_ROOT` names another) on a free
local port, with two members to a group room. It then drives headless `nori-cli --sync` instances:

- Two devices pair, see each other online, and sync a profile both ways.
- The server stops. Both wait quietly, then come back by themselves when it starts again.
- A third device pairs and shows the full group room as a refusal (`Refused { refused: Limit, .. }`).

`tools/sync-check.sh --android SERIAL` then pairs the app's debug build on that device into the group, through
`tools/app.sh sync`, and syncs a profile with it. `adb reverse` gives the device the server's port, so
both sides reach it at one address, which the pairing code carries. The Rust tests cover the protocol, reconnecting and the keep-alive rules. This check covers
what only a real server and real sockets show.

### Checked by hand

- **Updates.** The core decides which release to offer (`update.rs`), and Android downloads and installs
  it. In a debug build, `tools/app.sh do "update 0.3.0"` pretends to be 0.3.0 and shows the banner. To
  install in place, build an older release with
  `./gradlew :app:assembleRelease -PpretendVersion=0.3.9 -PrustTargets=x86_64` and install it. Then tap
  **Check for updates** and **Update**.
- **The car.** The core builds the tree, search and voice answers (`car.rs`). A debug build walks them
  with `tools/app.sh do "car root"`, `"car tree <id>"`, `"car search <words>"`, `"car play <id>"` and
  `"car voice <words>[|artist|album|playlist|genre|song]"`, and logs each row under the tag `noricar`.
  Check Android Auto itself on a phone with the Desktop Head Unit.
- **A TV.** Use an Android TV emulator (a `tv` system image, 1080p). The key events 19 to 23 (up,
  down, left, right, center) walk the remote's focus: `adb shell input keyevent 19`. Leaving a text field
  with the D-pad needs a real remote.

## Test the desktop app

The desktop UI tests run on Slint's testing backend, with virtual time:

```sh
cargo test -j4 -p nori-desktop
```

The rendering tests need a GPU:

```sh
cargo test -j4 -p nori-desktop compositor::tests:: -- --nocapture                         # macOS, Metal
DISPLAY=:0 cargo test -j4 -p nori-desktop layers_resize_without_losing_pixels_or_blur -- --ignored  # Linux
```

On Linux, run it on the native driver, because VirtualGL can leave enlarged regions empty.

### Drive the running desktop app

To drive a running desktop app from a script, build it with the `test-control` feature (Linux and macOS):

```sh
cargo build -j4 -p nori-desktop --features test-control
export NORI_DESKTOP_SOCKET=<private test folder>/control.sock
target/debug/nori-desktop --data <test data folder> --control-socket "$NORI_DESKTOP_SOCKET"
python3 tools/desktop-app.py state
python3 tools/desktop-app.py open fullscreen
python3 tools/desktop-app.py panel queue
python3 tools/desktop-app.py do pause
```

`tools/desktop-app.py` takes these commands:

| Command | Takes |
| --- | --- |
| `open` | `home`, `search`, `albums`, `artists`, `playlists`, `songs`, `settings`, `equalizer`, `fullscreen` |
| `panel` | `none`, `queue`, `lyrics`, `devices` |
| `do` | `play`, `pause`, `toggle`, `next`, `previous`, `close_fullscreen` |
| `play_album` | An album's id |
| `tab` | A Settings tab's index |
| `shot` | A path to save the window to, as a PNG |
| `profiles` | Nothing, and opens the profiles menu |
| `welcome_pick` | The index of a row on the welcome page shown, and picks that source or opens its sign-in |
| `welcome_continue` | Nothing, and presses the welcome's Continue |
| `login` | A server URL, user and password, and fills the sign-in form shown and sends it |
| `setup_answer` | The index of an answer to the first-run question shown, and picks it |
| `setup_back` | Nothing, and goes to the first-run question before |
| `appearance` | 0 System, 1 Day, 2 Night or 3 Midnight |
| `click` | An x and y in the window's logical pixels, and presses and releases the left button there |
| `state` | Nothing, and prints the UI's state |
| `sync` | `state`, `code`, `take <link>`, `confirm [keep]`, `cancel` or `reset`, and prints sync's state, the pairing and the devices |

- Each instance needs its own socket and data folder. The app never replaces an existing socket.
- Playback requests finish after the reply, so wait for the state to change before you assert.
- Run unattended checks in Xvfb, with a private audio sink, so the owner's desktop and audio stay free.

To measure the desktop app's CPU, heap and GPU, see [Desktop profiling](desktop-performance.md).

## Test the web client

Rust tests what the web client plays (`pushed.rs`, `driven.rs` and `later.rs` in the engine's tests).
The browser checks cover only what Rust cannot reach: the worker, the AudioWorklet, the browser's
decoders, OPFS, streaming `fetch` and the pages.

```sh
npm --prefix web/check install
tools/dev-server.sh
python3 web/serve.py
node web/check/check.mjs http://localhost:8080/                  # every phase
node web/check/check.mjs http://localhost:8080/ play,seekbar     # only these phases
```

The checks drive headless Chrome, at Google Chrome's macOS path by default. `CHROME` names another
browser. The phases are `browse`, `profile`, `play`, `recolor`, `lyrics`, `skip`, `queue`, `instant`,
`automix`, `crossfade`, `codecs`, `seekbar`, `edit`, `crash`, `reopen`, `tabs`, `offline`, `early` and
`slowstart`.

Two variables add phases on a slow network:

- `THROTTLED_URL=http://localhost:8082/`, with `python3 web/serve.py --port 8082 --kbps 400` running,
  times a song's first sound at 400 kbit/s (`slowstart`).
- `SLOW_URL` adds a Play during a slow load.

Headless Chrome is muted, so the checks read what the AudioWorklet played. They cannot check audio with a
locked screen, Safari's or Firefox's codecs, or real latency.

Two more scripts need the relay. Start it, then serve the client through it:

```sh
tools/jam-server.sh
python3 web/serve.py --port 8081 --server http://localhost:5274
node web/check/jam.mjs http://localhost:8081/                 # Play On and a jam between two browser profiles
node web/check/shots.mjs http://localhost:8081/ <folder>      # screenshots of both layouts in Day, Night, Midnight and two seasons
```

`node web/check/plugins.mjs <url>` checks the plugin host in the browser. Its default address is
`http://localhost:8099/`.

## Test the iPod app

`cargo test -p nori-ios` tests `IosOutput` on a simulated sink, and the session behind the C ABI on
`WavOutput`. Check these on the device:

- AURemoteIO itself. Play the engine's test tone, a sine, through the jack for a minute. It must not
  underrun at 93 ms, at 10 ms, or across a Bluetooth route change.
- Bonjour announcing and finding, the render timestamps' host time, the volume view, and the lock screen
  while another device plays.
- UIKit's drawing: the card and queue sliding, the word fill with CoreText, a dragged queue row. Look at
  these by eye.

`tools/ipod.sh run` shows the process and its log. Measure battery and CPU with `tools/ipod-bench.sh`,
unplugged, over SSH on Wi-Fi. [iPod internals](ipod-internals.md) has the setup.

## Test the tooling

| Command | Checks |
| --- | --- |
| `python3 tools/test-resources.py` | The shared queue: exclusion, nesting, device pools and cancellation |
| `python3 tools/test-app.py` | `tools/app.sh`'s reply matching and shell quoting |
| `python3 tools/test-mdns.py` | The remote check takes a live peer, not a cached announcement |
| `python3 tools/test-release.py` | A release handoff works without a compiler or key, and fails on changed artifacts |
| `python3 -m unittest discover -s tools/bazel -p 'test_*.py'` | The Bazel BUILD generator |
| `actionlint .github/workflows/*.yml` | The workflow files |
| `python3 tools/host-release/check.py <output>` | On macOS, the signatures of Mac packages built on Linux, and that they run |

`tools/host-release/check.py --checksums-only` checks the files without running them.
