# Plugins

Applies to nori 0.7, the Android, desktop and terminal clients. The iPod and the web have no plugins.

A plugin is a WebAssembly component built against the `nori:plugin` API in `wit/`. It runs sandboxed in
the plugin host (`crates/plugins`): it reaches only what the host links for it, only the hosts its
manifest names, and only the nori values its manifest says it overrides. The plan, with what is not
built yet, is `docs/research/plugins.md`.

| piece | where |
|---|---|
| the API (WIT) | `wit/`: `types.wit`, `host.wit` and `net.wit` (what a plugin calls), `plugin.wit` (what it exports, and the `plugin` world) |
| the SDK | `crates/plugin-sdk` |
| the host | `crates/plugins` |
| plugins' sound stages in the chain | `crates/player/src/stage.rs` |
| overrides laid over the settings | `crates/settings/src/overrides.rs` |
| plugins' providers | `crates/core/src/providers/plugin.rs`, through `Kinds` |
| nori's own plugins | [nori-plugins](https://github.com/norifm/nori-plugins): orbit (a sound stage), playback (speed and pitch as overrides), youtube-music (a provider) |

## Writing one

```rust
use nori_plugin_sdk::{state, Manifest, Row, Slot, SoundStage};

#[derive(Default)]
struct Orbit { /* ... */ }

impl SoundStage for Orbit {
    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> { /* ... */ samples }
    // configure, reset, save, restore ...
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    rows: &[Row::level("speed", 0.02, 1.0, 0.01, 0.12)],
    ..Manifest::new("fm.nori.orbit")
};

nori_plugin_sdk::plugin!(Orbit, MANIFEST, sound_stage);
```

