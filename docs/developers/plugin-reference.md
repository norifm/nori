# Plugin reference

This page lists the manifest, rows, strings, the package, the extension points, the host interfaces,
permissions, limits and repos of plugin API 7. For what a plugin is, see [Plugins](plugins.md).

The API is the WIT in `wit/`. `types.wit`, `host.wit` and `net.wit` hold what a plugin calls.
`plugin.wit` holds what it exports, and the `plugin` world. The SDK, `crates/plugin-sdk`, is that WIT as
Rust. WIT names become snake case, and a Rust keyword gets a trailing `_`: `settings::override_`,
`session::move_`.

## Manifest

The manifest is a `nori_plugin_sdk::Manifest` constant, passed to `plugin!`. Start from
`Manifest::new(id)`:

```rust
const MANIFEST: Manifest = Manifest {
    net: &["api.example.com", "*.cdn.example.com"],
    rows: &[Row::switch("enabled", true)],
    ..Manifest::new("com.example.plugin")
};
nori_plugin_sdk::plugin!(MyPlugin, MANIFEST, lifecycle, hooks);
```

| Field | Type | Default | What it declares |
|---|---|---|---|
| `id` | `&str` | Required | The plugin's id, in reverse DNS: two or more parts of letters, digits, `-` and `_` |
| `stage` | `Option<Slot>` | `None` | A sound stage's slot: `Slot::Master` or `Slot::Song` |
| `net` | `&[&str]` | Empty | Hosts it may reach over HTTP and sockets, with `*.` and `lan` |
| `signin` | `&[&str]` | Empty | Hosts whose cookies a provider's sign-in page hands back |
| `overrides` | `&[&str]` | Empty | nori's values it may override, by key |
| `rows` | `&[Row]` | Empty | Its page's rows, in order |
| `menu` | `&[MenuAction]` | Empty | Actions in the menus of songs, albums, artists or playlists |
| `places` | `&[Place]` | Empty | Places in nori's screens it fills |
| `devices` | `&[DeviceFilter]` | Empty | Bluetooth devices it reaches and may control |
| `permissions` | `&[Permission]` | Empty | What it may do with the session beyond its own data |
| `hooks` | `&[Hook]` | Empty | The events its `hooks` export hears |
| `wake` | `&[Wake]` | Empty | What may start nori for it while nori is closed |
| `commands` | `&[&str]` | Empty | Commands other apps may run, each reaching `lifecycle.action` |
| `cache_mb` | `u32` | 0 | Its cache's quota, in MB |
| `assets` | `&[Asset]` | Empty | Large files nori downloads for it |
| `category` | `Category` | `Other` | The page of Get plugins it is listed on |

`category` is `Category::Service` for a music service to play from, `Device` for headphones, speakers and
other devices, `Playback` for how music plays and sounds, and `Other` for the rest. Get plugins shows a
page per category, from the `category` its repo's index repeats. A plugin built before categories is
`Other`.

In `net`, a leading `*.` takes in a host's subdomains, and `lan` opens private addresses.

`plugin!` writes the manifest into the wasm's `nori:plugin` custom section. It adds the version from the
crate's `Cargo.toml`, the API version, and the extension points named after the manifest. nori calls
only those points. nori reads which host interfaces a plugin calls from the component's imports, so the
manifest does not list them.

### Menu actions, places and device filters

- `MenuAction { id, glyph, kinds }` adds an action to the menus of the item kinds in `kinds`
  (`ItemKind::Song`, `Album`, `Artist`, `Playlist`). `id` is its label's key in the strings. Running it
  calls `slots.menu`.
- `Place::Artwork` fills the player's sleeve, `Place::SongInfo` adds lines to a song's info, and
  `Place::PlayerStatus` adds a line under the player's output row. Each calls its `slots` function.
- `Place::Devices` shows the plugin's rows on the devices sheet (**Playing on**) too. It calls nothing.
- `DeviceFilter::ANY.name("WH-1000XM*").service(uuid).vendor(id)` matches a device when every part it
  gives matches. In a name, `*` stands for any run of characters, and case does not count.

