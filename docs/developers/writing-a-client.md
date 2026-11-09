# Writing a client

Applies to nori 0.7: the Android app (`android/app/`, `android/core/`), the terminal (`crates/cli`), the desktop
(`crates/desktop`), the iPod touch (`crates/ios`, `ios/`) and the web client in progress (`crates/web`,
`web/`).

The Rust core (`crates/`) is reused whole. A client links it and writes only what its platform forces on
it: drawing, words and the OS hooks. This page lists what the core already does, what every client still
builds, and how to call the core cheaply. The rules behind the split are in
[AGENTS.md](../../AGENTS.md) ("Boundaries").

## Rules

- Every decision lives once, in Rust, tested there. Before writing logic in a client, look for it in
  `crates/host` or the core. If another client would need it too, it belongs in Rust.
- The core returns data and enums, never user-facing text. Every word and every number written for
  display is the client's.
- Speed, CPU, memory, wakeups and battery come first. Work that a crossing would make slower (per-frame
  animation math, string building per list row) stays in the client. Moving work across the boundary
  needs a measurement showing it is at least as fast.
- The UI thread never waits for the core: no FFI or network call in constructors or while drawing.
- An optional feature that is off costs nothing: no thread, listener, socket or audio processor.
- Nothing polls or ticks while music plays with the screen off.

## Start from nori-host

`nori-host` (`crates/host`) is the session every client shares. The terminal, the desktop and the iPod
are interfaces over its `Session`; the web client runs the same session in a browser worker
(`Session::open_driven`, the engine turned by its caller). Android does the same work through the core
directly and is moving onto it.

`Session::open(Open { .. })` takes what only the platform can give:

| Field | What it is |
|---|---|
| `queue` | The app's queue session over its settings (`nori_core::queue::Session`) |
| `data` | The data directory: `nori.db` (`nori_host::db_path`), the stream cache, downloads, covers |
| `http` | The platform's HTTP: a `Transport` (API, covers) that is also a `ByteSource` (audio) |
| `profile` | The saved server profile to open |
| `output` | The sound card, a `nori_engine::AudioOutput` the client built |
| `volume` | A `Level`: set with `set_volume`, or followed with `volume_followed` where the system keeps the volume |
| `memory_mb` | How much audio the engine may hold for songs ahead (256 on the desktop, 96 on the iPod) |
| `covers` | The cover loader's config, or none |
| `offline` | Open on what is stored, with a transport that refuses |
| `mpris` | The process's media controls (`desktop` feature), or none |
| `device`, `discovery` | This device as remote control lists it, and the platform's mDNS (Bonjour on iOS) |
| `tasks` | Where work that waits on the network runs (`nori_host::tasks()` natively) |
| `keeps_music` | Songs are kept as files under `data` (false in a browser) |
| `decoders` | The platform's own decoders (`nori_player::decode::Platform`), preferred to Rust's |
| `out` | Where reports go, from any thread |

What the session then does for the client:

- Plays the core's queue through `nori-engine` with the core's planner, settings, stream addresses,
  ReplayGain, per-device sound (`CoreApp::per_device`) and AutoMix measuring (`CoreApp::measuring`).
- Keeps the stream cache and downloads (`Store`, `Downloader`), the cover loader and search
  (`SearchSession`).
- Runs the core's rules at each moment (below): saving the queue (`Keeper`), a song arriving, autofill at
  the queue's end, the offline bridge, scrobbling, lyrics and favorites.
- Reads pages (`crates/host/src/pages.rs`: `PageAsk`, `read_home`), the stored copy first and the
  server's when it differs.
- Says what the player shows (`player.rs`: `Now`, `QueueShown`, `Playback::{Here, There, Jam}`), the same
  for every client and for the system's media controls.