- Each exported interface of the `plugin` world is a trait of the SDK, written from `wit/` by its
  build.rs. Every method answers as nori would on its own: `None` (nori's answer stands), "unsupported",
  empty, or, for a call handed what it answers (a stage's samples), what it was handed. An export
  answering a record nori can't make up is an `option<...>` in the WIT; build.rs refuses one that isn't.
- `plugin!(Type, MANIFEST, points...)` exports `Type` for the points named; the others answer as nori.
- The manifest is Rust. `plugin!` encodes it at compile time into the wasm's `nori:plugin` custom
  section, with the crate's version and the points named, and nori reads it before running anything.
  What the plugin calls (HTTP, settings) isn't declared: nori reads it from the component's imports.
  The manifest says only what code can't: its id, the hosts and Bluetooth devices it may reach, the
  keys it overrides, its rows, a stage's slot, and its permissions, hooks, wakes, commands, cache quota
  and assets.
- Words are files: `strings/<lang>.toml`, key = text, English required: `name`, `about`, each row's id
  and each choice option's key.
- A plugin depends on the SDK from this repository, `nori-plugin-sdk = { git = "https://github.com/norifm/nori" }`,
  its commit pinned by the plugin's `Cargo.lock`.
- Build with `cargo build --release --target wasm32-wasip2`; a `.noriplugin` is a zip of `plugin.wasm`,
  `strings/` and an optional `icon.svg` (`tools/pack.py` in nori-plugins). A plugin's `cdylib` doesn't
  link for the host target (its export names are wasm's); its tests do.

## The host

- **Nothing without a plugin.** With none enabled, opening the host reads `plugins/index.json` and each
  installed plugin's `manifest.bin` (its manifest, kept at install; the wasm isn't read): no engine, no
  compiling, no thread. The settings' layer is empty, so the engine's settings are nori's own. The
  engine's perf bench measures the same wakes, CPU and allocations as without plugins.
- **One engine, compiled once.** wasmtime with Cranelift, made at the first plugin needed. Each plugin is
  compiled at install and kept as `compiled.cwasm` beside it for this engine (a newer nori compiles it
  again); never downloaded as native code. The compiler is built for size (it runs only at install),
  wasmtime's own calls for speed.
- **Install checks first.** A package is unpacked beside the old version and refused when its manifest
  can't be read, its API version is not this host's or the one before, its id isn't reverse DNS, it
  overrides a key nori doesn't have, or it imports calls of an interface this host doesn't link
  (`runtime::LINKED`). Its store survives an
  update.
- **Its own thread.** A plugin's own instance (lifecycle, hooks, provider, slots) runs on a thread made
  when it is turned on (with a lifecycle) or first called ([`Worker`](../../crates/plugins/src/worker.rs)):
  its calls may block (its HTTP waits on the core's transport there), and the core's executors never
  wait on one. A call whose caller has gone (a search typed over) is skipped. Each call has a fuel
  budget; a trap or a spent budget fails that call, and the next starts a fresh instance (started again).
- **Requests side by side.** `http.fetch` waits for its answer; `http.start`, `wait` and `take` put up to
  16 requests on their way at once, each on a thread of its own. The SDK's `fetching::send` and
  `fetching::run` drive them from async code, so a `join_all` of requests takes as long as the slowest.
- **WASI, the little std needs.** A Rust plugin's std imports WASI; the host answers randomness, the
  clocks and an empty environment, at whatever versions the plugin imports, and links the rest (files,
  sockets, stdio) to traps. A plugin writes through `log`.
- **Its own store**, capped (1 MB): its keys, and under `nori/` what the host keeps for it (its rows'
  values, its overrides), which it can't write.
- **HTTP** goes through the open profile's `Transport` (the one connection pool), only to its `net`
  hosts, with any method, headers and body bytes; a response carries its status, body and headers.
- **Sockets**: TCP to its `net` hosts, on its own thread (a service's own protocol).
- **Its cache** (`cache`), up to the manifest's `cache_mb`: a file per key in its folder's `cache/`, made
  at the first put; a put over the quota lets the least recently used go. **Its assets** (`assets`): the
  files its manifest names (URL, size, sha256), fetched by nori through the transport in 4 MB range
  requests, hashed as they come, kept as their sha256 in `assets/`. On a metered network an asset waits
  until the user allows mobile data for that plugin (`allow_mobile_assets`, a line on its page). The
  client shows each one's state as a line on the plugin's page (`Plugins::assets`, `Client::asset`). An
  update keeps the store, cache and assets; removing the plugin removes them.

## Permissions

What a plugin may do beyond its own data is in its manifest, shown on the install sheet
(`PluginCard`): `permissions` (`PlaybackRead`, `PlaybackControl`, `LibraryRead`, `LibraryWrite`,
`Subsonic`, `Schedule`), the `hooks` it hears, the `wake` events and `commands` it declares, its cache
quota and its assets' size. These fields come after the places and the devices in the manifest section, so a plugin
built before them reads as declaring none. A call without its permission is refused (`refused`, or
nothing for a call that answers nothing) and logged.

## The session

`session` reaches the session every client shares, without a rule written in a client:

- **What plays** comes from the client's `Moment`s (`Plugins::played`: playing, queue index, song id,
  position, the output's key), said only while `Plugins::follows()` (an enabled plugin reads the player
  or hears an event about it). A moment that only runs on as the last one did is no change. The route is
  read from the output's key (`nori_player::outputs::parts`).
- **The queue and the library** come from the open profile's core: each client hands the profile's
  client over with `Kinds::open_client` (crates/core `providers/plugged.rs`), which is also the
  `profile-opened` event. `subsonic.call` signs the method with the profile's credentials there; the
  plugin never sees them.
- **Controls** go to the client's own player (`Client::control`), so the remote, jam and skip rules apply
  as for a press: crates/host `plugged.rs` on the desktop and terminal, `PlayerConnection.plugin` on
  Android. `add` resolves ids through the core and refuses a provider song unless autofill may add them
  (`autoFillRemote`). `clear` drops the songs after the one playing.
- **`sign-in.open`** is a provider's, for its profile while that is open, at one of its `signin` hosts:
  the client opens its sign-in page there (Android's WebView sheet, the desktop's and terminal's sign-in
  window), and the cookies go to the provider's `sign-in-done` as on its own page.

## Hooks, timers and wakes

- **Built only when heard.** `Plugins::hears(hook)` is one load of a bit set made when plugins are turned
  on or off; a client checks it (or `follows()`) before it builds an event. The playback, song, queue and
  route events are told apart in Rust from the moments (`hooks.rs`); favorites come from the core's
  `Client::star`, settings from the settings' `watch` (stored) and the layer (in effect), media buttons
  from MPRIS and the media session, devices from Android's audio device callback and, on Linux and
  macOS, an audio device BlueZ or IOBluetooth sees connect (`use_bluetooth(.., outputs)`).
- **Lyric lines** wake the host at each synced line of the song playing (its lyrics fetched once per
  song through the core), the screen off too; that wake is the plugin's declared cost, which the install
  sheet says. A pause, seek or song change lets the waiting wake go.
- **Timers** run on the plugin host's executor (made at the first timer, or the client's,
  `use_tasks`). A `while-visible` one fires only while the player is on screen (`player_shown`); an
  `always` one as often as it asks while the plugin has an active session (it runs the cast, or controls
  a connected device: the host's own `Gate::active`), else only with `Schedule`, at most once per 15 minutes and never while music plays
  with the screen off. A timer that may not fire waits on the gate, not on a clock: no wake. The client
  watches the screen (`Client::watch_screen`) only while an `always` timer runs.
- **Wakes and commands.** `wake.want` turns on the wakes the manifest declares (kept in its store); the
  client keeps on only the receivers some enabled plugin wants (`Client::wake`, `Client::commands`). On
  Android, `PluginBootReceiver`, `PluginBluetoothReceiver` and `PluginCommandReceiver` are declared
  disabled and enabled only then; another app runs a command with the `dev.nori.music.RUN_COMMAND`
  broadcast (extras `plugin`, `command`) holding `<applicationId>.permission.RUN_COMMAND`. On Linux, the
  desktop and terminal own `fm.nori.Commands` on the session bus (`Run(plugin, command)`) only while a
  plugin has commands. The desktop has no wake. A command reaches the plugin's `lifecycle.action`.

## The LAN and casting

A plugin with `lan` among its manifest's hosts (`net: &["lan"]`) may reach private addresses: RFC 1918
and RFC 4193 ones, link-local, loopback and the LAN's multicast groups. A host name counts only when
every address it resolves to is private. Without `lan`, sockets and HTTP reach only its named hosts.
What it holds here (browsing, endpoints, audio, a cast) is let go when its instance goes: it is turned
off, removed or updated, or a call fails ([`Lan::forget`](../../crates/plugins/src/lan.rs)).

| interface | what the host does |
|---|---|
| `sockets` | TCP connect, read, write; `tls` wraps it in rustls (ring), checked against webpki's roots (`system`), one certificate's DER (`pinned`), or not at all (`none`, only to a private address); UDP bind (address reused, for SSDP's port), send, receive, and `join` an IPv4 group, which holds the platform's multicast lock while any socket is in one |
| `discovery` | `browse(kind)` asks the client to browse mDNS while any plugin browses that kind (Android NSD, mdns-sd on the desktop and terminal); each service found goes to the `discovered` hook of the plugins browsing it (`Plugins::discovered`) |
| `lan-serve` | the plugins' door (`door.rs`): plain HTTP on a port the system picks, open while a plugin has `open` or serves audio or a song. `/p/<id>/...` goes to plugin `id`'s `served` hook, its query kept. Its own listener, not remote control's door, which takes only requests signed with the account's secret; a speaker can't sign |
| `media` | a song's stream URL (its own quality) and cover URL from the open profile's client (`Kinds::open_client`), as they are; a URL on this device (a server on localhost, plain HTTP) is proxied at the door's `/m/<n>`, its range passed on. A provider plugin's songs have none |
| `audio-out` | `to-socket` (a plain TCP socket the plugin opened, at the rate and channels it asks) or `serve` (at the door's `/a/<encoding>`, nori's rate) make a `Sink`, and the client's engine plays into it (`Plugins::set_cards`, `nori_engine::cast::CastOutput`): pumped in real time half a second ahead, on a thread of its own, 16-bit. PCM is little-endian into a socket and `audio/L16` at the door, WAV has an open length, FLAC has verbatim frames of 4096. While a sink plays, nothing goes to offload; when the plugin stops, the device's own output comes back where the music is (`Engine::set_card`). `set-latency` adds to what the seek bar and lyrics wait for |
| `cast` | `start` tells the client a cast runs (`Client::cast`); a target cast parks the music here (`Card::Elsewhere`, the engine's `play_elsewhere`): the output is let go, and nori's queue moves, plays and pauses for the device, never heard here, at the place and state the device `report`s. `end` or the plugin's instance going ends it, and a target cast's music plays on here from where the device was (`Card::Back`, `come_back`). One plugin casts at a time. The volume keys and picking another output reach the plugin as the media buttons `volume-up`, `volume-down`, `cast-stop` (`Plugins::cast_button`); during a target cast what nori's queue does, from a press or a plugin's `session`, reaches it as `jump:<list index>`, `seek:<ms>`, `play` and `pause` (`Plugins::played`, which clients call while a target cast runs), never what only repeats its own report |

The clients' output pickers (Android's devices sheet, the desktop's devices panel, the terminal's) list
the cast's device beside remote control's; picking this device again sends `cast-stop`. Android holds
a Wi-Fi lock while a sink cast streams, and its media session shows the cast's device while one runs
(`CastPlayer`): the volume keys reach it as a remote device's. During a target cast every client's
player shows the engine's status, which is the device's place and state, and media3 keeps the service in
the foreground while it plays. The desktop and terminal hand the volume to the cast from crates/host's
`Session`.

## Overrides

A plugin changes nori's values by key, never by writing the settings: the user's settings stay as they
are, and turning the plugin off brings nori's own values back.

- A key is a setting's name (`crossfadeSec`), or a tunable's: a value with no setting
  (`playback.speed`, `playback.pitch`, `silence.skip`, `queue.previousAlwaysSkips`; `Tunables`).
- The host keeps a plugin's overrides in its store and lays the enabled plugins' over the settings in
  their order (`Layer`, held by the app's `Settings`); a later plugin's value of a key wins. A setting's
  override goes through that setting's own codec and range.
- The layer changes when a plugin overrides or clears a key, a row changes, or a plugin is turned on or
  off, moved or removed; the client then derives the engine's settings again
  (`nori_engine::core::settings(stored, layer)`), as for a settings change. Nothing is read per buffer.

## Sound stages

A stage runs in one of two slots (the manifest's `stage`). A stage counts as touching samples: songs
stay off offload, and bit-perfect output drops it.

- **Master:** in the chain after the equalizer and before nori's ceiling (a soft limit above -1 dBFS
  that keeps whatever a plugin boosts from clipping), ahead of silence skipping and speed.
- **Song:** on each song as the player reads it, before transitions, so each side of a mix has its
  own (`SongStages` in `crates/player/src/stage.rs`). Each decoded buffer goes through it once; a seek
  or a song made again for a new ending reads from a fresh instance. A change of its rows is heard
  from the next buffer read, after what the output already holds.

- **Its own instance** on the engine's thread, with every import linked to a trap: it can't wait on
  anything. A block (8192 frames or fewer) goes in as one `list<f32>`, channel after channel, and comes
  back the same length. The SDK exports `process` and `save` by hand without a post-return, so the host
  reads the answer in place: no host allocation per block (tests/host.rs counts them).
- **State is kept** like the chain's own: at every mark the host saves the stage (`save`: its inputs and
  the plugin's state bytes; the SDK's `state` module does it for a plain `Copy` struct) and a splice
  restores it, so the music made again is the same, bit for bit
  (`a_stage_made_again_mid_song_sounds_the_same` in the player's pipeline tests).
- **Inputs** (`nori_player::stage::Inputs`): its numeric rows in manifest order, the volume (dB, from
  the media volume: the host session's level on the desktop and terminal, the music stream's on
  Android, followed only while a stage is on) and the route (speaker, wired, Bluetooth, USB, else
  other; nori tells no car or cast route apart), and the song's facts: the gain nori levels it by, its
  measured loudness (once AutoMix measured it) and its plugin's prepared data. A row, volume or
  route change is a sound change, spliced in. A song's facts ride on the input as it is kept, so in
  the master slot a new song's facts start at its first frame and a splice runs again with the facts
  it ran with; in a mix the master slot reads the incoming song's. Prepared data that comes while its
  song plays is spliced in from the first frame the output can still replace. The inputs are copied
  into the plugin's memory on a change (prepared data included), never per block. Facts are looked
  up only while a stage is on.
- **A trap or a block over its fuel** bypasses the stage until the next reset (a seek, another song),
  which makes a fresh instance. Audio never waits for a plugin.

## Song preparation and AutoMix analysis

The enabled plugins exporting `preparation` or `automix` hear each song as the player reads it (the
one decode, the song playing and the next): the reader's tap (`nori_player::stage::Tap`) copies the
decoded samples into a feed made when the song starts and never waits; what the feed can't take is
dropped, and `feed`'s `at-frame` says where the samples start. A thread of its own
(`crates/plugins/src/prepare.rs`, `nori-prepare`) with an instance of each such plugin wakes when a
feed is half full and at the song's end, and sleeps while nothing is read. `begin` (and
`begin-analysis`, for a song read from its start) says whether the plugin wants the song; if none
does, the tap copies nothing more. With none enabled no tap, feed or thread exists.

- **Prepared data** (`finish`) is kept in memory for the last four songs and handed to the plugin's
  stage as the song's facts; data that comes while its song plays wakes the engine, which splices it
  in. It is not kept across restarts.
- **AutoMix analysis data** (`finish-analysis`) is kept the same way and handed to `refine` when nori
  stores its own analysis of the song (waiting a second at most for the plugin's pass to finish).

## AutoMix

The planner (`nori_automix::planner`) asks the plugins through `nori_automix::mixing::Mixing`, where
nori decides and with a 100 ms budget per call; past it, or with none enabled, nori's answer stands.

- **Whether a pair mixes** (the `deciders` point's `mixes`): given nori's same-album rule; false plays
  them gapless.
- **The plan** (`plan`): the first plugin with a plan gives it. nori checks it (`mixing::adopt`):
  positions inside both songs, a length of 20 ms to 60 s, a tempo change only when beat-matched and
  within the setting, an echo only where nori planned one; nori's fades, gains and echo are stretched
  to its length. A plan that doesn't hold is dropped for nori's.
- **Refining an analysis** (`refine`, with the plugin's analysis data): nori keeps it only when it
  holds (`mixing::refined`: tempo 40 to 250 BPM or none, key, cues inside the song and in order,
  loudness), and the beat grid starts at its first beat.

## Slots

Places in nori's screens a plugin fills, each named in its manifest (`places`, `menu`) and drawn by the
client from what the plugin answers (`crates/plugins/src/slots.rs`); one not named is never asked.

| place | the plugin answers | the client |
|---|---|---|
| menu actions | `menu(action, items)` when one is tapped | lists `menu_entries(kind)` in the menus of songs, albums, artists, playlists (from the manifest, no call) |
| song info | `song-info(song)`: lines, with a link each | shows them under the plugin's name |
| artwork | `artwork(song)`: a still image or a muted loop, or none (nori's cover) | plays the loop in the player's sleeve (Moving covers' rules) |
| player status | `player-status()`: one line | shows it under the output row |
| devices | nothing: its rows | shows its page's rows on the devices sheet ("Playing on") too, after nori's own entries |

## Deciders

A plugin exporting `deciders` may answer where nori picks something (`crates/plugins/src/deciders.rs`,
`nori_model::deciders::Deciders`). nori works out its own answer first; the enabled plugins exporting the
point, in their order, each get the inputs and the answer so far and return another or none. nori checks
each answer before it is used or handed on; a refused answer, a failed call or one past its budget keeps
the answer so far. The domain crates hold the deciders as a trait object, set by `Core::use_kinds`; while
no enabled plugin exports the point (`Deciders::deciding`), no input is built and nothing is called.

| decider | runs | budget | nori checks |
|---|---|---|---|
| shuffle | shuffle turned on, or a queue set shuffled (`nori_queue` `Session::shuffle`, `set`, `set_shuffled`) | 50 ms | every list index once, starting where nori's order starts (the song playing) |
| autofill | after nori's refill (`Client::autofill`), async | fuel only | each id resolves to a song, isn't queued, and is a provider song only where "may add provider songs" allows; none left keeps nori's |
| queue-ended | the queue played out (`Client::queue_ended`; host and Android append and play what it gives) | fuel only | as autofill |
| leveling | the song's ReplayGain (`Session::queue_gain`), once per song and gain; not while leveling is off | 250 ms | finite, -60 dB to nori's boost ceiling |
| sound-profile | an output arrives (`Core::device_arrive`) | 250 ms | a saved profile, or none |
| now-playing | Android's notification and lock screen (as a song starts), MPRIS on the desktop and terminal; once per song | 250 ms | a title |
| media-button | Android's media buttons (taken from media3 only while a plugin decides), MPRIS play-pause, next and previous | 50 ms | an act of the enum; `plugin` runs its lifecycle `action("media-button")` and the player does nothing |
| audio-focus | Android's focus changes (`Session::audio_focus`, nori's rule `focus_act`) | 50 ms | an act of the enum |
| look | the season's palette with no cover: Android's theme, the desktop's plain pages, the terminal's theme | 250 ms | contrast (`nori_look::theme::readable`) |

`mixes` is AutoMix's (AutoMix seams). On Android double and triple presses stay media3's own (it counts
them); a long press reaches the decider.

## Device control

A plugin names the Bluetooth devices it reaches in its manifest (`devices`: a name pattern with `*`, a
service UUID, a vendor id; one matches when all it gives matches), and only those: `bluetooth.connected`
lists them, and a channel opens to no other. The install sheet says it reaches Bluetooth devices.

- **The platform's Bluetooth** is the client's `Bluetooth` (`crates/plugins/src/bluetooth.rs`), given with
  `Plugins::use_bluetooth`: Android's `BluetoothSocket` and `BluetoothGatt` (`core/.../PluginBluetooth.kt`),
  BlueZ over the system bus on Linux (`crates/host/src/bluez.rs`, zbus as the media controls; RFCOMM
  through a client `Profile1`), and on macOS IOBluetooth for classic devices and RFCOMM and CoreBluetooth
  for GATT (`crates/host/src/macbt/`). There a device is its classic address (a device only on LE, its
  CoreBluetooth identifier, listed when connected to the system with a common service), and GATT to a
  classic device goes to the LE peripheral of its name. IOBluetooth runs on a thread of its own whose run
  loop sleeps between events; CoreBluetooth's delegate on a dispatch queue. Both are made at the first
  need and let go once nothing is watched or open. The app's `Info.plist` gives
  `NSBluetoothAlwaysUsageDescription` (`tools/host-release/app.py`).
- **Watched only while needed.** The platform is told to watch devices connecting only while an enabled
  plugin reaches devices; with none, no receiver, bus connection or thread exists. On Android the
  `BLUETOOTH_CONNECT` permission is asked when such a plugin is installed, and on its page while missing.
- **Links** are numbered by the host. What comes on one the platform hands to `bluetooth_heard`: a GATT
  notification goes to the plugin's `notified` hook; an RFCOMM channel's bytes wait for its `read`, and
  those no read took come to `notified` with the service UUID as the characteristic. A link closes when
  the plugin drops it or its instance goes.
- **Who controls a device.** A device connecting is offered to the enabled `device-control` plugins
  reaching it, in their order (`handles`); the first that says yes controls it while it stays connected.
  nori asks for its `state` then, after each notification and after `set-noise`, never on a timer.
  Disconnected, the plugin hears `gone`, and an instance there only for devices goes.
- **What clients draw** comes from `Plugins::controlled`: per device its batteries, the level the output
  row shows (`Battery::level`: a single battery, else the emptier bud, never the case), its noise
  control and modes, and the controlling plugin's card. `output_device` picks the one the music plays
  on (by name or id; the system's default output is the only one connected). Android draws it on the
  devices sheet (a card for the headphones playing, with a way on to their page), the player's output
  row (`WH-1000XM5 · 80%`), the equalizer's device page and the plugin's page; the desktop and the
  terminal on their devices panel and output row.

## Covers

A plugin's covers are URLs with `{size}` where the pixels go; the core fills it in
(`cover_address`), so no call per cover reaches the plugin.

## Installing

`preview_file(path)` and `preview_version(version)` (fetched and checked against its sha256) read a
package's manifest and words without installing it, for the sheet that shows what it is and may reach
before the user installs it (`install_preview`).

## Providers

A plugin with a `provider` export adds a kind of profile: `ServerKind::Plugin { id }`, stored as its id.
A client asks a `Kinds` (`Kinds::with_plugins`, or `Kinds::builtin` without plugins) for the kinds it
offers, their forms and sign-in, and hands it to each `Core` it opens (`Core::use_kinds`) and its
transport (`Kinds::set_transport`). See [Providers](providers.md).

## Tests

- `cargo test -p nori-plugins`: the manifest as the SDK writes it, the extension points' order against
  `wit/`, the store, and `tests/host.rs` over a real plugin (`fixture/`, built to wasm by the `testing` feature:
  needs `rustup target add wasm32-wasip2`): its stage and kept state, no allocation per block, its
  overrides, its HTTP refused outside its hosts, and a plugin needing an interface this host doesn't
  link refused (the fixture built with `unlinked`, `fixture/src/unlinked.rs`);
  `tests/lan.rs` the LAN interfaces over the same plugin: TLS checked each way against a local server,
  UDP to the LAN only, the multicast lock, browsing, its endpoints at the door, a song's URL proxied, and
  casts into a socket, at the door and to a target. `tests/devices.rs` runs device control over a fake
  platform Bluetooth with the fixture built with `headphones` (`fixture/src/headphones.rs`).
  `tests/session.rs` drives the fixture's `hooked` module on a simulated clock: the session's reads and
  controls and their permissions, events built only when heard, lyric lines' wakes, timers and their
  gate, the cache's quota, assets on Wi-Fi or mobile data, and wakes and commands kept on only while wanted.
  `tests/preparation.rs`: a stage's inputs, the song slot, song preparation and AutoMix analysis as
  the tap feeds them, refinements and plans.
- `cargo test -p nori-plugins --test deciders`: each decider over the test plugin, nori's checks, the budget.
  Their seams with a fake: `cargo test -p nori-queue deciders`, `-p nori-core autofill profiles`.
- `cargo test -p nori-engine --test engine another_card`: a card swapped mid-song plays on from there.
- `cargo test -p nori-player --test pipeline stage`: stages in the chain on the virtual clock: kept
  state, a song's facts from its first frame, prepared data spliced in, the song slot, the tap.
- `cargo test -p nori-automix mixing`: what nori checks of a plugin's plan and refinement.
- `cargo test -p nori-settings overrides`: tunables and settings overridden without being written.