### Assets

`Asset { name, url, size, sha256 }` names a large file. `name` holds letters, digits, `-`, `_` and `.`,
and does not start with `.`. `sha256` is 64 lowercase hex digits. nori downloads the file on an
unmetered network, or on mobile data once the user allows it on the plugin's page. The plugin reads it
with `assets.read` once its `asset-ready` event comes.

## Rows

Rows are what nori draws on the plugin's page. A row's `id` is its label's key in the strings, and the
key nori keeps its value under. nori keeps the values, so the page draws while the plugin is not running.

| Kind | SDK constructor | Its value | A sound stage reads |
|---|---|---|---|
| Switch | `Row::switch(id, default)` | `Value::Switch` | 0 or 1 |
| Choice | `Row::choice(id, options, default)` | `Value::Choice`, the option's index | The index |
| Segments | `Row::segments(id, options, default)` | `Value::Choice`, two to four options side by side | The index |
| Flags | `Row::flags(id, options, default)` | `Value::Flags`, one bit per option that is on | Nothing |
| Level | `Row::level(id, min, max, step, default)` | `Value::Level`, a slider's number | The number |
| Text | `Row::new(id, RowKind::Text)` | `Value::Text` | Nothing |
| Secret | `Row::new(id, RowKind::Secret)` | `Value::Text`, masked and kept out of logs | Nothing |
| Action | `Row::new(id, RowKind::Action)` | None: a tap calls `lifecycle.action(id)` | Nothing |
| Status | `Row::new(id, RowKind::Status)` | A line the plugin sets with `rows.set-status` | Nothing |
| Meter | `Row::meter(id, min, max)` | A bar the plugin sets with `rows.set` | Nothing |
| Section | `Row::new(id, RowKind::Section)` | None: a heading | Nothing |
| Note | `Row::new(id, RowKind::Note)` | None: plain words | Nothing |
| Link | `Row::link(id, url)` | None: opens `url` | Nothing |
| Page | `Row::page(id)` | None: opens the rows placed on it | Nothing |

A sound stage reads its rows' numbers in manifest order, in `Inputs::rows`. Only switches, choices,
segments and levels take a place in that list.

These modifiers change a row:

- `.hint(key)` puts a quiet line under the label.
- `.unit(key)` puts a unit after a level's or a meter's value.
- `.on(page)` puts the row on the page that page row `page` opens. A client that draws one long page
  (the desktop app, the terminal app, the web client) puts each page's rows under its name.
- `.glyph(glyph)` puts one of nori's glyphs before the label.

A level shows as many decimals as its `step` has.

The glyphs are `Sparkle`, `Info`, `Share`, `Link`, `Search`, `Heart`, `Download`, `Add`, `Play`, `Tune`,
`Headphones`, `Speaker`, `Battery`, `Touch`, `Timer`, `Gauge`, `Wave`, `Bluetooth`, `Gear`, `Globe`,
`Key` and `Person`. Each client draws them in its own style.

## Strings

A plugin's words live in `strings/<lang>.toml`, one file per language, such as `strings/de.toml`. Each
line is `key = "text"`, a TOML basic string. Other lines are skipped.

| Key | Holds |
|---|---|
| `name` | The plugin's name in lists and on its page |
| `about` | One line about the plugin |
| A row's `id` | The row's label |
| A choice's, segments' or flags' option | That option's label |
| A `hint`, `unit` or menu action's key | That text |
| Any other key | Words for `notices.show` and `rows.set-status`, with `{0}`, `{1}` for their arguments |

nori reads the user's language over English, so a key missing in `de.toml` comes from `en.toml`. A key
missing in both shows as the key itself. The web client reads only `en.toml`. nori-plugins' release
tool reads `name` and `about` from `en.toml` for the repo's index.

## Package

A package is a `.noriplugin` file, which is a zip, or a folder with the same files:

