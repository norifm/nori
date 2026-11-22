# Writing a client

A nori client links the Rust core and writes only what its platform forces on it: drawing, words and the
operating system's hooks. This page lists what the core already does, what every client still builds,
and how to call the core cheaply.

It covers every client:

- Android: `android/app/`, `android/core/` and `crates/android`.
- The desktop app: `crates/desktop` and `crates/desktop-ui`.
- The terminal app: `crates/cli`.
- The iPod app: `crates/ios` and `ios/`.
- The web client, in development: `crates/web` and `web/`.

[Architecture](architecture.md) has the layers and the rules that hold for every client. In short:
every decision lives once, in Rust. Before you write logic in a client, look for it in `crates/host` or
the core. If another client would need it too, it belongs in Rust.

## What a client must not do

- The UI thread never waits for the core. Make no FFI or network call in a constructor or while drawing.
- A client never words what it did not get as data. The core returns data and enums, and every word and
  every number written for display is the client's.
- A client never moves work across the boundary without a measurement. Work that a crossing would make
  slower, such as per-frame animation math or string building per list row, stays in the client.
  Moving it needs a measurement that shows the crossing is at least as fast.

## Start from nori-host

`nori-host` (`crates/host`) is the session that the desktop app, the terminal app and the iPod app
share. Each of them is an interface over its `Session`. The web client runs the same session in a
browser worker with `Session::open_driven`, where the caller turns the engine instead of a thread.
Android does the same work through the core directly and is moving onto `nori-host`.

`Session::open(Open { .. })` takes what only the platform can give:

| Field | What it is |
|---|---|
| `queue` | The app's queue session over its settings (`nori_core::queue::Session`) |
| `data` | The data directory: `nori.db` (`nori_host::db_path`), the stream cache, downloads, covers |
| `http` | The platform's HTTP (`Net`: a `Transport` and a `ByteSource`) |
| `profile` | The saved profile to open |
| `output` | The sound card, a `nori_engine::AudioOutput` that the client built |
| `volume` | A `Level`, set with `set_volume` or followed with `volume_followed` |
| `memory_mb` | The memory size that bounds the audio held ahead |
| `covers` | The cover loader's config, or `None` |
| `offline` | Opens on what is stored, with a transport that refuses |
| `mpris` | The process's media controls (`desktop` feature), or `None` |
| `device` | This device as remote control lists it |
| `discovery` | The platform's mDNS (Bonjour on iOS), or `None` for the session's own |
| `tasks` | Where work that waits on the network runs (`nori_host::tasks()` natively) |
| `keeps_music` | Whether songs are kept as files under `data` |
| `decoders` | The platform's own decoders (`nori_engine::Platform`), preferred to Rust's |
| `out` | Where reports go, from any thread |
| `kinds` | The kinds of profile it opens (`Kinds`): see [Providers](providers.md) |

The `Transport` side of `http` carries the API and covers, and the `ByteSource` side carries audio.
`volume_followed` is for platforms where the system keeps the volume. A browser keeps no music files.
`memory_mb` is 256 on the desktop, 96 on the iPod and 128 on the web. The engine holds a quarter of
`memory_mb` in megabytes of audio ahead, at least 16 and at most 48 (`nori_player::transport::load_control`).

### What the session does

Once open, the session does this work for the client:

- It plays the core's queue through `nori-engine`, with the core's transition planner, settings, stream
  addresses, ReplayGain, per-device sound (`CoreApp::per_device`) and AutoMix measuring
  (`CoreApp::measuring`).
- It keeps the stream cache and downloads (`Store`, `Downloader`), the cover loader and search
  (`SearchSession`).