- Runs remote control and jams while they are on (`remote.rs`: `Remotes`, `Elsewhere`), and joins and
  leaves a jam as a guest (`jam_join`, `jam_joined`, `jam_left`). A client replacing its session calls
  `close_for` with the profile taking over, and `profile_picked` to switch: a guest profile switched
  away from then leaves its jam and goes. A LAN jam's guest searches and
  streams through its host ([Providers](providers.md)): any native client joins any LAN jam, plugins
  or not.
- Looks for the account's device playing while the client says it is in sight (`in_sight`, on every
  change and for a session opened out of sight); the search takes up again by itself when this device's
  music stops or the device shown goes.
- Reports through `Said`: engine events, lyrics, search answers, notes for a status line (`Note`),
  reachability, remote and jam changes, another device's volume, an AutoEQ offer (`Curve`) and changed
  favorites. The client words each one.

The `desktop` feature (on by default) adds what the terminal and desktop share: the cpal sound card,
ureq's HTTP, MPRIS on Linux and Now Playing on macOS (`nori-mpris`), mdns-sd, and `desktop::open` and
`desktop::log_in` over them. The iPod and the web leave it off.

## What the core does

Link these and call them; never reimplement them. `nori-core` holds the `Core` and the `Client` and
re-exports every domain crate by module name (`nori_core::playlist` is `nori_queue::playlist`).

| Area | Crate and modules |
|---|---|
| Where music comes from: a Subsonic server or a plugin's provider, each with its own form and sign-in ([Providers](providers.md)) | `nori-core::providers::Kinds` |
| The server API, request signing, the `Transport` trait, stream cache keys | `nori-net`: `api`, `transport`, `requests`, `stream` |
| The one app database and its background writer | `nori-db` |
| Records, `CoreError`, the log, numbers for display | `nori-model`: `model`, `alog`, `numbers`, `lines` |
| Settings: one `StoredPrefs` field per setting, the model, the pages per client | `nori-settings`: `settings`, `settings_store`, `settings_model`, `settings_pages`, `credits` |
| A backup of the settings, sound profiles and plugins, sealed with a password | `nori-backup` (`nori-core::backup`) |
| Plugins: what repos offer, search, updates waiting, providers to offer on the first screen, a plugin's README and icon ([Plugins](plugins.md)) | `nori-plugins`: `offers_found`, `updates_waiting`, `provider_offers`, `offer_of`, `Plugins::offers_stand`, `Plugins::readme`, `Plugins::icon` |
| Library, search, browse, mixes, smart playlists, history, favorites, M3U | `nori-library`: `library`, `search`, `browse`, `mixes`, `smart`, `history`, `stars`, `m3u` |
| What each page shows, as data: discs, releases by kind, big buttons, row swipes, song menus, the car tree | `nori-library`: `pages`, `rows`, `menus`, `car` |
| The queue: order, shuffle, autofill, radio, the offline bridge, scrobbling, the rules at each moment | `nori-queue`: `playlist`, `queue`, `autofill`, `bridge`, `scrobble`, `rules` |
| Decoding MP3, FLAC, AAC-LC, Vorbis, ALAC and Opus, gapless | `nori-player::decode` |
| The sound chain: EQ, pre-amp, limiter, compressor, headphone sound, balance, mono, ReplayGain and leveling, speed and pitch, silence skip | `nori-player`: `dsp`, `graphic`, `compressor`, `spatial`, `gain`, `speed`, `sonic`, `silence`, `chain` |
| Crossfade and AutoMix: analysis, planning, mixing; the optional neural beat tracker | `nori-player`: `transitions`, `automix`; `nori-automix` |
| Time stretch and pitch shift | `nori-stretch` |
| The whole player on its own thread: bursts, demuxing, the ring, offload, radio, fetching ahead, the stream cache | `nori-engine` (its `core` feature plays the core's queue) |
| Output devices: naming, per-device sound profiles, AutoEQ, bit-perfect decisions | `nori-player`: `outputs`, `device`, `dac`, `policy`; `nori-devices` |
| Downloads and the stream cache's order | `nori-transfers`; `nori-engine::store` |
| Lyrics: the server's and eleven services, matching, scoring, sync against the voice, the lyrics clock | `nori-lyrics`; `nori-look::lyrics` |
| Cover art: fetching, disk and memory caches, decoding JPEG, PNG, WebP and a GIF's first frame | `nori-covers` |
| Colors: cover palettes, the season themes, page colors, the seek bar's pacing, e-ink and burn-in | `nori-look`: `cover`, `palette`, `theme`, `dress`, `motion`, `eink`, `burn_in` |
| Remote control and jams | `nori-remote`; `nori-core::remote`; `nori-tasks` |
| The perf recorder's bookkeeping and report | `nori-perf` |
| Desktop sound card, HTTP, media controls | `nori-output-cpal`, `nori-http`, `nori-mpris` |

## What every client builds

### Network

- **HTTP**: implement `Transport` (`crates/net/src/transport.rs`: `get`, `send`, `address_changed`,
  `network`) and the engine's `ByteSource`, or link `nori-http` (ureq over rustls with webpki roots).
  `send` carries a third party's headers and body (lyrics services, Apple's catalog) as given.