| File | Required | What it is |
|---|---|---|
| `plugin.wasm` | Yes | The component, built for `wasm32-wasip2` |
| `strings/<lang>.toml` | No | The plugin's words, one file per language |
| `icon.png` | No | A PNG, 256 px square, one color on transparent, at most 256 KiB |
| `README.md` | No | What its page shows listeners |

nori ignores other files. Build with `cargo build --release --target wasm32-wasip2` and the crate type
`cdylib`. A plugin's tests can run on the computer's own target, but the `cdylib` does not link there.

Each client draws the icon in the page's ink, so its own color does not show. The plugin's page shows
the README as headings, paragraphs, list items and code blocks. It leaves out the first heading,
because the page already shows the name. It drops bold, italics and inline code marks, and keeps only a
link's words. Tables are not supported.

## Install checks

nori refuses a package, and changes nothing, when one of these holds:

| Refusal | When |
|---|---|
| `Unreadable` | The package, its `plugin.wasm` or its `nori:plugin` section can't be read |
| `Unreadable` | A sound stage has no slot, or an asset has a bad name or sha256 |
| `Api` | It was built for another plugin API |
| `Needs` | It calls host interfaces that this nori does not link |
| `Id` | Its id is not reverse DNS |
| `UnknownKey` | It overrides a key that nori does not have |
| `Signature` | Its repo's key and signature do not match the package |
| `OtherKey` | It is not signed by the key the installed plugin pinned |

## Extension points

Each point is a trait of the SDK, named in `plugin!` by its snake-case name. A method the plugin does not
write gives nori's own answer: `None`, "unsupported", empty, or what it was handed.

| Point | Trait | Its calls |
|---|---|---|
| `lifecycle` | `Lifecycle` | `start`, `changed`, `action` |
| `hooks` | `Hooks` | `on`, `served` |
| `provider` | `Provider` | The add form, reads, writes, streams, sign-in and account (see [Providers](providers.md)) |
| `sound-stage` | `SoundStage` | `configure`, `process`, `latency`, `reset`, `save`, `restore` |
| `preparation` | `Preparation` | `begin`, `feed`, `finish` |
| `automix` | `Automix` | `begin-analysis`, `feed-analysis`, `finish-analysis`, `refine`, `plan` |
| `deciders` | `Deciders` | One call for each decider below |
| `slots` | `Slots` | `artwork`, `artwork-failed`, `menu`, `song-info`, `player-status` |
| `device-control` | `DeviceControl` | `handles`, `state`, `set-noise`, `gone` |
| `lyrics` | `Lyrics` | `lookup` |
| `scrobbler` | `Scrobbler` | `now-playing`, `scrobble` |
| `notification` | `Notification` | `buttons` |

`slots` also declares `shelf`, `page` and `search`. nori does not call them.

`lifecycle.start` runs when the plugin is turned on, before any other call. `lifecycle.action` runs for
an action row, a command, a cast target picked (`cast:<id>`), and a notification button of the
plugin's own.

### Sound stages

A stage runs in one of two slots. `Slot::Master` is after the equalizer and before nori's ceiling, a soft
limit that keeps a boost from clipping. `Slot::Song` runs on each song as the player reads it, before
transitions, so each side of a mix has its own.

- A block holds at most 8192 frames, channel after channel. `process` must hand back the same number of
  samples.
- `save` returns everything that changes what the stage makes next. nori saves it at marks and restores
  it when it makes music again, such as after a sound change. The SDK's `state::save` and
  `state::restore` do this for a plain `Copy` struct with no padding.
- `latency` returns the frames the stage delays the sound by, and the frames of tail it makes after its
  input ends.
- `Inputs` holds the rows' numbers, the volume in dB, the route, and the song's gain, loudness and
  prepared data when nori knows them. A change calls `configure` again.
- A stage touches the samples, so songs stay off offload while it is on. Bit-perfect output leaves it
  out.

### Deciders

nori works out its own answer first. Then each enabled plugin with `deciders` gets the inputs and the
answer so far, in the user's order, and returns another answer or `None`. nori checks each answer before
it uses it or hands it on. A refused answer, a failed call or one past its budget keeps the answer so far.