- It runs the core's rules at each moment, as
  [What the core decides at each moment](#what-the-core-decides-at-each-moment) lists them. Among them
  are saving the queue (`Keeper`), autofill, the offline bridge, scrobbling, lyrics and favorites.
- It reads pages (`crates/host/src/pages.rs`: `PageAsk`, `read_home`). The stored copy shows first,
  and the server's answer replaces it when it differs.
- It says what the player shows (`crates/host/src/player.rs`: `Now`, `QueueShown`, and `Playback::Here`,
  `Playback::There` or `Playback::Jam`). The answer is the same for every client and for the system's
  media controls.
- It runs remote control and jams while they are on (`crates/host/src/remote.rs`: `Remotes`,
  `Elsewhere`). It joins and leaves a jam as a guest (`jam_join`, `jam_joined`, `jam_left`).
- It looks for the account's device that plays while the client says it is in sight (`in_sight`). The
  client calls `in_sight` on every change, and also for a session opened out of sight. The search starts
  again by itself when this device's music stops or the device shown goes away.
- It lists and switches the account's identities (`identities`, `choose_identity`). The client reopens
  the session after a switch.
- It keeps a profile's name and look (`profile_renamed`, `profile_look`).

A client that replaces its session calls `close_for` with the profile that takes over, and
`profile_picked` to switch. A jam guest profile that the user switches away from leaves its jam and is
removed. A LAN jam's guest searches and streams through its host, so any native client joins any LAN
jam, with or without plugins.

The session reports through `Said`, from any thread, and the client words each report:

| `Said` | What happened |
|---|---|
| `Engine` | An engine `Event`: a new song, a state change, an error |
| `Lyrics`, `Search` | What the lyrics view or the search field shows changed |
| `Note` | Something done or failed, for a status line |
| `Reachable` | The reachability check made when the session opened |
| `Remote`, `JamEnded`, `Volume` | Remote control or the jam changed, or another device set the volume |
| `Curve` | An AutoEQ curve was offered or applied for a new output |
| `Starred` | A favorite changed, here or on another device |
| `Working` | Downloads or work on saved songs started |
| `Pictured` | The account has a new picture |
| `Update` | What an update check found |
| `AnalysisOffer` | Downloaded songs can be measured for AutoMix |
| `Overridden` | What plugins override of the settings changed |

After `Said::Working`, the client reads `progress` once a second until it answers `None`.

### The desktop feature

The `desktop` feature of `nori-host` is on by default. The iPod app and the web client turn it off. It
adds what the terminal app and the desktop app share:

- the cpal sound card (`nori-output-cpal`) and ureq's HTTP (`nori-http`)
- MPRIS on Linux and Now Playing on macOS (`nori-mpris`)
- mdns-sd, the plugin host, and Bluetooth on Linux and macOS
- `nori_host::desktop::open`, which takes a `DesktopOpen`
- `nori_host::desktop::Root`, which logs in, removes profiles, installs plugins and makes backups.

## What the core already does

Link these crates and call them. Never write them again in a client. `nori-core` holds the `Core` and
the `Client`, and it re-exports every domain crate by module name: `nori_core::playlist` is
`nori_queue::playlist`.

| Area | Crate and modules |
|---|---|
| Where music comes from ([Providers](providers.md)) | `nori_core::providers::Kinds` |
| The server API, request signing, the `Transport` trait, stream cache keys | `nori-net`: `api`, `transport`, `requests`, `stream` |
| The app database and its background writer | `nori-db` |
| Records, `CoreError`, the log, numbers for display | `nori-model`: `model`, `alog`, `numbers`, `lines` |
| Settings and their pages | `nori-settings`: `settings`, `settings_store`, `settings_model`, `settings_pages`, `credits` |
| A backup sealed with a password | `nori-backup` (`nori_core::backup`) |
| What plugin repos offer, updates, a plugin's README and icon ([Plugins](plugins.md)) | `nori-plugins` |
| Library, search, browse, mixes, smart playlists, history, favorites, M3U | `nori-library`: `library`, `search`, `browse`, `mixes`, `smart`, `history`, `stars`, `m3u` |
| What each page shows, as data | `nori-library`: `pages`, `rows`, `menus`, `car` |
| The queue: order, shuffle, autofill, radio, the offline bridge, scrobbling | `nori-queue`: `playlist`, `queue`, `autofill`, `bridge`, `scrobble`, `rules` |
| Scrobbling to ListenBrainz, Libre.fm and Last.fm | `nori_core::scrobblers` |
| Decoding MP3, FLAC, AAC-LC, Vorbis, ALAC and Opus, gapless | `nori_player::decode` |
| The sound chain | `nori-player`: `dsp`, `graphic`, `compressor`, `spatial`, `gain`, `speed`, `sonic`, `silence`, `chain` |
| Crossfade and AutoMix, with the optional neural beat tracker | `nori-player` (`transitions`, `automix`), `nori-automix` |
| Time stretch and pitch shift | `nori-stretch` |
| The whole player on its own thread | `nori-engine` (its `core` feature plays the core's queue) |
| Outputs, per-device sound profiles, AutoEQ, bit-perfect decisions | `nori-player` (`outputs`, `device`, `dac`, `policy`), `nori-devices` |
| Downloads and the stream cache's order | `nori-transfers`, `nori_engine::store` |
| Lyrics: the server's and eleven services, scoring, sync against the voice | `nori-lyrics`, `nori_look::lyrics` |
| Covers: fetching, disk and memory caches, decoding | `nori-covers` |
| Colors from covers, the seasons, the seek bar's pacing, e-ink and burn-in | `nori-look`: `cover`, `palette`, `theme`, `dress`, `motion`, `eink`, `burn_in` |
| Remote control and jams | `nori-remote`, `nori_core::remote`, `nori-tasks` |
| The perf recorder's bookkeeping and report | `nori-perf` |
| The desktop's sound card, HTTP and media controls | `nori-output-cpal`, `nori-http`, `nori-mpris` |

Backups need the `plugins` feature of `nori-core`, so the iPod app and the web client have none. The
backup holds the settings, sound profiles, favorites kept on this device, and plugins.

The sound chain holds the equalizer, pre-amp, limiter, compressor, headphone sound, balance, mono,
ReplayGain and volume leveling, speed and pitch, and silence skip.

### Scrobbling

`nori_core::scrobblers` sends a play to the server and to the profile's ListenBrainz, Libre.fm and
Last.fm accounts (`Client::scrobble`). The server gets the play when its switch is on and the profile's
rules allow it. Plays that an account did not take wait for later, for up to 14 days
(`scrobbles_waiting`).

The add form for an account comes from the core too:

- `scrobbler_kinds` lists the kinds, in order (`scrobbler_kinds_json` for JSON clients).
- `scrobbler_fields` lists the fields each kind asks for, with the advanced ones folded away.
- `scrobbler_help_page` says where the values come from.
- `scrobbler_form_ready` says when a form is complete.
- `scrobbler_sign_in` signs an account in. Last.fm signs in on its own page: call
  `Client::scrobbler_approval`, then `Client::sign_in_scrobbler`.

The accounts are `SavedServer::scrobblers`, changed with `profile_scrobbler_put` and
`profile_scrobbler_removed`.

## What every client builds

### Network

- **HTTP.** Implement `Transport` and the engine's `ByteSource`, or link `nori-http`, which is ureq over
  rustls with webpki roots. `Transport` is in `crates/net/src/transport.rs`: `get`, `send`,
  `address_changed` and `network`.
  `send` carries a third party's headers and body as given, for lyrics services and Apple's catalog.
- **Server headers.** `ServerHosts::policy(url)` says whether a request goes to the music server. Such
  a request carries the profile's headers, and it may be limited to an unmetered network. Keep the
  answer per host. Only Android's OkHttp applies profile headers, self-signed certificates and client
  certificates. `nori-http` applies none of them.
- **One connection pool** serves the API, covers and audio, so the radio wakes once.
- **Network state.** `Transport::network` answers whether the network is metered. A desktop's network is
  always unmetered.

### Sound

Implement `nori_engine::AudioOutput`, or use `nori-output-cpal` (PipeWire or ALSA, CoreAudio, WASAPI).
`WavOutput` writes to a file, on its own clock.

| Method | What the output does |
|---|---|
| `open` | Opens the device in the format closest to the one asked for |
| `start`, `pause`, `resume`, `close` | Pulls from the `Feed` on its own thread, or stops pulling |
| `latency_us` | How long until a sample pulled now is heard |
| `mixed_us` | Of that latency, what the device has mixed already (Bluetooth) |
| `takes_float`, `float` | Whether it plays float, for high quality output |
| `watch` | Tells the engine which device the music goes to (`DeviceWatch`), for per-device sound |
| `shallow` | Holds only a fraction of a second while a sound page is tuned |
| `failed` | Says that the device stopped and would not reopen |
| `ramp`, `flush`, `holding`, `bursts` | Only for a device that holds seconds, such as Android's AudioTrack |

The `Feed` is lock-free and does not allocate. A device that holds seconds does four more things:

- It fades on its own volume (`ramp`).
- It drops what it holds (`flush`).
- It says it still plays (`holding`).
- It wakes the engine from its own pulls (`bursts`).

Each of these is off unless the client gives it:

- `OffloadOutput` is a device that decodes compressed songs itself. The engine uses it only when nothing
  touches the samples (`nori_player::policy`).
- Bit-perfect output is `Engine::set_output` with `OutputFacts::bit_perfect`.
- Platform decoders are `Platform::aac` for HE-AAC (Android lends MediaCodec) and `Platform::later` for
  a decoder that answers later (a browser's WebCodecs). Without one, HE-AAC plays its AAC-LC core.

### Disk

The client gives `nori_engine::Store` a directory for the stream cache and downloads. The core holds
the stream cache to the `cache_mb` setting, in its own eviction order. A browser keeps no files
(`keeps_music: false`).

### Media controls

The media controls and now playing info are `nori-mpris` on Linux and macOS, the media session on
Android, and `MPRemoteCommandCenter` with `MPNowPlayingInfoCenter` on the iPod. Update them on events,
never on a timer.

While another device plays, MPRIS, Now Playing and the iPod's lock screen show nothing
(`Destination::shown_to_system`, or `Playback::shown_to_system` on `nori-host`). The system takes them
for sound that comes out of this device, and it tells Bluetooth headsets so. A headset joined to both
devices would then pause and resume this device, when the presses belong to the other one.

Android shows the other device as remote playback instead (media3 `DeviceInfo.PLAYBACK_TYPE_REMOTE`,
`remote/RemoteDevicePlayer.kt`):

- The notification and the lock screen control the other device.
- The volume keys step its volume through Android's volume panel (`mirror_volume_key`).
- The output switcher names it (`remote/RemoteRoute.kt`, a MediaRouter2 routing session). Android 13 to
  15 also need that session before the volume keys reach a remote session.
- The Bluetooth stack still tells a headset that the session plays. It sends the session the headset's
  play and pause, as the caller `Process.BLUETOOTH_UID`. `mirror_system_press` decides what such a press
  does, and play and pause from Bluetooth do nothing.

The phone's own player is released and idle meanwhile.

### Audio focus and the system's events

- **Audio focus.** Forward each focus change to `Engine::focus` (`nori_player::focus`). The engine keeps
  the focus state beside its play state (held by a call, lost for good, ducked). It asks a plugin
  (`audio-focus`), pauses and resumes, and sends `Event::Focus`. A platform with a focus to request, such
  as Android, gives the engine a `FocusRequest`. The engine requests it before playing and gives it back
  on a pause.
- **Media keys and the lock screen.** Call `Handle::media_key`. A toggle or a skip goes to the core's
  `media_button`, where a plugin can decide.
- **Headphones pulled out and route changes.** The client notices them. The engine and the core decide
  what follows: `pause`, `pause_now`, or the device's sound profile.
- **Going to the background.** Save the queue (`QueueMoment::Closing`).
- **A long pause.** The engine releases its output and sends `Event::LongPause`. It never does while it
  listens along to a paused jam host. A platform that keeps a playback session in the foreground
  releases it then.
- **Downloads** run on the platform's threads or jobs, when the core says to.

A feature only one platform can have, such as Android Auto, widgets or the iPod's lock screen, may live
wholly in its client. What it shows still comes from the core.

### Covers

- Ask the `nori-covers` `Loader` for a cover at the view's pixel size, and draw what comes back. `Paint`
  says what a decoded cover becomes: tight RGBA (`Loader::new`) or a platform picture
  (`Loader::with_paint`). Android's paint makes Bitmaps. Dropping the `Ticket` cancels a request.
- The server is asked for two renditions only, 320 and 800 px (`cover_rendition`, `cover_rules`). So
  every view of a cover shares one download. `warm` fetches a cover onto the disk only.
- Page colors come from `nori_look::cover::derive` (`nori_host::derive` on `nori-host`). Draw them and
  never compute them again.
- nori does not decode HEIF or AVIF. The view keeps its placeholder.

### The interface

- **Screens and layout** take each page's contents from the core as data.
- **Words** are the client's: labels, captions, counts and plurals, times, sizes, decibels. The core
  hands over counts, seconds and kinds (`PlaybackError`, `SongAction`, `MixName`, `ReleaseKind`,
  `NetError` and the like). Numbers go through `nori_model::numbers`. Android's words are `strings.xml`
  and `strings_ui.xml`, read through `ui/Say.kt`. The terminal's are `crates/cli/src/text.rs`, the
  desktop's `crates/desktop/src/words.rs`, and the iPod's `ios/Sources/Say.swift`.
- **Settings.** `settings_pages` gives a client (`SettingsClient`: `Phone`, `Desktop`, `Terminal`,
  `Web`) its pages, sections and rows. Each row has its spec, its value, and whether it shows and is
  enabled. The client words and draws them, and changes a setting by name (`setting_set`). It applies
  the `SettingChange` that answers to the engine (`Session::setting` does this on `nori-host`). Sizes on
  disk and counts go in as `PageFacts`.
- **Backup.** The Data page's Backup section has two actions, `MakeBackup` and `RestoreBackup`. The
  client picks the file and asks for the password. `backup_make` gives the sealed bytes to write, and
  `backup_restore` takes them back. A restore answers a `SettingChange` to apply, as a setting does
  (`Session::backup_restored` on `nori-host`), and each plugin's `PluginReturn`. A `BackupError` is the
  client's to word. The desktop app and the terminal app go through `Root::make_backup` and
  `Root::restore_backup`.
- **Motion.** The `motion` setting is Full, Reduced or E-ink, and nori's choice wins over the system's.
  Reduced uses short fades instead of slides and zooms. E-ink moves nothing.
- **Gestures and animation.** Their feel is per platform. How long things take, and the curve they move
  on, are the core's `Stage`. A move from rest uses `move_ms` and `move_ease`. Fades use `fade_ms` and
  `fade_out_ms`, and settling uses `settle_stiffness`. The page stack uses `page_ms`, `page_ease` and
  `page_shift`.
- **Drawing cost.** Redraw only when something visible changes. A paused player draws nothing.
- **A perf recorder** is optional. It reads the platform's counters into `nori-perf` (the perf build in
  [Building](building.md)).

### Appearance

The `theme` setting is a `ThemeMode`: `System`, `Light` (Day), `Dark` (Night) or `Midnight`. The `season`
setting is a `ThemeSeason`: `Follow` (the calendar) or one of the four seasons.

- `theme_palette(prefs, system_dark, month, day, southern)` gives the palette for the date and whether
  it is dark. `season_palette` does the same from the two values.
- `covers_tint_pages(theme)` is false only for Midnight, which is true black and turns cover colors off.
- `nori_look::eink` paces an e-ink seek bar. `nori_look::burn_in` drifts a screen that stays on.

A client labels the modes in its own words and draws the palette it gets. It resolves System from the
OS. The terminal app reads it from the terminal's background. The iPod has no system scheme, so the
iPod app reads System as Night.

## What the core decides at each moment

The client notices the moment, asks the core, and does what the core answers. On `nori-host` the
session does all of this. Android's `PlaybackService` calls the same functions.

### Playback

- **A song arrives.** `queue::Session::song_arrived` answers `SongSteps`. They say when to save the
  queue, and which songs to fetch for its end (autofill). They also hold the offline bridge's step, when
  to measure what is on the device, and the sleep timer's last song.
- **The sleep timer.** `queue::Session::sleep_pick(choice, now_ms)` sets a `Sleep` (`Off`, `At`,
  `EndOfSong`, `AfterSongs`). It answers whether the engine pauses at this song's end, and the alarm to
  set. When the alarm goes off, ask `sleep_due`. The engine's `Event::Stopped` ends a timer set for a
  song's end (`sleep_stopped`). The player shows `sleep()`. On `nori-host`, `Handle::sleep_pick` does
  all of it and waits for the time on a task.
- **Where the music plays.** `destination(remote, cast)` answers one `Destination`. It is `Here`
  (hosting a jam or not), `Cast` (a plugin's cast), `Mirroring` (another device) or `Guest` (a jam's).
  `destination_of` decides the order of the answers in one place, from what the client last heard of
  each part. On `nori-host`, call `Handle::destination`.
- **A jam guest's pick.** `Remote::jam_play` and `jam_add` send an admin's pick to the host's queue.
  For a guest, they ask the host, which answers a `Picked` that the client words.
- **Fetching ahead.** The engine fetches the next song as soon as it is next, even a provider song.
  That song plays anyway. The server needs a while to fetch it from its provider, too long for a mix
  into it.
  `Client::precache_targets` names the songs after it, by the `songs_ahead_wifi` and
  `songs_ahead_mobile` settings, and never a provider song. The engine fetches them whole, one after
  another, then leaves the network alone.
- **A new output's AutoEQ curve.** Call `Core::device_arrive`, then `Client::device_curve`. The client
  words the offer and answers it (`Session::curve_answer`).
- **A sound page's shallow buffer.** `tuning::SoundTuning` counts the sound pages in sight: the
  equalizer, AutoEQ's presets, **Settings › Sound**, and a plugin's page. It answers the buffer to switch
  to. After a change to what is heard on one of them, the buffer goes shallow. A plugin's page goes
  shallow from its first look. Once the last page leaves, the buffer goes deep again. On `nori-host`, the
  client only calls `Session::sound_page(page, shown)`.

### The queue

- **Keeping the queue.** `rules::queue_keep(QueueMoment)` says whether to save now or after a delay
  (`Core::playlist_save`), and whether to hand the queue to the server (`Client::playlist_push`).
- **A song the network would not bring.** `Core::bridge_take` answers a `BridgeTake`. It is `Jump` (to
  a download), `Bridged` (apply the bridge's edit), `Skip` or `Stop`. `Core::bridge_parked` brings the parked queue back.
- **A song removed from the queue.** The last song removed on its own can go back where it was for
  `nori_queue::undo::UNDO_MS` (`Stage::undo_ms`). After that, `Session::put_back` (`playlist_restore`)
  refuses it. A client that offers the undo shows it for that long. A client that shows no undo calls
  `Session::undo`.
- **Clearing the queue.** `Session::clear_upcoming` (`playlist_clear_upcoming`, or `Op::Clear` on the
  device that plays) removes the songs after the one that plays, in play order. For the same `UNDO_MS`,
  `Session::put_back_cleared` (`playlist_restore_all`, `Op::RestoreCleared`) puts them back.
- **The queue's head.** `menus::queue_actions` lists the actions for the whole queue, in order. **Clear**
  shows while songs follow the one that plays. **Add to playlist** adds the queue in play order, through
  the client's playlist picker, where the account's playlists take songs. A jam guest gets neither.
  Clear is a word. Add to playlist is the plus-circle glyph that a mix's page uses to keep itself.

### Pages and search

- **A screen's read.** `Client::read_cached(read, PageShown)` hands over the stored answer at once and
  the server's answer when it differs. A failure is an error only when nothing was stored.
- **A list read a page at a time.** `paged::Paged` says which page to read next, over the list's cursor.
  One page is on its way at a time, and none after the last. A page answered twice (stored, then the
  server's) is taken again. A page that failed is read again on the next ask, so the list does not end.
  A list read again from the top (`reset`) drops the page on its way. `paged::Pages` counts pages, for
  Android.
- **Search.** `SearchSession` answers each keystroke from the offline index, and asks the server once
  typing pauses (`typed_with_index`, `answer`). Its `SearchView` gives one `SearchShown`: blank, asking,
  failed, found or nothing found. An ask that newer typing replaced ends at once with `None` and drops
  its request. Every view it returns belongs to the field, wherever the field's answer shows: the
  search page and a preview under the field alike. On `nori-host`, call `search_typed`, `search_server`
  (reported as `Said::Search`) and `search_server_now` (on the caller's thread).
- **Home's queue offer.** `offer::Offered` keeps the answer of `Client::queue_offer` while it still
  fits what the player shows. `Client::queue_offer` asks nothing for a profile without an account. The
  offer goes away once music starts or the player shows its song. It is handed over once, when the user
  takes it or turns it down (`HomeOffer` for Android). On `nori-host`, `Handle::queue_offer` asks
  against what the player shows. `Handle::queue_offer_closed` marks the offer done. If the user took it,
  it plays the offer from its song and place.

### Lyrics

`Client::lyrics_for(id, LyricsShown)` is the whole order: the server's lyrics, then the services' race,
then none. A client drops the lookup when the song heard changes or the view closes.

`lyrics_follow::LyricsHeard` says what a lyrics view shows for the song heard, as a `LyricsNow`: no
song, looking, shown or failed. The client says which song is heard (`hear`). While a view is open, the
client runs the lookup it is told to (`fetch`). It hands the answers in (`answer`, `ended`).
`LyricsHeard` keeps only a better answer for the song heard. It also keeps the last few songs' words. A
song that comes back starts from them, and a song whose lookup ended is not looked up again. On `nori-host`, this is
`Handle::lyrics_heard`, `lyrics_view`, `lyrics_refresh` and `lyrics_nudge`, and each change is a
`Said::Lyrics`.

While remote control is on, the lookup runs until it is dropped. A device that shows another device's
playback shows the lyrics that device sends with its state, timed as it times them. Only the device
that plays hears the song to check the lyrics against, so the other device looks nothing up. A nudge
there goes to the device that plays (`Op::LyricsTiming`), and a nudge made where it plays comes back.

`nori_look::lyrics::matching_line` handles a better answer that arrives while lyrics show. Lyrics carry
`offset_ms`, which `lyrics_clock` applies.

### Profiles, accounts and favorites

- **Adding a profile and signing its account in.** `providers::flow` has both. `AddFlow` is the pages
  of adding a profile. `SignInFlow` is the sign-in shown, one dialog at a time. `Client::sign_in_due` says
  what follows a profile opening or its account changing. A code sign-in is `Client::sign_in_with_code`.
  A signed-in answer finishes as any sign-in does (`Handle::signed_in` on `nori-host`, `Nori.signedIn`
  on Android). [Providers](providers.md) has the details.
- **The profiles' order.** `StoredPrefs::servers` is the user's order, and switching keeps it
  (`profile_picked`, `settings::servers_activated`). Every list and switcher shows the profiles in that
  order and marks the one in use where it stands. Settings move a profile with `profilePlace`
  (`<id>:<place>`), but not a jam guest's.
- **A profile's mark.** `SavedServer::icon` is a `settings::ProfileIcon`, and `profile_icons` is the
  picker's order. It is the profile's letter (`profile_letter`, the default), its provider's mark, or one
  of Iconoir's faces.
- **A favorite.** `Client::star` marks the item at once, and it puts the old mark back if the server
  refuses. Seals show on songs, albums and artists wherever `ProfileRules::stars` says. Where the
  account cannot keep them (`stars_here`), the core keeps them on this device, and the client draws and
  calls the same functions.

### Downloads

- **A download's covers.** `Core::download_cover_urls` gives the covers for the cover loader to `warm`.
- **A playlist kept downloaded.** A playlist's page downloads the playlist whole with
  `Core::download_playlist`, which keeps it downloaded. `Client::kept_playlists_sync` at launch brings
  the kept playlists up to date. Both answer a `KeptChange`: the downloads queued, and the songs to
  delete. Only songs that keeping brought, and that no kept playlist holds, are deleted. Removing all of
  a playlist's downloads stops keeping it. On `nori-host`, call `Handle::download_later(Fetch::Playlist)`
  or `download_playlist`. The session syncs when it opens.

## The clients today

Each client fills the same parts with its platform's pieces:

| Client | Session | Sound | HTTP | Media controls |
|---|---|---|---|---|
| Android | The core and `nori-engine`, from Kotlin | AudioTrack, MediaCodec, offload, bit-perfect USB | OkHttp through media3 | media3 session, notification, Android Auto, widget |
| The terminal app | `nori-host` | cpal | `nori-http` | MPRIS, macOS Now Playing |
| The desktop app | `nori-host` | cpal | `nori-http` | MPRIS, macOS Now Playing |
| The iPod app | `nori-host`, behind a C ABI | AURemoteIO | `nori-http` | `MPRemoteCommandCenter`, `MPNowPlayingInfoCenter` |
| The web client | `nori-host`, its engine driven by a worker | AudioWorklet, WebCodecs | `fetch` | The Media Session API |

Covers differ by client:

- Android's `CoverPixels` paints Bitmaps, and `CoverLoader` keeps them.
- The terminal app draws RGBA through ratatui-image (kitty, sixel, iTerm2 or half blocks).
- The desktop app draws RGBA in Slint.
- The iPod app hands RGBA to `CGImage` without a copy.
- The web client lets the browser draw the core's addresses, with colors from `cover_look`.

[iPod internals](ipod-internals.md) describes the iPod app's AURemoteIO output.

### Android around the engine

Android plays only through `nori-engine`, with its AudioTrack writer in `crates/android/src/track.rs`.
media3 stays for the session, the notification, Android Auto, the stream cache and downloads. OkHttp
(`android/core/src/main/kotlin/dev/nori/music/net/Http.kt`) is the one HTTP client, and the engine reads
song bytes through media3's cache over it.

Around the engine, Android keeps these in Kotlin:

| Part | Where |
|---|---|
| Per-device sound | `Outputs.kt`, `DeviceSound.kt` |
| Bit-perfect USB | `BitPerfect.kt` |
| The offline bridge's network watch | `OfflineBridge.kt` |
| A media3 player that mirrors the core's queue for the session | `EnginePlayer` in `RustPlayer.kt` |

Every queue edit goes through the core first. The mirror then applies what the core answers.

## Calling the core cheaply

Calls are coarse: one response or one page per call. Each client crosses the boundary its own way:

- **Rust clients** call the crates directly. Depend on `nori-core` with `default-features = false`, so
  no uniffi is built. Add the `plugins` feature for plugins' providers. The `ffi` feature of each domain
  crate is off by default.
- **Android** uses uniffi's JNI generator (`uniffi-bindgen-kotlin-jni`), not JNA. JNA costs far more
  per call and leaves garbage. `crates/android/build.rs` generates the scaffolding.
  `cargo run -p uniffi-bindgen -- bindings src:nori-android <dir>` writes the Kotlin: `nori-core`'s in
  `dev.nori.music.ffi`, and each domain crate's in its own package below it. A thread that the core
  starts is attached to the JVM once, under its own name. It finds the app's classes through the class
  loader that `JNI_OnLoad` kept.
- **Android's raw JNI doors** are for anything called per frame, per buffer or per list row. They take
  and return primitives, and they do not allocate. They are registered with `RegisterNatives` in
  `crates/android/src/lib.rs`. A primitive-only door is `@CriticalNative` (no env, no class). A short
  door over arrays or direct buffers that calls nothing back is `@FastNative`. Every raw pointer is
  checked for null and length before a slice is made of it. Doors only convert, and logic belongs in
  the core.
- **The iPod app** calls a hand-written C ABI (`crates/ios/src/lib.rs`, `ios/Sources/nori_ios.h`). Coarse
  calls answer JSON. Per-buffer work is the render callback.
- **The web client** calls wasm-bindgen exports that answer JSON.

Measure a crossing before you move work across it. The `bench` test command of the Android debug build
prints what each kind of crossing costs, next to the same work in Kotlin. [Building](building.md) shows
how to run it.

## Twins of Kotlin logic

Android keeps some logic in Kotlin because a crossing would cost more than the work. For each such
piece, the core carries a twin with the same answers, so the other clients get the rule without a port.

Test vectors hold each twin to its Kotlin original:

1. The originals are copied into `crates/core/testdata/twins/CoreTwins.kt`.
2. `tools/twins.sh` runs them on the JVM and writes `core_twins.tsv`.
3. `crates/core/tests/twins.rs` must match every row.

When an original changes, copy it again and run `tools/twins.sh`. Nothing that a client calls per row,
frame or buffer allocates: `crates/core/tests/twins.rs` and `crates/look/src/no_alloc.rs` count.

## What stays in the client

- **Gestures and per-frame animation math**: drags, springs, easing, flick velocity, and the decision a
  gesture ends in. A crossing costs more than the few multiplications, and other clients have other
  input.
- **What a gesture drew ahead of its answer**, such as a record slid in or a row dropped or swiped
  away. The client holds it until the player answered the command, whatever the outcome, and never for
  a guessed time. On Android each skip, move and removal gets a ticket (`PlayerConnection.ticket`), and
  `data/Pending.kt` lets it go once the state shows its outcome.
- **Drawing**: layout, text measurement, glyph caches, bitmaps, the lyrics' fill and glow. When the page
  redraws and how far the singing is are the core's (`LyricClock`).
- **Glue around platform objects**: the media session, the HTTP client's TLS setup, the network monitor,
  notifications, the Bitmap memory cache, and sorting the platform's errors into the core's kinds.
- **The perf build's counters** and **the debug build's test bridge**.