- **Server headers**: `ServerHosts::policy(url)` says whether a request goes to the music server (it then
  carries the profile's headers) and whether it may only use an unmetered network. Keep the answer per
  host. `nori-http` applies neither profile headers, self-signed certificates nor client certificates
  yet; only Android's OkHttp does.
- **One connection pool** for API, covers and audio, so the radio wakes once.
- **Network state**: `Transport::network` answers metered or not. A desktop's is always unmetered.

### Sound

Implement `nori_engine::AudioOutput`, or use `nori-output-cpal` (PipeWire/ALSA, CoreAudio, WASAPI) or
`WavOutput` (a file, on its own clock).

| Method | What the output does |
|---|---|
| `open`, `start`, `pause`, `resume`, `close` | Opens the device near the format asked; pulls from the `Feed` (lock-free, allocation-free) on its own thread |
| `latency_us`, `mixed_us` | How long until a sample pulled now is heard; of that, what the device has mixed already (Bluetooth) |
| `takes_float`, `float` | Whether it plays float, for high quality output |
| `watch` | Tells the engine which device the music goes to (`DeviceWatch`), for per-device sound |
| `shallow` | Holds only a fraction of a second while a sound page is tuned (`nori_core::tuning`) |
| `failed` | The device died and would not reopen |
| `ramp`, `flush`, `holding`, `bursts` | Only for a device that holds seconds (Android's AudioTrack): fades on its volume, drops what it holds, says it still plays, wakes the engine from its own pulls |

Optional, each off unless the client gives it:

- `OffloadOutput`: a device that decodes compressed songs itself, used when nothing touches the samples
  (`nori_player::policy`).
- Bit-perfect output: `Engine::set_output` with `OutputFacts::bit_perfect`.
- Platform decoders: `Platform::aac` for HE-AAC (Android lends MediaCodec), `Platform::later` for a
  decoder that answers later (a browser's WebCodecs). Without one, HE-AAC plays its AAC-LC core.

### Disk

A directory for `nori_engine::Store`: the stream cache (held to the `cache_mb` setting in the core's
eviction order) and downloads. A browser keeps none (`keeps_music: false`).

### The operating system

- Media controls and now playing: `nori-mpris` on Linux and macOS, the media session on Android,
  `MPRemoteCommandCenter` and `MPNowPlayingInfoCenter` on the iPod. Update them on events, never on a
  timer. They show nothing while another device plays (`Destination::shown_to_system`; on the host
  `Playback::shown_to_system`): the system takes them for sound coming out here and says so to Bluetooth
  headsets, and a headset joined to both devices then pauses and resumes this one, presses that would
  reach the other device. Android's session keeps this phone's own player then, let go of and idle.
- Audio focus and interruptions: forward each change to `Engine::focus` (`nori_player::focus`). The engine
  keeps the focus state beside its play state (held by a call, lost for good, ducked), asks a plugin
  (`audio-focus`), pauses and resumes, and says `Event::Focus` with where the focus holds the music. A
  platform with a focus to ask for (Android's) gives the engine a `FocusRequest`, which the engine asks
  before playing and gives back on a pause.
- Media keys and the lock screen: `Handle::media_key` (a toggle or a skip is the core's
  `media_button`, a plugin's word).
- Headphones pulled out, route changes. The client notices; the engine and the core decide (`pause`,
  `pause_now`, the device's sound profile).
- Going to the background: save the queue (`QueueMoment::Closing`).
- A long pause: the engine lets its output go and says `Event::LongPause` (never while listening along to
  a paused jam host); a platform that keeps a playback session in the foreground lets it go then.
- Downloads on the platform's threads or jobs, when the core says to.
- Platform-only features may live wholly in their client (Android Auto, widgets, the lock screen); what
  they show still comes from the core.

### Pictures

- Ask `nori-covers`' `Loader` for a cover at the view's pixel size and draw what comes back. `Paint`
  says what a decoded cover becomes: tight RGBA (`Loader::new`) or a platform picture
  (`Loader::with_paint`; Android's paints Bitmaps). Dropping the `Ticket` cancels a request.
- The server is asked for two renditions only, 320 and 800 px (`cover_rendition`, `cover_rules`), so
  every view of a cover shares one download. `warm` fetches onto the disk only.
- Page colors come from `nori_look::cover::derive` (`nori_host::derive` on the host). Draw them; never
  recompute them.
- HEIF and AVIF are not decoded: the view keeps its placeholder.

### The interface

- **Screens and layout**, taking each page's contents from the core as data.
- **Words**: labels, captions, counts and plurals, times, sizes, decibels. The core hands over counts,
  seconds and kinds (`PlaybackError`, `SongAction`, `MixName`, `ReleaseKind`, `NetError` and the like).
  Numbers go through `nori_model::numbers`. Android's words are `strings.xml` and `strings_ui.xml` (read
  through `android/app/.../ui/Say.kt`), the terminal's `crates/cli/src/text.rs`, the desktop's
  `crates/desktop/src/words.rs`, the iPod's `ios/Sources/Say.swift`.
- **Settings**: `settings_pages` gives a client (`SettingsClient`: `Phone`, `Desktop`, `Terminal`,
  `Web`) its pages, sections and rows, each with its spec, value, and whether it is shown and enabled.
  The client words and draws them and changes a setting by name (`setting_set`). The `SettingChange` it
  answers is applied to the engine (`Session::setting` does this on the host). Sizes on disk and counts
  are handed in as `PageFacts`.
- **Backup**: the Data page's Backup section has two actions, `MakeBackup` and `RestoreBackup`. The
  client picks the file and asks for the password; `backup_make` gives the sealed bytes to write and
  `backup_restore` takes them back. A restore answers the `SettingChange` to apply as a setting's is
  (`Session::backup_restored` on the host) and each plugin's `PluginReturn`; a `BackupError` (wrong password,
  damaged, not a backup, newer, no password, database) is the client's to word. The desktop and the
  terminal go through `nori_host::desktop::{make_backup, restore_backup}`.
- **Appearance** (below), and the `motion` setting: Full, Reduced or E-ink. nori's choice wins over the
  system's. Reduced makes every move direct and keeps the fades; E-ink makes everything direct.
- **Gestures and animation**: their feel is per platform. How long things take and the curve they move
  on are the core's (`stage`: `move_ms` and `move_ease` for a move from rest,
  `fade_ms`/`fade_out_ms`, `settle_stiffness`, and the page stack's
  `page_ms`, `page_ease` and `page_shift`).
- **Drawing cost**: redraw only when something visible changes. A paused player draws nothing.
- Optional: a perf recorder that reads the platform's counters into `nori-perf`
  ([the perf build](building.md#the-perf-build)).

## Appearance

The `theme` setting is `ThemeMode`: `System`, `Light` (Day), `Dark` (Night) and `Midnight`. The `season`
setting is `ThemeSeason`: `Follow` (the calendar) or one of the four seasons.

- `theme_palette(prefs, system_dark, month, day, southern)` gives the palette for the date and whether it
  is dark. `season_palette` does the same from the two values.
- `covers_tint_pages(theme)` is false only for Midnight, which is true black and turns cover colors off.
- `nori_look::eink` paces an e-ink seek bar; `nori_look::burn_in` drifts a screen kept on.

A client labels the modes in its own words, resolves System from the OS (the terminal from its
background; the iPod, which has no system scheme, as Night), and draws the palette it is given.

## What the core decides at each moment

The client notices the moment and carries out what the core answers. On the host the session does all
of this; Android's `PlaybackService` calls the same functions.

- **A song arrives**: `queue::Session::song_arrived` answers `SongSteps`: when to save the queue, songs to
  fetch for its end (autofill), the offline bridge's step, when to measure what is on the device, and the
  sleep timer's last song.
- **The sleep timer**: `queue::Session::sleep_pick(choice, now_ms)` sets a `Sleep` (`Off`, `At`,
  `EndOfSong`, `AfterSongs`) and answers whether the engine pauses at this song's end and the alarm to
  arm; the alarm going off asks `sleep_due`, the engine's `Event::Stopped` ends a song's-end timer
  (`sleep_stopped`), and the player shows `sleep()`. On the host `Handle::sleep_pick` does all of it,
  its time waited for by a task.
- **Where the music plays**: `destination(remote, cast)` answers one `Destination` (here, hosting or
  not; a plugin's cast; another device mirrored; a jam guest's jam), the order of the answers written
  once (`destination_of`, which a client gives what it last heard of each part). On the host, `Handle::destination`.
- **A jam guest's pick**: `Remote::jam_play` and `jam_add` send an admin's pick to the host's queue and
  ask the host for a guest's (`Picked`, which the client words).
- **The queue kept**: `rules::queue_keep(QueueMoment)` says whether to save now or after a delay
  (`Core::playlist_save`) and whether to hand the queue to the server (`Client::playlist_push`).
- **A song the network would not bring**: `Core::bridge_take` answers `BridgeTake` (jump to a download,
  apply the bridge's edit, skip, or stop); `Core::bridge_parked` brings the parked queue back.
- **A screen's read**: `Client::read_cached(read, PageShown)` hands the stored answer at once and the
  server's when it differs. A failure is an error only when nothing was stored.
- **A list read a page at a time**: `paged::Paged` (over the list's cursor; `paged::Pages` counts pages,
  for Android) says which page to read next: one on its way at a time, none after the last, a page
  answered twice (stored, then the server's) taken again, and a page that failed read again with the next
  ask rather than ending the list. A list read again from the top (`reset`) drops the page on its way.
- **Lyrics**: `Client::lyrics_for(id, LyricsShown)` is the whole order: the server's, the services'
  race, then none. What a lyrics view shows for the song heard is `lyrics_follow::LyricsHeard`'s
  `LyricsNow` (no song, looking, shown, failed): the client says which song is heard (`hear`), runs the
  lookup it is told to (`fetch`) while a view is open and hands its answers in (`answer`, `ended`); it
  keeps only a better answer for the song heard, and the last few songs' words, so a song come back to
  starts from them and one whose lookup ended is not looked up again. On the host this is
  `Handle::lyrics_heard`, `lyrics_view`, `lyrics_refresh` and `lyrics_nudge`, each change a
  `Said::Lyrics`. `nori_look::lyrics::matching_line` handles a better answer arriving. Lyrics carry
  `offset_ms`, which `lyrics_clock` applies.
- **A song taken out of the queue**: the last one taken out on its own goes back where it was for
  `nori_queue::undo::UNDO_MS` (`Stage::undo_ms`); `Session::put_back` (`playlist_restore`) refuses it
  after that. A client offering the undo shows it that long; one that shows none calls `Session::undo`.
- **Search**: `SearchSession` answers each keystroke from the offline index and the server once typing
  pauses (`typed_with_index`, `answer`); its `SearchView` says where the answer stands as one
  `SearchShown` (blank, asking, failed, found, nothing found). On the host: `search_typed`,
  `search_server` (reported as `Said::Search`) and `search_server_now` (on the caller's thread).
- **Home's queue offer**: `offer::Offered` keeps a look's answer (`Client::queue_offer`, which asks
  nothing for a profile without an account) while it stands against what the player shows, lets it go
  once music starts or the player shows its song, and hands it over once when taken up or turned down
  (`HomeOffer` for Android). On the host `Handle::queue_offer` asks against what the player shows, and
  `Handle::queue_offer_closed` marks it done and, taken, plays it from its song and place.
- **Adding a profile and signing its account in**: `providers::flow`: `AddFlow` is the pages of adding
  one, `SignInFlow` the sign-in shown, one dialog at a time; `Client::sign_in_due` says what follows a
  profile opening or its account changing, the offer again after the provider ended the session included
  ([Providers](providers.md)). A code sign-in is `Client::sign_in_with_code`: the client holds the
  `CodeSignIn` while `SignInFlow::checks_code` says so and hands its events to the flow; a signed-in answer
  is finished as any sign-in (the host's `Handle::signed_in`, Android's `Nori.signedIn`).
- **A profile's mark**: `SavedServer::icon` (`settings::ProfileIcon`, the picker's order `profile_icons`): its
  letter (`profile_letter`, the default), its provider's mark, or one of Iconoir's faces.
- **A favorite**: `Client::star` marks it at once and puts the old mark back if the server refuses.
- **Fetching ahead**: `Client::precache_targets` names the songs after the next (the `songs_ahead_wifi`
  and `songs_ahead_mobile` settings; never a provider's song). The engine fetches them whole, one after
  another, then leaves the network alone.
- **A new output's AutoEQ curve**: `Core::device_arrive`, then `Client::device_curve`; the client words
  the offer and answers it (`Session::curve_answer`).
- **A sound page's shallow buffer**: `tuning::SoundTuning` counts the sound pages in sight (the
  equalizer, AutoEQ's presets, Settings › Sound, a plugin's page) and answers the buffer to switch to:
  shallow after a change to what is heard on one (a plugin's page from its first look), deep again once
  the last leaves. On the host the client says only `Session::sound_page(page, shown)`; the session's
  `apply` counts the changes.
- **A download's covers**: `Core::download_cover_urls`, for the cover loader to `warm`.

## The clients today

| | Session | Sound | HTTP | Covers | Media controls |
|---|---|---|---|---|---|
| Android | the core and `nori-engine` from Kotlin | AudioTrack (`crates/android/src/track.rs`), MediaCodec for HE-AAC, offload, bit-perfect USB | OkHttp (`android/core/.../net/Http.kt`) | `CoverPixels` paints Bitmaps; `CoverLoader` keeps them | media3 session and notification, Android Auto, widget |
| Terminal | `nori-host` | cpal | `nori-http` | RGBA through ratatui-image (kitty, sixel, iTerm2, half blocks) | MPRIS, macOS Now Playing |
| Desktop | `nori-host` | cpal | `nori-http` | RGBA in Slint | MPRIS, macOS Now Playing |
| iPod | `nori-host` behind a C ABI | AURemoteIO ([iPod internals](ipod-internals.md)) | `nori-http` | RGBA handed to `CGImage` without a copy | `MPRemoteCommandCenter`, `MPNowPlayingInfoCenter` |
| Web | `nori-host`, engine driven by a worker | AudioWorklet, WebCodecs | `fetch` | the browser draws the core's addresses; colors from `cover_look` | the Media Session API |

Android plays only through `nori-engine`. media3 stays for the session, the notification, Android Auto,
the stream cache and downloads.
Around the engine Android keeps per-device sound (`Outputs.kt`, `DeviceSound.kt`), bit-perfect USB
(`BitPerfect.kt`), the offline bridge's network watch (`OfflineBridge.kt`), and a media3 player that
mirrors the core's queue for the session (`EnginePlayer` in `RustPlayer.kt`; every edit goes through the
core first and applies the `QueueEdit` it answers).

## Calling the core cheaply

- **Rust clients** call the crates directly. Depend on `nori-core` with `default-features = false` and no
  uniffi is built (add the `plugins` feature for plugins' providers); each domain crate's `ffi`
  feature is off by default.
- **Android** uses uniffi's JNI generator (`uniffi-bindgen-kotlin-jni`), not JNA, which costs far more
  per call and leaves garbage. `crates/android/build.rs` generates the scaffolding, and
  `cargo run -p uniffi-bindgen -- bindings src:nori-android <dir>` writes the Kotlin: nori-core's in
  `dev.nori.music.ffi`, each domain crate's in its own package below it. A thread the core starts is
  attached to the JVM once, under its own name, and finds the app's classes through the class loader
  `JNI_OnLoad` kept.
- **Android's raw doors** are for anything called per frame, per buffer or per list row: primitives in
  and out, no allocation, registered with `RegisterNatives` in `crates/android/src/lib.rs`. A
  primitive-only door is `@CriticalNative` (no env, no class); a short door over arrays or direct buffers
  that calls nothing back is `@FastNative`. Every raw pointer is checked for null and length before a
  slice is made of it. Doors only convert; logic belongs in the core.
- **The iPod** calls a hand-written C ABI (`crates/ios/src/lib.rs`, `ios/Sources/nori_ios.h`): coarse
  calls answer JSON, per-buffer work is the render callback.
- **The web** calls wasm-bindgen exports that answer JSON.
- Calls are coarse: one response or one page per call.
- Measure a crossing before moving work across it. The Android debug build's `bench` test command
  prints what each kind of crossing costs next to the same work in Kotlin
  ([Report, benchmarks and self test](building.md#report-benchmarks-and-self-test)).

## Twins

Where Android keeps a piece of logic in Kotlin because a crossing would cost more than the work, the core
carries a twin with the same answers, so the other clients get the rule without porting it. Each twin is
held to its Kotlin original by test vectors: the originals are copied into
`crates/core/testdata/twins/CoreTwins.kt`, `tools/twins.sh` runs them on the JVM and writes
`core_twins.tsv`, and `crates/core/tests/twins.rs` must match every row. When an original changes, copy
it again and run `tools/twins.sh`. Nothing a client calls per row, frame or buffer allocates
(`crates/core/tests/twins.rs` and `crates/look/src/no_alloc.rs` count).

## What stays in the client

- **Gestures and per-frame animation math**: drags, springs, easing, flick velocity, and the decisions a
  gesture ends in. A crossing costs more than the few multiplications, and other clients have other
  input.
- **What a gesture drew ahead of its answer** (a record slid in, a row dropped or swiped away) is held
  until the player has answered the command, whatever came of it, never for a guessed time. On Android
  each skip, move and removal gets a ticket (`PlayerConnection.ticket`), and `answered` reaches it once
  the state shows its outcome (`data/Pending.kt`).
- **Drawing**: layout, text measurement, glyph caches, bitmaps, the lyrics' fill and glow. When the page
  redraws and how far the singing is are the core's (`LyricClock`).
- **Glue around platform objects**: the media session, the HTTP client's TLS setup, the network monitor,
  notifications, the Bitmap memory cache, sorting the platform's errors into the core's kinds.
- **The perf build's counters** and **the debug build's test bridge**.