| Decider | When nori asks | Budget | What nori checks |
|---|---:|---:|---|
| `shuffle` | Shuffle turned on, or a queue set shuffled | 50 ms | Every index once, starting with the song playing |
| `autofill` | After nori's own refill | Work only | Each id is a song, not queued, and a provider song only where allowed |
| `queue-ended` | The queue played out | Work only | As for `autofill` |
| `leveling` | Once per song and gain, while volume leveling is on | 250 ms | A finite gain from -60 dB to nori's boost ceiling |
| `sound-profile` | An output arrives | 250 ms | A saved sound profile, or none |
| `now-playing` | Once per song, for the notification and the media controls | 250 ms | A title |
| `media-button` | A media button press | 50 ms | One of the enum's acts |
| `audio-focus` | An audio focus change | 50 ms | An act that fits the focus state |
| `look` | The season's palette, with no cover | 250 ms | Readable contrast |
| `mixes` | Whether two songs mix, given nori's same-album rule | 100 ms | None: false plays them gapless |

"Work only" means the call has its fuel and no time limit.

### AutoMix

`plan` and `refine` each have 100 ms. The first plugin with a plan gives it. nori keeps a plan only when
all of these hold:

- Its positions are inside both songs.
- Its length is 20 ms to 60 s.
- A tempo change is beat-matched and within the user's limit.
- An echo is where nori planned one.

nori keeps a refined analysis only when its tempo is 40 to 250 BPM or none. Its cues must be inside the
song and in order. Otherwise nori's own plan or analysis stands.

### Notification buttons

`buttons` gets nori's own buttons and what plays, and returns its own list, or `None` for nori's. It may
keep, reorder or leave out nori's buttons, and add its own, each with a glyph and a label from its
strings. A press of its own button calls `lifecycle.action(id)`. Only Android has these buttons.

## Host interfaces

A plugin may call each interface its component imports. A call that lacks what the table names fails
with `refused`, and nori logs why.

| Interface | What it gives | What it needs |
|---|---|---|
| `log` | Lines in nori's log, as `plugin <id>: <line>` | Nothing |
| `store` | Small values by key, backed up with the settings | Nothing |
| `cache` | Bytes it can fetch again, the least recently used going first | `cache_mb` |
| `assets` | The state and bytes of its manifest's assets | `assets` |
| `http` | Requests with any method, headers and body, one at a time or side by side | `net` |
| `notices` | A short line shown to the user, from its strings | Nothing |
| `settings` | nori's settings as stored and in effect, and its overrides | `overrides`, to override |
| `rows` | Its rows' values, live status lines, rows hidden or greyed | Nothing |
| `session` | What plays, the queue, the library, and playback controls | A permission for each part |
| `subsonic` | The open profile's Subsonic API, signed by nori | `Permission::Subsonic` |
| `timers` | Wakes after a time, once or repeating | `Permission::Schedule`, outside an active session |
| `sign-in` | The client's sign-in page for its provider's profile | `signin` |
| `wake` | Its wake receivers on or off | `wake` |
| `sockets` | TCP with optional TLS, and UDP | `net`, and `lan` for private addresses and multicast |
| `discovery` | mDNS browsing, found services coming to `hooks` | `lan` |
| `lan-serve` | HTTP endpoints at `/p/<id>/` on nori's LAN address | `lan` |
| `audio-out` | nori's audio into a socket, or served at a LAN URL | A socket it opened, or `lan` to serve |
| `media` | A song's stream and cover URLs for another device | Nothing |
| `cast` | Devices offered in nori's output pickers, and the cast's state | Nothing |
| `bluetooth` | RFCOMM and GATT links to matching devices | `devices` |

A Rust plugin's standard library also imports WASI. nori answers randomness, the clocks and an empty
environment. Files, sockets and standard input and output trap.

The SDK's `fetching` module runs `http.start` requests from async code: `fetching::send` makes a request
future, and `fetching::run` waits for it. Requests polled together, as by a `join_all`, are on their way
at once.

