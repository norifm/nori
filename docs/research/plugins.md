# Plugins

Internal planning note, not published. Written 2026-10-24 against `master` at `2413016a`; updated
2026-10-27 at `29bc5e0e` with what each feature nori removed needs to come back as a plugin, and what
the plugin system may cost with no plugin.

Built (2026-10-27, the `plugins` branch; docs/developers/plugins.md): the whole API in `wit/`; the SDK
(traits written from the WIT, the manifest in the wasm's custom section, not a `plugin.toml`); the host
with `log`, `store`, `http`, `settings`, `rows` and `notices`; overrides (settings by name, and the
tunables speed, pitch, skip silence, previous always skips); master-slot sound stages; plugins'
providers (YouTube Music moved out of nori into one); rows and the Plugins page in each client but
the iPod and the web. Built since (2026-10-30, the `plugin-apis` branch): song preparation, AutoMix
seams, slots, deciders, hooks' events and wakes, device control (Bluetooth on Android, Linux and macOS),
the LAN (IPv4 and IPv6 multicast, TLS) and casting with deep sinks (crates/sink-protocol), lyrics
services and scrobblers, repo signatures with key pinning, safe mode, one switch over every plugin,
and backups (crates/backup). Not yet: the web host, macOS Bluetooth run on a Mac.

## Owner decisions so far (2026-10-24)

- Plugins are WebAssembly components, sandboxed, for every client except the iPod (Android, desktop,
  terminal, web). A plugin that needs something a client lacks is not offered there (YouTube needs
  cross-origin HTTP, so not the web).
- Plugins should be very extensive: providers, sound processing in the audio path, AutoMix analysis and
  planning, headphone control over Bluetooth, nori's hardcoded and derived values.
- UI is declarative: a plugin describes rows, each client draws them in Seasons.
- Install shows every permission at once; the user allows all or cancels.
- Repos are static indexes on GitHub (nori's own preloaded), more can be added.
- Updates: a per-plugin choice (automatic, or held at its version).
- A plugin changes nori's values as an override layered over the settings, not by writing them: the
  user's own settings are never touched, and removing the plugin leaves nothing behind. A settings page
  whose values a plugin overrides says so once, at its top.
- Settings removed in 0.6.3 (#69) do not come back to nori's settings. What still runs becomes
  overridable; what was deleted (loudness compensation, the expander) must be possible as a plugin, even
  if nori never ships one. nori ships no "Advanced settings" plugin of its own.
- A plugin's manifest states every nori value it overrides; it can override nothing else. Enabled
  plugins apply in the user's plugin order and the last one wins. Two plugins that override the same
  value are flagged at install and on both plugins' pages.
- Plugins and their settings are for the whole app, like nori's settings, not per profile. A plugin can
  still check which profile and provider is open. Backups include them.
- Plugins live on their device. What they decide for the session (queue, transitions) reaches remote
  controllers and jam guests as the session's own state; their sound processing and pages stay on the
  device. A controller doesn't see or change another device's plugins.
- Backups carry plugins with their stores, secrets included, encrypted with a password the user sets
  when making the backup.
- Safe mode: a crash during a plugin's call disables that plugin on the next start; a startup crash
  that names no plugin starts once with every plugin off.
- Casting (Snapcast, UPnP/DLNA, Chromecast) is plugins, not built into nori.
- A plugin can't change nori's UI: it has its own pages (and a device's section, a provider's add
  form), drawn by nori from its rows, and fills slots in nori's own screens, drawn by nori from the
  plugin's data (Slots, decided 2026-10-27).
- Distribution is GitHub and F-Droid. Google Play is not a goal; if it ever is, a build feature switches
  off what Play forbids.

## Owner decisions (2026-10-27)

- With no plugin installed, nori runs as it would without the plugin system: the same wakeups, CPU,
  allocations, threads and start time. Every seam below is built to that rule (Performance rules).
- APK size is not a constraint: Cranelift ships, and so does what a slot needs (HLS for moving covers).
- Every feature nori removed must be possible again as a plugin, even if nori never ships one (What
  nori removed, as plugins).

## What a plugin is

A `.noriplugin` file: a zip of

| file | what |
|---|---|
| `plugin.toml` | id (reverse DNS), version, plugin API version (`nori:plugin@1`), author's public key, permissions, what it extends, the nori values it overrides, its rows |
| `plugin.wasm` | one WebAssembly component (WASI 0.2) |
| `strings/<lang>.toml` | its words; English required |
| `icon.svg` | optional |

The plugin API is a WIT package, `nori:plugin`, versioned on its own. A host supports the current major
version and the one before. Every addition to it is a promise: it covers what a client can already do
and the extension points below, never nori's internals, so the engine can still be refactored.

**A plugin can do whatever a client can do, plus the extension points.** The client powers are the
`crates/host` session API (queue, playback, library, settings, jams), each area behind a permission.

## Where it lives

| piece | what |
|---|---|
| `crates/plugins` | the host: loading, compiling, the WIT bindings, permissions, the plugin store, overrides, repos and signatures, the sound stage's instance. Behind a `plugins` cargo feature, on in release clients |
| `crates/plugin-sdk` | for plugin authors: `wit-bindgen` macros, the sound stage's state region, test helpers. Published with a template repo |
| `wit/` | the `nori:plugin` package |
| `crates/settings` | `Tunables` and the override layer (below) |
| `crates/player`, `crates/engine` | the sound stage slots, AutoMix analysis and planner seams |
| clients | the declarative rows, the plugin manager, the install sheet, Bluetooth and sign-in page capabilities |

### Runtime

wasmtime with the component model.

- **Android, desktop, terminal:** Cranelift compiles a plugin once at install on the device; the result
  is cached, keyed by the plugin's hash, the wasmtime version and the CPU. Never download precompiled
  native code (Play forbids it, and it can't be verified like wasm).
- **Web:** the browser's own engine. Browsers don't run components directly, so the web host transpiles
  them with jco at install, or loads a core module build. The web host comes last.
- **iPod:** none.

Cranelift ships on every native client; its APK size is accepted. It compiles wasm SIMD (`simd128`),
which sound stages and song preparation need: a neural model such as Sing's comes near native speed
only with SIMD. wasmtime's Pulley interpreter is kept only for a platform where Cranelift can't run,
and there it hosts providers and device control (they wait on the network and Bluetooth), not sound
stages or song preparation.

## Performance rules

- **No plugin installed or enabled:** wasmtime is never created. The engine checks an `Option` once per
  burst for a sound stage and for song preparation. Same wakeups, CPU, allocations and start time as
  today (Tests).
- **Each seam checks before it builds:** the enabled plugins' manifests make one bitmask of what they
  use (events, deciders, slots, inputs). A seam checks its bit first; with it clear, nori takes its own
  path and builds no event payload, no decider inputs and no copy of its own answer. A decider with no
  plugin returns nori's answer as it is.
- **The checks sit where nori already decides:** on a settings change, a song change, a menu opening, a
  focus change; never per buffer or per frame.
- **Nothing exists ahead of use:** no listener, thread, surface, video player, receiver, cache folder or
  timer is made for a seam no plugin uses. Android receivers that would start nori for a plugin are
  declared disabled and enabled only while a plugin subscribes (Wake and commands).
- **An event can't start a clock nori doesn't run:** a subscription that needs one (lyric lines with
  the screen off) is a cost the plugin declares and the install sheet shows.
- **Instances are created when needed:** a provider when its profile opens, a device controller when its
  device connects, a hook plugin when an event it subscribed to fires. An idle plugin holds no memory
  beyond its cached compiled code.
- **Nothing ticks:** plugins wake on events they subscribed to, or on an answer to their own request.
  No timers while music plays with the screen off. A plugin that needs a schedule asks for one (a
  permission) and gets at most one wake per N minutes, never during screen-off playback.
- **The audio thread never calls a plugin that can block:** the sound stage instance has no imports.
- **Overrides cost nothing per buffer:** they are read where settings are derived (on a change), not on
  the audio path.
- **One OkHttp pool:** plugin HTTP goes through the core's `Transport`.
- **Limits:** each instance gets a memory cap and a time budget per call. A plugin that traps or runs
  over is stopped and the call fails as a network error would; after repeated failures it is disabled
  and the user told. A trap never takes nori down.

## Extension points

| point | the plugin exports | the host gives it | nori must add |
|---|---|---|---|
| **Rows** | its settings rows, actions, status | its stored values, change events | row rendering in every client |
| **Overrides** | override values for nori's settings and tunables | effective and stored settings, change events | `Tunables`, the override layer, the page notice |
| **Provider** | the `Provider` trait as WIT | HTTP to its domains, its store, a sign-in page | `WasmProvider: Provider` in `crates/core/src/providers/` |
| **Sound stage** | `configure`, `process`, `reset`, state region, latency | samples, inputs (volume, gain, route, song, its prepared data) | two slots in the chain, state keeping, policy |
| **Song preparation** | per-song data made from the samples ahead of playback | the decoded samples of the song playing and the next, as the engine reads them | a feed beside the engine's read-ahead (Song preparation) |
| **AutoMix analysis** | a streamed pass over a song, and/or refining nori's analysis | the PCM chunks nori's pass reads, nori's `TrackAnalysis` | a seam in the analysis pass, plugin data in the analysis store |
| **AutoMix planner** | a `TransitionPlan` for two songs, or none for nori's | both analyses (nori's and its own), the transition settings, nori's plan | a seam in `nori_automix::planner`, plan validation |
| **Device control** | which devices it handles; their state and commands | a Bluetooth channel to the device | the channel on Android and desktop, a section on the device page |
| **Hooks** | handlers for events | the session API, notices | an event bus in `crates/host` |
| **Lyrics service** | lookup | HTTP | a service slot in `crates/lyrics`'s race |
| **Scrobbler** | submit, now playing | HTTP | a sink beside the Subsonic scrobble in `crates/queue` |

### Deciders

Beyond these, every place where nori picks something is a decider, built like the AutoMix planner:
nori works out its own answer; enabled plugins, in order, get the inputs and nori's answer and may keep,
adjust or replace it; nori checks the result in Rust and keeps its own when the plugin's is invalid or
over its time budget. Nothing in the engine is patched, and every choice can be replaced.

| area | overrides | session API | deciders |
|---|---|---|---|
| queue | autofill size, skip after errors, previous restarts | add, remove, move, play, clear, shuffle, repeat | shuffle order, what autofill adds, what plays when the queue ends, radio and instant mix picks |
| transitions | AutoMix and crossfade values | | the transition plan, whether a pair mixes at all (the same-album rule) |
| sound | processor values, leveling | switch sound profile | leveling gain per song, sound profile per output route |
| audio path | | | sound stages; a decoder for a format nori can't play |
| prefetch, downloads | counts, tries, downloads at once | download, cancel | what is prefetched and downloaded next |
| library, search | page sizes, delays, freshness | browse, search, favorite, playlists | search ranking, smart playlist rules, mixes |
| lyrics | timeouts, trust | | services; transforming lyrics (translation, romanization) |
| covers, metadata | sizes, caches | | cover sources (a still or a loop, Artwork), tag fixes, artist info |
| now playing | | | what the notification, lock screen, MPRIS and Bluetooth (AVRCP) show for the song |
| media buttons | | | what a headset or Bluetooth button press does (play, skip, a plugin's action) |
| audio focus | | | what a focus loss, a transient loss and a duck do |
| look | | | the palette of a season of the plugin's own: the five roles, checked for contrast by nori-look; the accent keeps its one meaning |
| devices | | | Bluetooth control, per-device profiles, the AutoEQ pick |
| remote, jams | | invite, accept, remove a guest | what a guest may do, accepting requests |
| scrobbling | scrobble point | | where scrobbles go |

A decider runs where nori's own decision runs (a queue change, the queue running low, a replan, a song
change, a button press, a focus change), never per buffer. One on a command's path (skip, play, a
button) has a short budget; past it, nori's answer stands.

Audio focus is decided in Kotlin today (`RustPlayer.kt` `onFocus`); it moves to a pure function in
Rust first, as the boundaries ask anyway, and becomes a decider there. The look decider runs where
nori-look works out a page's colors (a cover or theme change); disabling the plugin brings back
nori's own seasons. With the `motion` and `coverColorsOn` overrides it is what an e-ink plugin needs.

**Casting** is plugins, through two seams:

- **Output sink:** the plugin takes nori's final audio instead of the device, so every sound setting,
  AutoMix and gapless stay. It reports its latency (the seek bar and lyrics shift by it) and takes
  volume. The audio goes out in real time, so the phone wakes far more often than with 10 s bursts.
  Its first implementations are the engine's own outputs (AudioTrack, cpal, the web), so the seam is
  not designed around one plugin.
- **Playback target:** nori hands the device songs (stream URLs, metadata) and commands, and the device
  plays them itself. nori's sound settings and AutoMix don't apply, but the phone can sleep. It sits
  beside remote control's devices, like moving playback to another nori.

What they call is under Host APIs. In both, the plugin only controls: audio bytes are moved by the
host natively, pumped to a socket or served through the existing LAN door
(`crates/remote/src/lan/door.rs`).

| plugin | seam | plugin size | notes |
|---|---|---|---|
| Snapcast | sink | ~0.5-1k lines | PCM to the user's snapserver (TCP source), its JSON-RPC for groups and volume; ~1 s buffer delay; fits the self-hosted stack |
| UPnP/DLNA | target, or sink through the LAN stream | ~2-3k | SSDP, device XML, SOAP AVTransport and RenderingControl, DIDL-Lite, GENA events; renderer quirks (formats, gapless through `SetNextAVTransportURI`) |
| Chromecast | target, or sink through the LAN stream | ~2-3k | Cast v2 without Google's SDK: protobuf over TLS on 8009, a heartbeat every 5 s while casting, the Default Media Receiver only (a custom receiver needs Google's registration) |

nori's side: the sink ~0.6-1k lines of Rust, the target ~0.8-1.2k, the capabilities ~1k, plus a device
entry in each client's output picker.

Later, if wanted: a transition stage that renders both decks itself.

### Out of reach

- The audio thread's timing: bursts, the 10 s buffer, the engine clock and its stall guards.
- The database and stored formats: a plugin has its own store and read-only queries, never the schema.
- Passwords, other plugins' stores, the permission system, installing and updating plugins.
- Provider songs (octo-fiesta's `ext-` ids) the user didn't ask for: streaming one makes the server
  download it from outside, so a plugin's autofill or decider may add them only where the user's "may
  add provider songs" switch lets nori's own autofill. A plugin acting on the user's tap (play this)
  is the user asking.

### Rows (plugin UI)

A plugin describes rows; each client draws them with its own components (`Design.kt` on Android), so a
plugin can't break Seasons and every client gets its page for free.

| row | what |
|---|---|
| switch, choice, flags, level | the existing `SettingKind`s, with the same `SettingSpec` shape (options, min, max, default) |
| text | one line: a key, a token, an address. Secret ones are masked and kept out of logs |
| action | a button; the plugin gets a call |
| status | a live value the plugin pushes (battery 80%, "ANC: on"); redrawn only while visible |
| section, note | a header; a line of plain text |

Rows can be greyed out or hidden by a condition on another row, as `settings_pages.rs` does for nori's
own. The plugin's values are kept by the host in the plugin's store, so a plugin that is not running
still shows its page.

Where rows appear: Settings › Plugins › *plugin*; a device controller's section on the device's page; a
provider's add form (the existing `FormField`s); and the slots below.

### Slots

Beyond its own pages, a plugin can fill **slots**: places nori defines in its own screens, which nori
draws with its own components from the plugin's data. A plugin never draws, never picks colors (the
accent stays "you, now") and never moves nori's layout. Every slot it fills is in its manifest and on
the install sheet.

| slot | the plugin gives | example |
|---|---|---|
| device card | known controls and rows (Device control) | headphone ANC and battery |
| artwork | for the player's sleeve: a still image or a muted looping video (Artwork) | moving covers, Spotify Canvas, larger covers |
| player control | a small card of its rows (a level, a status, an action) over the foot of the lyrics or the artwork, while the plugin says it is active | Sing's vocals level and "getting this song ready" |
| menu actions | a label and a glyph for the menus of songs (in lists, the queue and the player), albums, artists, playlists and a selection of several; which items it applies to; a call with the items when tapped, or a sheet of its rows for those items | "Find on Bandcamp", "Send to my other speaker", "Rate" |
| home shelf | a titled row of songs, albums, artists or playlists | "Concerts near you" |
| page | a titled list of songs, albums, artists or playlists, reached from Library | "Your year in music" |
| song info section | a few lines and links in a song's info | credits, the artist's tour dates |
| search section | a titled group of results | a store's matches |
| player status | one short line under the output row, only while the plugin's session is active | "Casting to Living room" |

- Menu actions join the menus the core already builds (`crates/library/src/menus.rs`: `song_menu`'s
  `SongAction`, `PageAction`) as one more action kind, after nori's own, so every client gets them
  where it opens that menu: the "..." button and a long press on Android, a right click on the desktop
  and the web, the action key in the terminal. The menu shows at once; a plugin's answer to "does this
  apply" comes from its manifest's filter (item kind, provider, local or not), not from a call. With no
  plugin action, `song_menu` appends an empty list and allocates nothing.
- A menu action can be bound to one of the plugin's switch rows: its label and glyph follow the row's
  stored value ("Sing", "Turn off Sing"), read by the host without calling the plugin, and tapping it
  flips the row.
- A menu action can open a sheet of the plugin's rows for the chosen items (a choice of 1 to 5 stars);
  the plugin gets the items and the values set.
- Glyphs come from nori's icon set, or a one-color SVG nori tints.
- Content is asked for when the slot comes into view and kept until it changes; the UI thread never
  waits for a plugin: a slot shows nothing until its answer arrives, then shows it.
- Each client draws the slots its screens have; a client without such a screen skips that slot.

v1: device card, artwork, player control, menu actions, page, home shelf. The rest later.

#### Artwork

A plugin gives the sleeve, for the song playing, a still image or a muted looping video (an HLS or MP4
URL). nori plays the video itself and owns every rule moving covers had:

- it plays only while the full player is open on the artwork, at rest, the app in front, the screen on
  and Motion not off; otherwise the still cover shows;
- it fades in only after its first frame is on the surface (and a moment after a move has ended), and
  fades out at a finger's first touch, when the sheet leaves fully open, and on a new album before its
  video is ready; flights draw the still cover. Nothing appears or leaves in one frame;
- mobile data only if the plugin's page allows it (Wi-Fi only by default);
- the bytes go through the one OkHttp pool into the plugin's cache (Storage), so a loop is fetched once;
- a video that answers an HTTP error is reported to the plugin, which can look it up again.

The plugin is asked when the song changes and the slot is in view, never ahead; it finds the video
itself (`net:` to its domains) and keeps its answers in its store. Android plays the loop with a muted
ExoPlayer of its own (no audio track, no focus, no session, no wake lock), made on first use and let go
of when the player is put away, the app is left or the screen goes off; `media3-exoplayer-hls` ships
for it. With no artwork plugin the sleeve draws the still cover as now: no player, surface or listener
is made. The desktop and the web play it with their own video (the web's `<video>`); the terminal and
the iPod have no such slot.

Words come from the plugin's `strings/`, the one exception to "the core returns no words". They are
short and plain like nori's own, but nori can't enforce that.

## Changing nori's values: overrides

### Three kinds of value

| kind | example | today | becomes |
|---|---|---|---|
| **Setting** | `crossfadeSec`, `songsAheadWifi`, `compressorPreset` | `StoredPrefs` field, on a settings page | overridable by key |
| **Tunable** | search delay 150 ms, 4 downloads at once, AutoMix max 12 s | a `const` in some crate | a `Tunables` field with default, range and effect bits, overridable |
| **Derived rule** | AutoMix sub-switches always on; Auto leveling = album in order, track in shuffle | a function over settings | stays one function; its fixed inputs become tunables |

`Tunables` is a struct in `crates/settings` declared like `StoredPrefs`, one line each:

```rust
#[tunable("search.delayMs", clamped(50, 2000), default = 150)]
pub search_delay_ms: i64,
#[tunable("automix.maxS", clamped(4, 30), default = 12, effect = REPLAN)]
pub automix_max_s: i32,
```

It is not stored anywhere: its values are the defaults unless an override says otherwise. The `const`
it replaces is read from it instead, once, where the value is used (copied into a processor at
`configure`, never read per sample).

### The override layer

The effective value of a key = the last enabled plugin (in the user's plugin order) that overrides it,
else the stored setting (for settings) or the default (for tunables).

- Overrides are kept in each plugin's own store, not in `StoredPrefs`. Disabling or removing a plugin
  removes its overrides; the user's settings were never changed.
- An override goes through the same codec and range as the setting (`clamped(...)`), so a plugin can
  set songs ahead to 20 but not to -1 or 10 000.
- Changing one applies through the same effect bits (`APPLY_AUDIO`, `APPLY_GAIN`, `REPLAN`, `SOUND`,
  `PLAYER`) as a setting change, so the engine splices it in seamlessly as it does now.
- The manifest lists the keys a plugin overrides, and it can override no other. The install sheet shows
  them ("changes: search delay, AutoMix lengths") and says which already-installed plugin overrides the
  same ones; both plugins' pages keep saying so while both are enabled.
- Overrides a plugin sets in one call apply as one change: the engine replans or splices once. A plugin
  may change them on events (car connected: crossfade 0).
- A choice can be overridden only with a value it offers; a plugin wanting another compressor preset
  overrides the compressor's numbers.
- Overrides are global. Sound settings already have a profile per output; a plugin that wants per-route
  values reacts to route changes itself.
- Keys are API: a key keeps its name when the code behind it moves; a removed key is ignored for one
  plugin API version, then refused. A plugin page lists keys it overrides that this nori doesn't know.
- A plugin reads both the stored and the effective values, and gets change events, so it can act on
  them: an AutoMix plugin does nothing while AutoMix is off, or forces its own length only when the
  user's crossfade is 0.
- Derived state that shows on a settings page (`offload_paused`, `sound_chain_on`, greyed rows) is
  computed from the effective values, so it stays true.
- A settings page with any overridden value shows one notice at its top: plugins change some of these
  values. Tapping it opens Settings › Plugins. Rows show the user's stored values.

### What becomes overridable

From the inventory of `master` at `2413016a`, with the settings dropped later that day (speed, pitch,
skip silence, previous always skips), whose code still runs with fixed values. **Expert** marks a value
that only makes sense together with others; it is still overridable, and nori documents it only in the
SDK.

**Between songs, AutoMix** (the removed AutoMix settings come back as tunables read by
`Settings::transition_prefs`):

| key | default | was |
|---|---|---|
| `automix.maxS` | 12 (`AUTO_MIX_MAX_S`) | `autoMixMaxS` |
| `automix.maxTempoPct` | 6 (`AUTO_MIX_MAX_TEMPO_PCT`) | `autoMixMaxTempoPct` |
| `automix.beatMatch`, `.keepPitch`, `.bassSwap`, `.filters`, `.echoOut` | on (hardcoded in `transition_prefs`) | the AutoMix sub-switches |
| `automix.beatsOnMobileData` | off | `autoMixBeatsMobileData` |
| `automix.measureAhead` | 3 (`player/src/queue.rs` `MEASURE_AHEAD`) | |
| `crossfade.curve` | equal power (`plan.rs`) | `crossfadeCurve` |
| `crossfade.inS`, `crossfade.outS` | follow `crossfadeSec` | `crossfadeInSec`, `crossfadeOutSec` |
| planner thresholds (expert) | `plan.rs`: skip, BPM confidence, fade bounds, bass cut, sweeps, key clash, vocal duck | |
| beat model reach (expert) | 30 s / 35 s (`beats.rs`) | |

**Leveling and sound** (most removed sound settings are values of processors that still run):

| key | default | was |
|---|---|---|
| `leveling.targetLufs` | from `Level` (-23/-18/-14) | `loudnessTarget` |
| `leveling.untaggedDb` | -6 (`gain.rs` `UNTAGGED_DB`) | `untaggedGainDb` |
| `leveling.boostMaxDb` | 6 (`BOOST_MAX_DB`) | close to `gainBoostDb` |
| `leveling.autoInOrder`, `leveling.autoShuffled` | album, track (`GainMode::Auto`) | |
| `eq.preampDb` | none = automatic (`auto_preamp_db`) | `preampDb` |
| `compressor.threshold/ratio/attack/release/makeup/knee` | the chosen preset's values | `comp*` |
| `headphone.crossfeedDb`, `headphone.crossfeedHz` | 4.5 dB, 700 Hz | `crossfeedDb`, `crossfeedHz` |
| `headphone.wideStrength` | 0.6 (`spatial.rs` `WIDE`) | `virtualizer` |
| `limiter.releaseMs`, `limiter.lookaheadMs` (expert) | 120, 5 | |
| `silence.*` (expert) | `silence.rs` threshold, minimum, kept | |
| `volume.boostMaxDb` | 12 | |

**Playback, queue, autofill:**

| key | default | was |
|---|---|---|
| `scrobble.atShare`, `scrobble.atMostS`, `.atLeastS` | half, 4 min, 10 s | `scrobblePercent` |
| `playback.speed`, `playback.pitch` | 1.0, 1.0 (the engine's `Settings`, `engine.rs`) | `speed`, `pitch` (dropped 2026-10-24) |
| `silence.skip` | off (the engine's `Settings`) | `skipSilence` (dropped 2026-10-24) |
| `queue.skipAfterErrors` | 3 in a row | `skipOnError` |
| `queue.previousAlwaysSkips` | off (the engine's `Settings`, `queue/src/rules.rs` `queue_previous_restarts`) | `previousAlwaysSkips` (dropped 2026-10-24) |
| `queue.previousRestartsAfterMs` | 3000 | |
| `prefetch.afterMs`, `prefetch.lookAt` | 6000, 8 | |
| `queue.releaseIdleMs` | 5 min | |
| `autofill.songs`, `.fillAhead`, `.recentDays`, `.forgetDays` | 15, 2, 14, 90 | |
| `mix.radio`, `.instantMix`, `.shuffleAll`, `.shuffleAlbums` | 50, 50, 200, 10 | |
| `history.halfLifeDays`, `.minHeardS` | 30, 2 | |

Speed and pitch are the session's as well as the sound's: a listening jam guest or a device following
another plays the leader's (`led_speed` in `engine.rs`), so an override on the leader reaches them as
session state, as speed did as a setting. A speed other than 1.0 or skipping silence turns offload off
through `nori_player::policy`, as before.

**Library, search, downloads, network, covers:**

| key | default | was |
|---|---|---|
| `search.delayMs` | 150 (`LIVE_SEARCH_DELAY_MS`) | `liveSearchDelayMs` |
| `search.songs/albums/artists`, `search.recentKept` | 40/20/10, 20 | |
| `browse.*` page and shelf sizes, `browse.few` | `browse.rs` | |
| `cache.freshS` per kind | 2 min / 1 h / 1 d (`cache_policy.rs`) | |
| `downloads.atOnce` | 4 (`DOWNLOADS_AT_ONCE`) | `parallelDownloads` |
| `downloads.tries` | 3 | |
| `covers.ahead` | = songs ahead | `coversAhead` |
| `covers.rowPx`, `.fullPx`, `.diskMb`, `.memoryShare` | 320, 800, 256, 0.15 | |
| `net.connectS`, `.readS`, `.streamReadS`, `.requests` | 10, 30, 240, 24/48 | |

**Lyrics:** per-request timeouts, services asked at once (6), the trust thresholds and when a miss is
asked again (`race.rs`, expert), auto-sync reach (`sync.rs`, expert).

**Screen:** dim delays and strengths behind `DimIdle`/`DimLevel`, the output rate caps behind `MaxRate`.

**Not overridable** (internal state or engine safety): the hidden settings (`servers`, keys, EQ bands,
home rows and other UI state: each has its own editor or is a secret), analysis and cache versions,
the burst buffer and read-ahead (10 s / 2 s), engine stall and clock constants, the effect bits.

`uiScale`, `reduceMotion` and the rest of the removed look settings have no Rust code left; their
replacements (`Motion`, `coverColorsOn`) are ordinary settings and overridable as such. Colors beyond
them go through the look decider (Deciders).

## Sound stage

### Where

The engine runs the chain in this order (`player/src/dsp.rs`, `chain.rs`): transitions → pre-amp,
equalizer, compressor, mono, virtualizer, crossfeed, balance, volume boost, limiter → silence skipping
→ speed. A plugin subscribes to one of two slots:

- **song:** before transitions; it sees one song at a time with that song's metadata and analysis;
- **master:** after the equalizer's tone and before the limiter, so the limiter still catches whatever
  a plugin boosts.

Several plugins in one slot run in the user's plugin order.

### How audio flows

At `configure(rate, channels)` the host allocates one block buffer inside the plugin's memory. Per
block (`MARK_FRAMES` = 8192 frames or less) the engine converts samples to f32 into it, calls
`process(frames)`, reads them back and goes on (dithering to 16-bit where the chain does). The plugin
edits them in place. Nothing allocates on the audio path; the `no_alloc` tests cover a stage.

### What the engine must add

- **State keeping.** Output is rendered up to 10 s ahead, and a sound change re-runs the chain from the
  last kept state (`Kept`, every `MARK_FRAMES`). A stage's state must be kept the same way: the plugin
  keeps all its state in one region it declares (the SDK does this), and the host copies that region
  into preallocated slots at each mark. `reset()` on seek and song change.
- **Inputs.** Some processors need more than samples. The stage declares which inputs it reads, and an
  input change is a sound change (spliced in as above):

  | input | for |
  |---|---|
  | volume (dB) | loudness compensation |
  | the song's gain and measured loudness, its tags | untagged gain, per-song processing |
  | output route and device kind (speaker, wired, Bluetooth, USB, car) | headphone-only processing |
  | sample rate, channels, float or 16-bit | |
  | its own row values | its parameters |
  | its own data for this song, from song preparation (read-only, mapped into its memory) | Sing's vocal mask, any precomputed processing |

  Prepared data that arrives while its song plays is an input change: the stage passes the song
  through until then, and the change is spliced in like any other.

- **Latency and tail.** A stage reports both. Latency is compensated in the position the player shows
  and in transition timing; a jam guest runs its own plugins, so sync is unaffected.
- **Policy.** An active stage counts as touching samples (`nori_player::policy`): offload and bit-perfect
  output turn off, as for nori's own processors.
- **Bypass.** The audio stage instance has no imports at all. A stage that traps or exceeds its share of
  the burst (a budget, e.g. its block in under a quarter of the block's play time) is bypassed for the
  rest of the song and flagged on its plugin page. Audio never waits for a plugin.
- **Analysis reads before plugins.** AutoMix and lyrics sync measure nori's decoded audio, never a
  plugin's output.

### Song preparation

Some processing needs the whole song, or seconds of it ahead, before it can play: a vocal mask from a
neural model, a precomputed filter. A preparation plugin is fed the decoded samples of the song playing
and the next, as the engine reads them ahead (no extra decode), on a low-priority thread of its own,
and returns its data for that song. nori keeps the data in the plugin's cache (Storage) and hands it to
the plugin's sound stage as an input.

- It runs only while the plugin says it is active (Sing switched on), and only for those two songs:
  never for the library, never for a provider song the user didn't ask to play.
- The engine copies samples into a preallocated feed it checks once per burst; with no preparation
  plugin active the feed doesn't exist. The copy never waits: a full feed drops the rest, and the
  plugin's data for that song comes later or not at all. The `no_alloc` tests cover the feed.
- The thread keeps the plugin's instance (and its model) for a while after the last feed, then lets
  it go. It works only while a song's data is missing and never on a timer: with the screen off it
  wakes with the read-ahead that feeds it and sleeps once the data is made.
- Its time budget is per song, not per block: it may take the song's length, and the stage plays the
  song unprocessed until the data is there.

## What nori removed, as plugins

Every feature nori dropped can come back as a plugin, through the seams above. nori ships none of them.

| feature | removed | how a plugin brings it back |
|---|---|---|
| Moving covers (Apple Music's motion artwork) | `60d04a8b` | the artwork slot with a loop; `net:` to Apple's iTunes Search and catalogue APIs (the old `crates/core/src/motion.rs` ports nearly unchanged); its answers per album in its store, the loops in its cache |
| Spotify Canvas (never built) | | the same slot per song, with `signin:` for the listener's Spotify session |
| Sing (vocals turned down) | `7a830087` | song preparation makes the mask (the Open-Unmix model as an asset), a song-slot stage applies it; a menu action bound to its switch; the player control slot for the vocals level and why a song still has its vocals |
| Speed, pitch, skip silence, previous always skips | 2026-10-24 | overrides (`playback.speed`, `playback.pitch`, `silence.skip`, `queue.previousAlwaysSkips`); a player control or a menu action to change them |
| 1-5 star ratings | `2175dddc` | `account.subsonic` for `setRating`; a menu action opening a sheet with the stars; the song info section for the rating shown. `userRating` is still in the library data, smart playlists and mixes |
| Removed look settings (e-ink, colors per screen) | `6d83364d` | the look decider for the palette, with the `motion` and `coverColorsOn` overrides |
| Removed sound knobs | `6d83364d` | below |

The removed sound code is at `6d83364d^` and ports to the SDK nearly unchanged:

- **Loudness compensation (ISO 226):** `player/src/contour.rs` designs shelves from the reference level
  and the volume. As a master-slot stage: reads the volume input, a row for the reference (70-90 phon),
  re-designs its shelves when either changes, runs them as biquads.
- **Expander / noise gate:** `compressor.rs`'s `Expander`, a master-slot stage with its threshold,
  ratio, attack and release as rows.
- **Bass boost:** an EQ preset today; a plugin can also override the EQ.
- Preamp, gain boost, untagged gain, crossfeed and compressor values, virtualizer strength: overrides
  (above), no sound stage needed.

## AutoMix seams

- **Overrides:** every AutoMix value above.
- **Analysis.** A plugin gets the same PCM chunks nori's one streamed analysis pass reads, in the same
  pass (no extra decode), and returns its own data, stored in the analysis store under the song, the
  plugin and the plugin's analysis version. Or it refines nori's `TrackAnalysis` (beat grid, cues) after
  the pass. Analysis runs on the lowest-priority thread, once per song.
- **Planner.** `plan(out, in, durations, settings) -> TransitionPlan` is already pure and its types
  serialize (`serde`). A planner plugin gets both songs' analyses (nori's and its own), the effective
  transition settings and nori's own plan, and returns a plan or none (nori's plan plays). The engine
  validates every plan in Rust (positions inside the songs, lengths within bounds, fades in order) and
  falls back to nori's on a bad one. The mixer renders the plan; the plugin does not touch the audio.

## Providers

`WasmProvider` implements `Provider` by calling the plugin, so nothing else in the core or a client
changes (docs/developers/providers.md).

- Its add form is its own: `info` returns the fields adding a profile asks for (`FormField`: address,
  user, key; plain or advanced) and how it signs in, and every client draws that form as it does for
  the built-in providers.
- The plugin resolves where a song streams from (a URL, fetched as it is); the engine fetches, caches and
  decodes the bytes natively, as now.
- HTTP only to the manifest's domains, through `Transport`.
- Sign-in: a "sign-in page" capability. The client opens its visible sign-in WebView at the plugin's
  start URL and hands back the cookies for the plugin's domains (what YouTube's built-in sign-in does
  now on Android).
- The core's rules still hold over every provider: never queue or prefetch what the user didn't ask
  for from a provider that downloads on stream, provider items never indexed.
- **YouTube Music** moves out as the first provider plugin: `innertube-music` is already its own crate
  and pure Rust. The built-in `youtube` feature stays until the plugin is on par. An existing YouTube
  profile then offers to install the plugin; a jam guest asked to play a `yt-` song without it is told
  which plugin it needs.

## Device control

- The plugin says which devices it handles (Bluetooth name pattern, service UUIDs, vendor ids).
- The host gives a byte channel to the connected device: RFCOMM/SPP by UUID or BLE GATT
  (characteristics, notifications). Android: `BluetoothSocket`, `BluetoothGatt`, the
  `BLUETOOTH_CONNECT` permission asked when the first such plugin is installed. Desktop: BlueZ on Linux
  (`bluer`), IOBluetooth/CoreBluetooth on macOS. The web: Web Bluetooth, GATT only, Chromium only.
- The protocol logic (packets per brand) lives in the plugin. Prior art: OpenSCQ30 (Soundcore, Rust),
  Gadgetbridge, librepods (AirPods).
- What it reports comes in two parts: **known controls** nori defines and draws the same for every
  brand (battery per bud and case, noise control as an enum: off, noise canceling, transparency,
  adaptive, of which the device offers some), and free **rows** for anything brand-specific (wear
  detection, touch actions, multipoint).
- Where they show, on Android (other clients put the same in their devices panel):
  - **the devices sheet** (`RemoteScreens.kt` `DevicesSheet`, opened by the player's output button):
    a card at its top for the headphones playing now, with the battery and noise control as one
    segmented row, and a way on to the device's page. While a plugin controls the current output, the
    output button opens this sheet even with remote control off; the system output switcher is one
    row of it, as it is with remote control on;
  - **the player's output row**: the battery beside the device's name (`WH-1000XM5 · 80%`), from the
    known controls; nori's UI, not the plugin's;
  - **the device's page**: everything, known controls and rows, under its sound profile in the
    equalizer's device list (`DevicesSection.kt`), and in Settings › Plugins.
  All of it only while the device is connected.
- The instance lives while the device is connected; it is woken by the device's notifications, not by
  polling.

## Hooks

Events a plugin can subscribe to: playback started or paused, song changed, queue changed, favorite
changed, profile opened, output route changed, device connected, settings changed (stored or
effective), a media button pressed, a lyric line reached. It answers with session API calls (add to
queue, set a setting, show a notice), each behind its permission.

An event is built only when an enabled plugin subscribed to it. Lyric lines need a wake per line,
which nori never has with the screen off; a plugin subscribing to them then (lyrics over Bluetooth,
through the now-playing decider) brings that wake, and its install sheet says so.

### Wake and commands

- **Wake:** a plugin can ask to start nori when an event happens while nori is closed (headphones
  connected: autoplay). Android declares the receivers for these disabled and enables one only while
  an enabled plugin subscribes; while disabled, Android never starts nori for it. The desktop has no
  such wake.
- **Commands:** a plugin declares named commands other apps may run (Tasker-style automation).
  Android receives them as an intent behind a permission of nori's own, on a receiver enabled only
  while such a plugin is; Linux over D-Bus, registered only then. A command is handed to its
  plugin like an action row's call.

## Host APIs

What a plugin can call, as WIT interfaces. Standard WASI where one fits (so authors can use ordinary
crates), nori's own where the platform needs nori in the middle. Each is behind a permission.

| interface | what | notes |
|---|---|---|
| `wasi:http` outgoing | HTTP requests | served by the core's `Transport` (one OkHttp pool on Android), limited to the manifest's domains, or to LAN addresses with `lan` |
| `wasi:sockets` | TCP connect, UDP bind/send/receive, multicast join | `lan` limits them to private addresses. Android: the host holds a `MulticastLock` while a group is joined |
| `nori:plugin/tls` | a TLS stream over a TCP socket | rustls in the host, so plugins needn't carry it. Verification: system roots, a pinned certificate, or none for a LAN address the plugin discovered (Chromecast's self-signed certificate) |
| `nori:plugin/discovery` | browse mDNS service types (`_googlecast._tcp`, `_snapcast._tcp`) | Android NSD, the desktop's mDNS; runs only while the plugin asks (an output picker open) |
| `nori:plugin/lan-serve` | HTTP endpoints under the LAN door, `/p/<plugin>/...` | UPnP event callbacks (GENA); the requests come to the plugin |
| `nori:plugin/audio-out` | nori's final audio, encoded (PCM, WAV, FLAC), pumped by the host to a socket the plugin opened, or served at a LAN URL | the plugin sets the stream up; the host moves every byte natively, at real time, so no plugin code runs per buffer |
| `nori:plugin/media` | a song's stream URL and cover URL reachable from another device on the LAN (the server's signed URL, or nori's LAN door proxying it) | for playback targets |
| `nori:plugin/timers` | one-shot and repeating wakes | while the plugin has an active session (a cast, a connected device) as often as it needs; otherwise only with `schedule`, and never during screen-off playback |
| `nori:plugin/bluetooth` | connected devices (name, an id, service UUIDs, vendor and product ids); RFCOMM by UUID; BLE GATT (services, read, write, notify, MTU); connect and disconnect events | Android: `BluetoothSocket`, `BluetoothGatt`, `BLUETOOTH_CONNECT`. Linux: BlueZ (`bluer`). macOS: IOBluetooth, CoreBluetooth. Web: Web Bluetooth (GATT only) |
| `nori:plugin/session` | queue, playback, library, settings and overrides, notices | `crates/host`'s session API |
| `nori:plugin/events` | subscriptions | |
| `nori:plugin/store`, `log`, `rows` | its own data, the log, its pages | |
| `nori:plugin/cache`, `assets` | its cache and its declared downloads (Storage) | |
| `nori:plugin/sign-in` | the visible sign-in page and its cookies | Android WebView, the desktop's sign-in window |

### Storage

| tier | what | size | backed up |
|---|---|---|---|
| store | rows, overrides, small data (answers per album, cookies) | small, capped | yes |
| cache | bytes it can fetch or make again (video loops, prepared song data); least recently used go first | the quota in its manifest, shown on the install sheet | no |
| assets | large files it names in its manifest (a model of tens of MB): URL, size, sha256 | as declared, shown before the download | no; fetched again after a restore |

nori fetches assets itself through `Transport`, on Wi-Fi unless the user allows mobile data, and shows
the progress as a status row on the plugin's page; the plugin is told when one is in. A plugin's cache
and assets live in a folder made on first use and removed with the plugin.

### What each needs

| plugin | uses |
|---|---|
| Snapcast (sink) | discovery (`_snapcast._tcp`), a TCP socket to the snapserver's TCP source, `audio-out` pumping PCM into it, the JSON-RPC (TCP 1705) for groups and volume |
| UPnP/DLNA (target, or sink) | UDP multicast for SSDP (M-SEARCH to 239.255.255.250:1900, NOTIFY), `wasi:http` to the LAN for device descriptions and SOAP (AVTransport, RenderingControl), `lan-serve` for GENA events, `media` URLs (target) or `audio-out` served at a LAN URL (sink), `timers` for the position while casting |
| Chromecast (target, or sink) | discovery (`_googlecast._tcp`), TCP + TLS without verification to port 8009, then length-prefixed protobuf `CastMessage`s the plugin encodes itself (connection, heartbeat, receiver, media namespaces), `timers` for the heartbeat (every 5 s while casting), `media` or `audio-out` URLs for the Default Media Receiver to fetch |
| headphone ANC | `bluetooth`: the connected device's ids to know it, RFCOMM (Soundcore, Sony and others) or GATT, notifications for state and battery; rows for the device's section |

### Platform glue the host adds for casting

A cast is a session the host owns: the plugin calls `cast.start(kind)` and `cast.end()`, and the host
takes and releases everything the platform needs. A plugin can't hold any of it outside a cast; a
trap, a stopped plugin or a lost device ends the cast and releases it all. Not casting, none of it
exists.

| | when | cost |
|---|---|---|
| Wi-Fi lock | only a sink cast (nori streams its own audio from the phone) | keeps the radio out of power save while streaming; the streaming costs more than the lock |
| media3 session showing the cast | every cast | none: the same session and notification, another state |
| volume keys to the device (`VolumeProvider`) | every cast | none: called on a key press |
| foreground service with no AudioTrack | a target cast | none beyond the notification |

A target cast lets the phone sleep: the device fetches the music itself. What still wakes it is the
plugin's own control: Chromecast's heartbeat every 5 s on its socket, UPnP's events. UPnP's position
is asked only while a screen shows it (`timers` has a "while visible" kind, like the seek bar's).

The output picker of every client lists the plugin's devices beside remote control's.

### What a plugin can't do on a platform

- AirPods' controls use classic L2CAP (AAP), which stock Android doesn't let apps open (librepods
  needs root there). Linux can; Android can't. Such a plugin lists the capability it lacks and isn't
  offered on Android.
- The web has no raw sockets, multicast or classic Bluetooth: casting plugins can't work there;
  GATT-only headphone plugins can, in Chromium.

### Compatibility

A component lists every interface it imports, so the host reads what a plugin needs from the wasm
itself, not only from its manifest. The manifest marks some of them optional.

- A required interface a client lacks: the plugin is listed but can't be installed there, and says
  which (on the web: "needs Bluetooth Classic and LAN sockets").
- An optional one: the host links a stand-in whose every call returns `unsupported`, and the plugin
  works without it (a headphone plugin that uses RFCOMM where it can and GATT on the web).

To check: whether the Default Media Receiver plays plain HTTP media on the LAN (nori's LAN door is
HTTP).

## Permissions

| permission | grants |
|---|---|
| `net:<domains>` | HTTP to those domains |
| `lan` | HTTP, sockets, multicast and TLS to private addresses; mDNS discovery; LAN door endpoints |
| `audio.out` | nori's final audio as a stream (casting) |
| `media.urls` | songs' stream and cover URLs for another device |
| `settings.read` | stored and effective settings, change events |
| `settings.override:<keys>` | overriding those keys |
| `playback.read`, `playback.control` | now playing and the queue; changing them |
| `library.read`, `library.write` | browsing; favorites and playlists |
| `account.subsonic` | the profile's Subsonic API through the core (never the password itself) |
| `audio.process` | a sound stage, song preparation |
| `automix.analyze`, `automix.plan` | the AutoMix seams |
| `artwork` | the player's artwork slot |
| `now-playing`, `media-buttons`, `audio-focus`, `look` | those deciders |
| `wake:<events>` | starting nori on those events while it is closed |
| `commands` | other apps running its declared commands |
| `bluetooth:<filter>` | a channel to matching devices |
| `signin:<domains>` | the sign-in page |
| `notices` | showing a notice |
| `schedule` | a periodic wake (limited, never in screen-off playback) |

Every plugin has its own store (its rows, overrides, data such as cookies), invisible to other plugins.
Install lists every permission and override key; the user allows all of them or cancels. An update that
asks for more asks again.

## Repos and installing

- A repo is a static `index.json`: per plugin its id, name, versions, plugin API version, needed
  capabilities, permissions, sha256, download URL and the author's signature (ed25519).
- nori's own repo on GitHub is preloaded; the user adds others by URL or QR code. Self-hosted only: a
  repo is a file anyone can host, no nori service.
- The author's key is pinned at first install; an update signed by another key is refused unless the
  user removes and reinstalls.
- Updates: per plugin, automatic (default) or held at its version.
- Sideloading a `.noriplugin` file works the same way, with the same sheet.
- Desktop and terminal load a plugin from a folder and reload it on change, for development.
- A client says which plugins it can't run, and why (Compatibility).
- F-Droid, to check: whether a preloaded repo listing network-service plugins earns an anti-feature
  flag (`NonFreeNet`). If it does, the F-Droid build preloads no repo.

## Backup, failures and bug reports

- A backup holds the installed plugins (id, version, repo, order, enabled), their stores (secrets
  included) and their overrides, encrypted with the password set when the backup is made. Restoring
  reinstalls them from their repos. nori has no settings backup yet (only the sound settings' import),
  so the backup itself is a feature of its own, needed before this.
- A plugin's failure is its own: a trap or an overrun fails that call (above).
- Safe mode. Traps never crash nori; what can is host code running for a plugin (a capability's glue,
  memory). The host marks which plugin it is calling (one atomic store per call), and the crash report
  records it. The next start disables that plugin and says why; a startup crash with no plugin marked
  starts once with every plugin off.
- Bug reports and the self test list the enabled plugins and their overrides. One switch turns every
  plugin off, to tell a nori bug from a plugin's.
- No plugin depends on another in v1.

## Work per client

| client | work |
|---|---|
| Android | row rendering with `Design.kt` shapes, Settings › Plugins, install sheet, page notice, device page section, Bluetooth channel, sign-in page handover, the artwork loop (muted ExoPlayer, HLS), the player control slot, the disabled wake and command receivers, audio focus moved to Rust |
| desktop | rows in Slint, the same pages, Bluetooth channel (Linux, macOS), the artwork loop (its video player to choose), the player control slot, commands over D-Bus on Linux |
| terminal | rows in its settings view, the same pages, Bluetooth channel on Linux |
| web | the jco host, rows in Svelte, Web Bluetooth, the artwork loop (`<video>`), the player control slot |
| iPod | none |

## Size estimate (to be measured in phase 0)

| part | estimate |
|---|---|
| wasmtime runtime + component model | ~1-3 MB per Android ABI (accepted) |
| Cranelift | several MB more (accepted) |
| `media3-exoplayer-hls` | its size, loaded only when a loop plays (accepted) |
| `crates/plugins` | 4-6k lines of Rust |
| `crates/plugin-sdk` | 1-2k lines |
| `Tunables` + override layer, consts moved | ~1k lines, mostly one line per value |
| engine: sound stage slots, state keeping, AutoMix seams | ~1-1.5k lines |
| clients | Android ~1k Kotlin, desktop ~600, terminal ~400, web ~400 |
| Bluetooth | Android ~300 Kotlin, desktop ~400 Rust |

## Tests

- Test plugins (Rust, built to wasm in CI) for each extension point, in `crates/plugins/tests`.
- A pipeline test on the virtual clock with a sound stage: state keeping across a sound change
  (identical output after a splice), bypass on a trap and on an overrun, `no_alloc`.
- An override test per kind: effective value, range clamp, effect bits applied, removal restores.
- A planner plugin's bad plan falls back to nori's.
- With the `plugins` feature on and no plugin installed: `perf-host` equals master, the `no_alloc` tests
  pass unchanged, and Android's cold start stays within a fixed budget of master's.
- Song preparation: the feed doesn't exist with no active plugin; with one, `no_alloc` holds and a full
  feed drops samples instead of waiting.
- A decider with no plugin gets nori's answer back without its inputs being built.
- Device checks only for the Bluetooth channel, the sign-in handover, the artwork loop's surface and the
  wake and command receivers (docs/developers/testing.md).

## Phases

0. Spike: wasmtime with Cranelift on Android arm64: compile time at install, cold start with wasmtime
   linked, a biquad stage's CPU per minute against native, and a neural model of Sing's size with wasm
   SIMD against native.
1. Host, rows, slots, repos and install, storage, `Tunables` and overrides (with speed, pitch, skip
   silence and previous), hooks, the first deciders (queue, now playing, media buttons). Sample plugins
   live in the SDK's repo, as tests of the API; moving covers among them, for the artwork slot.
2. Provider seam; YouTube Music as a plugin; the built-in one removed when on par.
3. Sound stage and song preparation. Loudness compensation, the expander and Sing as sample plugins
   in the SDK repo (testing the API, not shipped in nori's repo unless decided). Audio focus moves to
   Rust and becomes a decider.
4. AutoMix analysis and planner seams.
5. Device control and Bluetooth; a Soundcore or Sony plugin as the first.
6. Lyrics service and scrobbler seams; the lyric line event; wake and commands; the look decider.
7. The web host.
8. Output sink and playback target; Snapcast as the first sink, UPnP/DLNA as the first target.

Settings backup (with its password) is its own feature; plugins join it when both exist.

## Open questions

- The desktop's video player for artwork loops (Slint draws no video).
- Whether wasm SIMD is fast enough for Sing's model on a phone, or a host interface that runs a model
  natively is needed (phase 0).
