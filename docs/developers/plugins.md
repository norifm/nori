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
  `strings/`, an optional `icon.png` and an optional `README.md` (`tools/pack.py` in nori-plugins). A plugin's `cdylib` doesn't
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
- **The API version.** A component's types must match exactly, so any change to a type in `wit/`, even a
  new case in a variant, breaks every plugin built before it: raise `API` (crates/plugins and the SDK) and
  the package version in `wit/`, then release every plugin. A repo keeps each plugin's older versions, so
  an older nori stays on the ones it runs. An installed plugin built for an older API stays listed, with
  its store, and updates are looked for at once.
- **Install checks first.** A package is unpacked beside the old version and refused when its manifest
  can't be read, its API version is not this host's, its id isn't reverse DNS, it
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
  It goes through the client's proxy as nori's own requests do: on Android OkHttp's, the system's
  (an HTTP proxy, host and port, or a PAC file: Android has no TLS to a proxy); on the desktop and
  terminal nori-http's ureq, the proxy `ALL_PROXY`, `HTTPS_PROXY` or `HTTP_PROXY` names (`NO_PROXY`
  excepted), `http://` or `https://` (TLS to the proxy, then CONNECT, tested against a local HTTPS
  proxy). SOCKS is not built in: with a `socks5://` one named, requests go direct.