`settings.override` refuses a key that the manifest does not list. A value outside a key's range is
held to the range.

## Permissions

A permission lets a plugin use part of the `session` or `timers` interface. The install sheet lists each
one, in the words this table shows for Android:

| Permission | Allows | The install sheet says |
|---|---|---|
| `PlaybackRead` | `now`, `queue`, `queue-length`, `current-profile` | See what plays and the queue |
| `PlaybackControl` | Play, pause, skip, seek, shuffle, repeat, and queue edits | Control playback and the queue |
| `LibraryRead` | `search`, `lookup`, `songs-of` | Search your library |
| `LibraryWrite` | `star`, `add-to-playlist` | Change favorites and playlists |
| `Subsonic` | `subsonic.call` | Use your server's API as you |
| `Schedule` | Timers without an active session | Wake now and then, never while music plays with the screen off |

`session.add` refuses a provider song unless the user's switch lets autofill add provider songs.
`session.clear` removes the songs after the one playing.

## Hooks

A plugin hears an event only when its manifest names the hook and `plugin!` names `hooks`. nori builds
no event that no enabled plugin hears.

| Hook | The event it brings |
|---|---|
| `PlaybackStarted` | `playback-started` |
| `PlaybackPaused` | `playback-paused` |
| `SongChanged` | `song-changed`, with the song or none |
| `QueueChanged` | `queue-changed` |
| `FavoriteChanged` | `favorite-changed`, with the id and whether it is a favorite now |
| `ProfileOpened` | `profile-opened`, with the profile's id |
| `RouteChanged` | `route-changed`, with the route |
| `DeviceConnected` | `device-connected` and `device-disconnected` |
| `SettingChanged` | `setting-changed`, with the setting's key and value |
| `MediaButton` | `media-button`, with the button's name |
| `LyricLine` | `lyric-line`, with the line's index and text |
| `OutputPicker` | `output-picker`, true when a client's output picker opens and false when it closes |

`LyricLine` costs a wake at each lyric line while music plays, and the install sheet says so.

These events need no hook:

- `timer`, for the plugin's timers.
- `asset-ready`, for its assets.
- `discovered`, for the mDNS services it browses.
- `notified`, for data from its Bluetooth links.
- `woke`, for a wake event that started nori.

The media buttons are `play-pause`, `play`, `pause`, `next`, `previous` and `stop`. During a cast,
`volume-up`, `volume-down` and `cast-stop` come too.

## Overrides

An override key is the name of one of nori's settings, such as `crossfadeSec`, or a tunable: a value
with no setting. A setting's override goes through that setting's own checks. Turning the plugin off
removes its overrides. When two plugins override one key, the later one in the user's order wins.

| Tunable | Range | What it sets |
|---|---|---|
| `playback.speed` | 0.5 to 2 | Playback speed |
| `playback.pitch` | 0.5 to 2 | Pitch |
| `silence.skip` | Switch | Skipping silence |
| `queue.previousAlwaysSkips` | Switch | Previous goes back a song, never to the song's start |
| `autoMix.longestSec` | 1 to 40 | AutoMix's longest mix, in seconds |
| `autoMix.tempoPct` | 0 to 12 | AutoMix's largest tempo change, in percent |
| `autoMix.beatMatch` | Switch | Beat matching in mixes |
| `autoMix.bassSwap` | Switch | Bass swaps in mixes |
| `autoMix.filters` | Switch | Filter effects in mixes |
| `autoMix.keepPitch` | Switch | Keeping the pitch when the tempo changes |

The AutoMix tunables replace what the user's AutoMix style sets.

## Limits

| Limit | Value |
|---|---:|
| A plugin's store | 1 MB |
| A plugin instance's memory | 256 MB |
| Requests on their way at once | 16 |
| Frames in a sound stage's block | 8192 |
| Songs in one `session.queue` call | 500 |
| Timers without an active session, at most once per | 15 minutes |
| An icon | 256 KiB |

Each call also has a fuel budget, which wasmtime counts as the call runs. It is 20,000,000,000 for a
plugin's own call, 2,000,000,000 for a decider's and 400,000,000 for a sound stage's block. A call that runs out
fails, and the next call starts a fresh instance. A sound stage that fails is bypassed until the next
seek or song.

## API version

A plugin runs only on a nori with the same plugin API version. The API is 7: `API` in the SDK, and the
`nori:plugin@7.0.0` package in `wit/`. A changed record, function or export makes a new API. A new host
interface does not, since a plugin built before it still runs. An older nori refuses a plugin that calls
the new interface, as `Needs`.

A plugin built for an older API stays installed with its store, but it runs nothing. Its profile waits,
and nori looks for its update at once, even when the user keeps its version. A repo lists the versions
built for each API, so each nori installs the newest version it runs.

## Repos

A repo is a static `index.json` with its packages, on any host. nori's own repo is nori-plugins' index.

```json
{"plugins": [{"id": "com.example.balance", "name": "Balance", "about": "…",
  "source": "https://…", "readme": "balance/README.md", "icon": "balance/icon.png",
  "points": ["sound-stage"], "imports": ["types"], "net": [], "category": "playback",
  "versions": [{"version": "0.2.0", "api": 7, "released": "2026-11-02", "url": "balance-0.2.0.noriplugin",
    "sha256": "<hex>", "key": "<hex, 32 bytes>", "signature": "<hex, 64 bytes>"}]}]}
```

| Field | Required | What it holds |
|---|---|---|
| `id`, `name` | Yes | The plugin's id and name |
| `about`, `source` | No | One line about it, and where its code is |
| `readme`, `icon` | No | URLs of its README and icon, absolute or relative to the index |
| `points` | No | Its extension points. nori offers a `provider` on the first screen |
| `imports`, `net` | No | Its host interfaces and hosts, which tell the web client whether it runs there |
| `category` | No | `service`, `device`, `playback` or `other`, its manifest's `category`. Another or none is `other` |
| `versions` | Yes | Every version it published, in any order |
| `version`, `api`, `url`, `sha256` | Yes | The version, its API, its package and the package's sha256 |
| `released` | No | The release day, `YYYY-MM-DD` |
| `key`, `signature` | No | The author's ed25519 public key and its signature over the package's sha256 |

nori shows only the versions built for its own API. It checks a package's sha256 before anything else.

### Signatures

`signature` is the author's ed25519 signature over the package's SHA-256 digest, its 32 raw bytes, not
the hex. A version has both `key` and `signature`, or neither. The first signed version a user installs
pins its key. Every later version must be signed by that key, and a file installs over it only once the
user removes the plugin. An unsigned version installs while no key is pinned, and the sheet says
"Unsigned".

nori's SDK has a signing tool. From a nori checkout, run:

```sh
cargo run -p nori-plugin-sdk --example sign -- new KEYFILE
cargo run -p nori-plugin-sdk --example sign -- KEYFILE PACKAGE
```

The first command makes a key and prints its public half. The second prints the version's `sha256`,
`key` and `signature` fields. Keep the key file safe: every update must be signed with it.

## Installing a package by hand

Each client installs a `.noriplugin` without a repo. A file is always unsigned.

| Client | Where | What it takes |
|---|---|---|
| Android | **Settings › Plugins › Get plugins**, then **Install from file** | A `.noriplugin` file on the device |
| The desktop app | **Settings**, the **Plugins** tab, **Get plugins**, then **Install from file** | The path of a `.noriplugin` file or a plugin folder |
| The terminal app | **Settings › Plugins › Get plugins**, then **Install from a file** | The path of a `.noriplugin` file or a plugin folder |
| The web client | **Settings › Plugins › Get plugins**, then **Install from file** | A `.noriplugin` file |

The terminal app also installs one at start: `nori-cli --install-plugin PATH` installs or updates the
package or folder at `PATH` and turns it on. To put a file on an Android emulator, run
`adb push balance.noriplugin /sdcard/Download/`.