- **Sockets**: TCP to its `net` hosts, on its own thread (a service's own protocol). Never through a
  proxy: a socket is mostly for the LAN, and an HTTP proxy carries no UDP.
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
- **The output picker** (`output-picker(open)`): Android's devices sheet while it is resumed, the
  desktop's devices panel (the full player's or the side sheet's), the terminal's devices panel. Each
  client says whether its picker is open (`Plugins::output_picker`, crates/host's
  `Handle::output_picker`) whenever it might have changed; the host tells the plugins only a change, so
  a plugin that finds devices (`discovery`, SSDP) browses only while the user is picking.
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
| `sockets` | TCP connect, read, write; `tls` wraps it in rustls (ring), checked against webpki's roots (`system`), one certificate's DER (`pinned`), or not at all (`none`, only to a private address); UDP bind (address reused, for SSDP's port; one IPv6 socket that takes IPv4 too, its addresses mapped, or IPv4 alone where the system has no IPv6), send, receive, and `join` a group of the LAN, IPv4 (239/8, 224.0.0/24) or IPv6 of interface-, link- or site-local scope (`ff02::c`, `ff05::c`; `ff02::fb%3` on interface 3, else the system's pick), which holds the platform's multicast lock while any socket is in one. A sender is heard as IPv4 when it is, and a link-local one with its zone (`fe80::1%3`), which `send` and `tcp.connect` take back |
| `discovery` | `browse(kind)` asks the client to browse mDNS while any plugin browses that kind (Android NSD, mdns-sd on the desktop and terminal); each service found goes to the `discovered` hook of the plugins browsing it (`Plugins::discovered`) |
| `lan-serve` | the plugins' door (`door.rs`): plain HTTP on a port the system picks, open while a plugin has `open` or serves audio or a song. `/p/<id>/...` goes to plugin `id`'s `served` hook, its query kept. Its own listener, not remote control's door, which takes only requests signed with the account's secret; a speaker can't sign |
| `media` | a song's stream URL (its own quality) and cover URL from the open profile's client (`Kinds::open_client`), as they are; a URL on this device (a server on localhost, plain HTTP) is proxied at the door's `/m/<n>`, its range passed on. A provider plugin's songs have none |
| `audio-out` | `to-socket` with `pace.deep` makes a deep sink (below). Otherwise `to-socket` (a plain TCP socket the plugin opened, at the rate and channels it asks, or nori's) or `serve` (at the door's `/a/<encoding>`, nori's rate) make a `Sink`, and the client's engine plays into it (`Plugins::set_cards`, `nori_engine::cast::card`): pumped in real time half a second ahead, on a thread of its own, 16-bit. PCM is little-endian into a socket and `audio/L16` at the door, WAV has an open length, FLAC has verbatim frames of 4096. While a sink plays, nothing goes to the phone's offload; when the plugin stops, the device's own output comes back where the music is (`Engine::set_card`). `set-latency` adds to what the seek bar and lyrics wait for |
| `cast` | `offer` lists the devices it can cast to now: each is a row of nori's own devices list on every client, and picking one runs its `action("cast:<id>")`, so a speaker needs no row of the plugin's (`Plugins::cast_targets`, `cast_to`). What is offered is also published in the device's remote state (`DeviceState.outputs`): the account's other devices list each output once, through the device playing where it reaches it, and picking one moves the music there and asks it to cast (`Remote::outputs`, `cast_on`, `Op::CastTo`). A device casting reports the cast's volume as its own, so controllers set it. A plugin keeps offering what it last found: it browses only while a picker is open. `start` tells the client a cast runs (`Client::cast`); a target cast parks the music here (`Card::Elsewhere`, the engine's `play_elsewhere`): the output is let go, and nori's queue moves, plays and pauses for the device, never heard here, at the place and state the device `report`s. A slider sets a deep sink's volume as it is (`Plugins::cast_volume`), a plugin's own device a step toward it. A deep sink's socket lost ends its cast: the music comes back to the device's own output and the plugin hears `cast-stop`. `end` or the plugin's instance going ends it, and a target cast's music plays on here from where the device was (`Card::Back`, `come_back`). One plugin casts at a time. The volume keys and picking another output reach the plugin as the media buttons `volume-up`, `volume-down`, `cast-stop` (`Plugins::cast_button`); during a target cast what nori's queue does, from a press or a plugin's `session`, reaches it as `jump:<list index>`, `seek:<ms>`, `play` and `pause` (`Plugins::played`, which clients call while a target cast runs), never what only repeats its own report |

The clients' output pickers (Android's devices sheet, the desktop's devices panel, the terminal's) list
the cast's device beside remote control's; picking this device again sends `cast-stop`. Android holds
a Wi-Fi lock while a sink cast streams (a deep sink's only while a burst is sent), and its media session shows the cast's device while one runs
(`CastPlayer`): the volume keys reach it as a remote device's. During a target cast every client's
player shows the engine's status, which is the device's place and state, and media3 keeps the service in
the foreground while it plays. The desktop and terminal hand the volume to the cast from crates/host's
`Session`.

### Deep sinks

`audio-out.to-socket` with `pace.deep(depth-ms)` makes a deep sink: a device that holds tens of seconds
(a speaker with PSRAM) and takes nori's audio the way a phone's AudioTrack does, a burst and then nothing
until it runs low, so the phone's radio and CPU sleep between bursts. The host speaks the protocol
([nori-sink-protocol](../../crates/sink-protocol/src/lib.rs), `no_std` + `alloc`, so firmware uses the
same code) over the plugin's socket; the plugin only finds the device, connects, calls `to-socket` and
starts the cast. `encoding` is not used; `rate` and `channels` none means each song's own format. No
plugin code runs per buffer (`nori_plugins::deep`, `nori_engine::cast::deep`).

**Framing.** Every frame is a `u32` length (little-endian, the bytes after it), a kind byte, then its
fields, little-endian: `u16`/`u32`/`u64`, `f32` as its bits, strings a `u16` length and UTF-8, an option a
byte (0 none, 1 some) and the value; byte payloads run to the frame's end. A frame is at most 1 MiB.
`Frames::push` gathers bytes; `next_host`/`next_device` hand out frames borrowing them.

**Handshake.** The host sends `Hello { version }` and waits (3 s) for `Caps`; nothing else comes before.
Then `Depth`, and before the first audio a `Format`.

Host to device (kind):

| frame | fields | meaning |
|---|---|---|
| `Hello` 0x01 | `version: u16` | first frame; the device answers `Caps` |
| `Depth` 0x02 | `ms: u32` | how much to hold; ask for more when under half of it. The host fits it to `buffer_bytes` at the format (90 %); 250 ms while a sound page is tuned, then full again |
| `Format` 0x03 | `rate: u32, channels: u8, bits: u8, codec: u8` (0 PCM, 1 FLAC, 2 MP3) | the audio after it is in this format; `bits` is PCM's width (16, 24), 0 for encoded (FLAC says its own) |
| `Audio` 0x04 | samples | PCM, whole frames, interleaved, little-endian (24-bit packed in 3 bytes) |
| `Encoded` 0x05 | bytes | one whole packet of the file (a FLAC or MP3 frame); a FLAC song's first is `fLaC` and its STREAMINFO block, which counts no frames |
| `Gain` 0x06 | `db: f32` | the song whose audio follows plays at this gain (ReplayGain). Sent only before encoded songs: in PCM nori has applied it to the samples |
| `Flush` 0x07 | `seq: u32, keep: option<u64>` | drop what is held past stream frame `keep`; none, or if it already played past `keep`, drop everything not played. Answer `Flushed` and `Status` |
| `Pause` 0x08, `Resume` 0x09 | | stop at once keeping what is held; go on. Answer `Status` |
| `Ramp` 0x0a | `target: f32, ms: u32` | fade from the fade's current gain to `target` (linear, 0 to 1) over `ms`; `ms` 0 sets it |
| `Volume` 0x0b | `gain: f32` | the user's volume, linear 0 to 1, heard at once (not positional) |
| `Cover` 0x0c | `id: u32`, JPEG | a cover at the size `Caps` asked, kept for `Now` to name; sent once, before the first `Now` that names it |
| `Now` 0x0d | `id: u32, title, artist, album, duration_ms: u32, cover: option<u32>` | the song whose audio follows |

Device to host:

| frame | fields | meaning |
|---|---|---|
| `Caps` 0x81 | `version: u16, buffer_bytes: u32, max_rate: u32, bits: u8` (bit 0 16-bit, bit 1 24-bit), `decodes: u8` (bit 1 FLAC, bit 2 MP3), `cover: option<u32>` (px square; none: no screen) | what it can do. Only a song above `max_rate` is resampled; FLAC and MP3 it decodes go as their own packets |
| `Want` 0x82 | `frames: u32` | send this many frames more; replaces credit left |
| `Status` 0x83 | `played: u64, held: u32, delay_us: u32` | frames played since Hello (never reset by a flush; the host subtracts), frames held, the delay after the buffer |
| `Flushed` 0x84 | `seq: u32, played: u64` | the flush `seq` is done, at `played` |
| `Press` 0x85 | `button: u8` (0 play-pause, 1 next, 2 previous) | a button: nori takes it as a media button, so plugins' deciders apply |
| `Volume` 0x86 | `gain: f32` | its own knob: the cast's volume in nori, not sent back |

**Positional frames.** `Format`, `Gain` and `Now` apply where they are in the stream: what came before
plays out in its format, and the device switches exactly at the next audio. They wait in the buffer with
the audio. A flush drops them with the audio past `keep`; covers and the last `Now` shown survive it.
After a flush the first audio is preceded by `Format` (and by `Gain` and `Now` when they apply), so the
device never guesses.

**Flow control is credit, not TCP.** The device reads its socket always. When it holds less than half
its depth it sends one `Want` for what fills it (and a `Status`); the host writes exactly that much audio
and then nothing until the next `Want`. PCM never exceeds the credit; encoded audio goes in whole
packets until the frames they cover reach it, so the last overshoots by less than a packet (accept it);
counts stay exact through packet durations, and no credit splits a FLAC or MP3 frame. Control frames go
out at once, never behind unasked audio, and the host keeps its send buffer small (64 KB), so a seek or a
pause is heard in milliseconds, not after the device drains megabytes. The pump sleeps between `Want`s,
woken only by the device, the engine (a command, music written at the end of its turn) or the socket;
never a timer. The Wi-Fi lock (Android) is held only while a burst is sent (`Client::streaming`).

**When Status is sent:** with each `Want`, and in answer to `Pause`, `Resume`, `Depth` and `Flush`;
never periodically. The host runs the place on from the last one by its own clock (the seek bar, lyrics,
the engine's deep-output hooks: `holding`, `latency_us`).

**Flushes.** A seek or a jump drops everything (`keep` none). A sound change keeps what the device holds
up to the frame where the music was made again (`keep`) and continues from there, so it is heard within
a fraction of a second, seamlessly; the engine keeps the input reaching back over the device's depth to
make it again.

**Format follows the song.** With no rate asked, each song goes at its own rate, channels and the width
the device takes (24-bit with high quality output on). The engine reopens its output where it always does
between songs of different formats (after a crossfade or AutoMix mix, never inside one: the mix converts
the incoming song); a deep sink takes the `Format` in its stream instead of draining first.

**Offload.** While nothing touches the samples (`nori_player::policy`: no equalizer, effect, speed or
mix) and the device decodes the song's codec, the engine's offload path sends the file's own packets
(`Format` with the codec, then `Encoded`), as Android's offload output does: the same switch points
(PCM again when the equalizer comes on, behind a dip where the ear is; back at the next song or where
the ear is), seeks by the file's seek table or bitrate (FLAC seek table, MP3 Xing/TOC), a `Flush` and the
packets from there. ReplayGain is `Gain` at each song's start; fades are `Ramp`s. `Status` counts decoded
frames. MP3 encoder gaps are not told: an MP3 that joins its album gaplessly plays as PCM.

**Now playing.** At each song's first frame (a gapless boundary, after a skip's flush, where a mix's
next song begins) the host puts `Now`, with the song's cover sent once before it if `Caps.cover` asked a
size (from nori's cover cache at that size; JPEG only). An album's songs share a cover id. Covers are
never fetched for songs the engine is not already reading; one not on disk yet is fetched in the
background and goes with the album's next song, so the audio threads never wait on the network.

**What firmware implements:** read the socket always; answer `Hello` with `Caps`; keep a FIFO of audio
and positional frames with stream frame counts; play it, switching format, gain and now-playing exactly
at their place; send `Want` when under half the depth (one at a time: the next once the last's frames
arrived or it is low again), `Status` at the times above; `Flush` by truncating at `keep` (or everything)
and dropping incoming audio up to where it played; `Pause`/`Resume` at once; `Ramp` and `Volume` as gain
stages (fade × volume × song gain); `Press` and `Volume` from its buttons and knob.

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

`preview_file(path)` and `preview_version(version)` (fetched and checked against its sha256 and its
signature) read a package's manifest and words without installing it, for the sheet that shows what it
is and may reach before the user installs it (`install_preview`).

### Repos and signatures

A repo is a static `index.json` (`crates/plugins/src/repo.rs`):

```json
{"plugins": [{"id": "fm.nori.orbit", "name": "Orbit", "about": "…", "source": "https://github.com/…",
  "readme": "orbit/README.md", "icon": "orbit/icon.png", "points": ["sound-stage"],
  "versions": [{"version": "1.2.0", "api": 2, "released": "2026-11-02", "url": "orbit-1.2.0.noriplugin",
    "sha256": "<hex>", "key": "<hex, 32 bytes>", "signature": "<hex, 64 bytes>"}]}]}
```

- `readme` is the plugin's Markdown README, absolute or relative to the index: its page shows it before
  it is installed. Once installed, a `README.md` in its package goes first (`Plugins::readme`). `icon` is
  the same for its icon (`Plugins::icon`): a PNG of at most 256 KiB, 256 px square, one color on
  transparent, which each client with images draws in the page's ink.
- What a client's plugin pages show comes from the core: `offers_found` for the browser's search and
  repo, `updates_waiting` for the list, `offer_of` for the offer a plugin's page shows, and
  `offers_stand` for offers read before an install, update or removal. Android, the desktop and the
  terminal have the same pages: the list, the browser, the repos, and a plugin's page.
- `points` are the extension points it fills. A first screen lists those filling `provider` before they
  are installed (`provider_offers`).
- `key` is the author's ed25519 public key, `signature` that key's signature over the package's SHA-256
  (the 32 raw bytes, not the hex). Both or neither; one without the other, or a signature that doesn't
  verify (`verify_strict`), refuses the version (`Refusal::Signature`).
- The first signed version installed pins its key (`index.json`'s `key` beside the plugin). From then
  on every version must be signed by that key: another key, an unsigned version, or a file is
  refused (`Refusal::OtherKey`) until the user removes the plugin and installs it again.
- Unsigned versions and `.noriplugin` files install while no key is pinned, and the sheet says
  "Unsigned" (`PluginPreview::key` is None). A package can't carry its own signature (it would sign
  itself), so a file is always unsigned; once a plugin's key is pinned, only its repo updates it.
- Updates: each plugin updates from the repo it came from, at most once a day, unless held at its
  version ("Keep this version"). An update refused for its key is logged and left for the user, who
  sees the refusal when installing it by hand.
- Signing, for authors: `cargo run -p nori-plugin-sdk --example sign -- new KEYFILE` makes a key (prints
  the public key); `cargo run -p nori-plugin-sdk --example sign -- KEYFILE PACKAGE` prints the version's
  `sha256`, `key` and `signature` fields. Any ed25519 tool signing the digest's bytes does the same.

## Safe mode

`crates/plugins/src/safe.rs`. `plugins/safe` is a small file the host maps into memory once plugins are
installed, locked while that run has it. Each call into a plugin is marked in its slot (one per plugin
and caller: its own thread, its preparing instance, its sound stage) with an atomic store as it starts and one as it ends;
loading its compiled code counts as a call. The kernel keeps the file as it was however nori stops
(a panic, an abort, a native crash, the system ending it). A panic in a call stays marked even when it
is caught. A wait inside a call (an HTTP answer, a socket's or a Bluetooth channel's read) is unmarked, so
Android ending nori while a plugin waits blames no plugin.

- At the next start, a plugin nori stopped in is turned off and logged (the bug report's log carries it);
  `take_safe_start()` answers `SafeStart::Crashed` once, for the client to say.
- The header marks a start until the client calls `started()` (Android: the first frame, or the player
  service up; desktop and terminal: the session or the sign-in up). A start that didn't get there, with
  no plugin marked, starts the next one with every plugin off (`SafeStart::AllOff`), their own switches
  kept; the one after runs them again. Turning a plugin on ends it.
- A second nori over the same folder (the file is locked) reads nothing and marks nothing.

## Every plugin off

The `pluginsOff` setting (a switch in the head of each client's Plugins page, a row at the terminal's
top; the list goes grey while it is off) turns every plugin off without touching their own switches, to tell a nori bug from a plugin's: `Plugins::running()` is empty, so no plugin is
called, and the settings leave the plugins' layer out (`Settings::layer`), which is kept for when the
switch goes back. While it is off the first screen offers no plugin's provider (`provider_offers`), since one
turned on there wouldn't run. `Plugins::report()` lists the plugins on, signed or not, and what each overrides, in
English, for Android's bug report and the perf build's self test.

## Providers

A plugin with a `provider` export adds a kind of profile: `ServerKind::Plugin { id }`, stored as its id.
A client asks a `Kinds` (`Kinds::with_plugins`, or `Kinds::builtin` without plugins) for the kinds it
offers, their forms and sign-in, and hands it to each `Core` it opens (`Core::use_kinds`) and its
transport (`Kinds::set_transport`). See [Providers](providers.md).

## Lyrics

A plugin with a `lyrics` export is a lyrics service in nori's race (`nori_lyrics::race`), ranked after
nori's own services, asked in the first wave and scored and checked against the voice like them
(`PluginLyrics`, `Plugins::lyrics_services`). It answers lines, timed or not (no word timing), credited
as `LyricsOrigin::Plugin { id }`, which each client names by the plugin's name. It is asked under the same
switches as nori's services (online lookups, third-party lookups), not when a jam's host chose one of
nori's services. A miss is cached like a service's; a failed call is asked again next time.

## Scrobbler

A plugin with a `scrobbler` export is told `now-playing` when a song starts and `scrobble` when a play
counts by nori's rule (half the song or 4 minutes), from `nori_queue::Session::scrobble_track`
(`Scrobbles`, set by `Core::use_kinds`), on its own thread; nothing waits for it. Its account is its own:
the server's scrobbling switch and the profile's rules (a jam guest's) don't apply to it.

## Backups

A backup (crates/backup) keeps the installed plugins as `Plugins::kept` gives them: in their order,
each with its id, version, repo, being on and being held, and every value in its store (its rows, its
overrides, its secrets and its own keys), plus the repos the user added. The file is sealed with the
password the user sets: Argon2id (64 MiB, 3 passes, one lane, kept in the header) makes a 64-byte
output whose first half is the XChaCha20-Poly1305 key and whose second half is stored to tell a wrong
password from a damaged file; the whole header is the cipher's associated data.

`Plugins::restore` adds the repos, fetches each plugin from its own repo at the version backed up, else
the newest that repo lists, else keeps the one installed here, then writes its store over the one
there and puts the order (the backup's first, others after), being on and being held back. Each
plugin's `PluginReturn` says how it came back: `AsKept`, `Other { version }`, `FromFile` (installed
from a file, which nothing can fetch again) or `Unavailable` (its repo doesn't list it now or couldn't
be read). Restoring needs no consent sheet: the user allowed each plugin when it was first installed. A
plugin's `cache/` and `assets/` are not kept: it fills its cache again and its assets are fetched
again (whether it may fetch them on mobile data is in its store, so that comes back). What the
platform grants (Android's Bluetooth permission) is asked again when a plugin needs it.

The rest of a backup: every stored setting as the settings table keeps it (read back through `load`,
so an older backup moves on as an older database does), the server profiles with their passwords among
them, and the saved sound profiles. A restore keeps the profiles here beside the backup's (one with the
same id, or the same address and user, stays as it is here) and the profile in use stays in use. Not
in a backup: each profile's library index, history, downloads and caches (the server has them, or
they are fetched again), its smart playlists (a profile's library data, not settings), and `app_kv`
(this device's id, update checks).

## Tests

- `cargo test -p nori-plugins`: the manifest as the SDK writes it, the extension points' order against
  `wit/`, the store, and `tests/host.rs` over a real plugin (`fixture/`, built to wasm by the `testing` feature:
  needs `rustup target add wasm32-wasip2`): its stage and kept state, no allocation per block, its
  overrides, its HTTP refused outside its hosts, and a plugin needing an interface this host doesn't
  link refused (the fixture built with `unlinked`, `fixture/src/unlinked.rs`), signatures and the
  pinned key, safe mode's two cases and the switch over every plugin;
  `tests/lan.rs` the LAN interfaces over the same plugin: TLS checked each way against a local server,
  UDP to the LAN only over IPv4 and IPv6, which groups it may join and the multicast lock, an IPv6
  link-local group's datagram answered at its sender's zone (it needs an IPv6 interface with
  multicast, as any Linux with a network has), browsing, browsing while the output picker is open, its endpoints at the door, a song's URL proxied, and
  casts into a socket, at the door and to a target. `tests/devices.rs` runs device control over a fake
  platform Bluetooth with the fixture built with `headphones` (`fixture/src/headphones.rs`).
  `tests/session.rs` drives the fixture's `hooked` module on a simulated clock: the session's reads and
  controls and their permissions, events built only when heard, lyric lines' wakes, timers and their
  gate, the cache's quota, assets on Wi-Fi or mobile data, and wakes and commands kept on only while wanted.
  `tests/preparation.rs`: a stage's inputs, the song slot, song preparation and AutoMix analysis as
  the tap feeds them, refinements and plans.
  `tests/lyrics.rs` and `tests/scrobbler.rs`: its lyrics service in nori's race and its scrobbles.
- `cargo test -p nori-plugins --test deciders`: each decider over the test plugin, nori's checks, the budget.
  Their seams with a fake: `cargo test -p nori-queue deciders`, `-p nori-core autofill profiles`.
- `cargo test -p nori-engine --test engine another_card`: a card swapped mid-song plays on from there.
- `cargo test -p nori-player --test pipeline stage`: stages in the chain on the virtual clock: kept
  state, a song's facts from its first frame, prepared data spliced in, the song slot, the tap.
- `cargo test -p nori-automix mixing`: what nori checks of a plugin's plan and refinement.
- `cargo test -p nori-settings overrides`: tunables and settings overridden without being written.
- `cargo test -p nori-backup`: a backup round trip between two devices (settings, sound profiles, and
  the fixture and its `second` build with their stores, order, being on and held), what a restore does
  for a version its repo dropped, a file-installed plugin and a plugin its repo no longer lists, the
  profile in use kept, and wrong passwords, changed bytes and other files refused with nothing
  changed.
