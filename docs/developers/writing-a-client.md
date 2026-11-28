# Writing a client

A nori client links the Rust core and writes only what its platform forces on it: drawing, words and the
operating system's hooks. This page says what the core already does, what every client still builds,
and how to call the core cheaply.

It covers every client:

- Android: `android/app/`, `android/core/` and `crates/android`.
- The desktop app: `crates/desktop` and `crates/desktop-ui`.
- The terminal app: `crates/cli`.
- The iPod app: `crates/ios` and `ios/`.
- The web client, in development: `crates/web` and `web/`.

The [rules that hold](architecture.md#rules-that-hold) in Architecture hold for every client. Read them
first. Before you write logic in a client, look for it in `crates/host` or the core. If another client
would need it too, it belongs in Rust.

## Start from nori-host

`nori-host` (`crates/host`) is the session that the desktop app, the terminal app and the iPod app
share. Each of them is an interface over its `Session` (`crates/host/src/session.rs`). The web client
runs the same session in a browser worker with `Session::open_driven`, where the caller turns the engine
instead of a thread. Android does the same work through the core directly and is moving onto
`nori-host`.

`Session::open` takes an `Open` with what only the platform can give:

- The data directory, for `nori.db`, the stream cache, downloads and covers.
- Its HTTP (`Net`): a `Transport` for the API and covers, and a `ByteSource` for audio.
- The sound card (an `AudioOutput`) and the volume.
- Its mDNS and media controls, when it has them, and its own decoders.
- The profile kinds it opens (`Kinds`, see [Providers](providers.md)).

`memory_mb` bounds the audio held ahead. The engine holds a quarter of it, at least 16 MB and at most
48 MB (`nori_player::transport::load_control`). It is 256 on the desktop, 96 on the iPod and 128 on the
web.

### What the session does

Once open, the session does this work for the client:

- It plays the core's queue through `nori-engine`, with the core's transition planner, settings, stream
  addresses, per-device sound (`CoreApp::per_device`) and AutoMix measuring (`CoreApp::measuring`).
- It keeps the stream cache and downloads (`Store`, `Downloader`), the cover loader and search.
- It runs the core's rules at each moment, as
  [What the core decides at each moment](#what-the-core-decides-at-each-moment) lists them.
- It reads pages (`crates/host/src/pages.rs`). Home comes whole from what the device keeps
  (`Client::home_kept`, no request), then each shelf's server answer (`Client::read_refreshed`), so a
  client draws Home's first frame from one read.
- It says what the player shows (`crates/host/src/player.rs`: `Now`, `QueueShown`, `Playback`). The
  answer is the same for every client and for the system's media controls.
- It runs remote control and jams while they are on (`crates/host/src/remote.rs`), and joins a jam as a
  guest. A LAN jam's guest searches and streams through its host, so any native client joins any LAN
  jam, with or without plugins.
- It looks for the account's device that plays while the client says it is in sight (`in_sight`).

The session reports through `Said`, from any thread, and the client words each report. After
`Said::Working`, the client reads `progress` once a second until it answers `None`. A client that
replaces its session calls `close_for` with the profile that takes over.

### The desktop feature

The `desktop` feature of `nori-host` is on by default. The iPod app and the web client turn it off. It
adds what the terminal app and the desktop app share:

- the cpal sound card (`nori-output-cpal`) and ureq's HTTP (`nori-http`)
- MPRIS on Linux and Now Playing on macOS (`nori-mpris`)
- mdns-sd, the plugin host, and Bluetooth on Linux and macOS
- `nori_host::desktop::Root`, which logs in, removes profiles, installs plugins and makes backups

## What the core already does

Link the core's crates and call them. Never write them again in a client. `nori-core` re-exports every
domain crate by module name: `nori_core::playlist` is `nori_queue::playlist`.
[Architecture](architecture.md#the-parts) says what each crate owns.

The core also owns the parts a client is most tempted to write itself:

- What each page shows, as data: `nori-library` (`pages`, `rows`, `menus`, `car`).
- Numbers for display: `nori_model::numbers`.
- The settings pages: `nori_settings::settings_pages`.
- Colors from covers, the seasons and the seek bar's pacing: `nori-look`.
- The scrobbling accounts and their add form: `nori_core::scrobblers` (`scrobbler_kinds`,
  `scrobbler_fields`, `scrobbler_sign_in`).

## What every client builds

### Network

- **HTTP.** Implement `Transport` (`crates/net/src/transport.rs`) and the engine's `ByteSource`, or link
  `nori-http`, which is ureq over rustls with webpki roots. `Transport::send` carries a third party's
  headers and body as given, for lyrics services and Apple's catalog.
- **Server headers.** `ServerHosts::policy(url)` says whether a request goes to the music server. Such
  a request carries the profile's headers and its TLS exceptions, and it may be limited to an unmetered
  network. See [The profile's connection options](#the-profiles-connection-options).
- **The profile's proxy.** `SavedServer::proxy` is a SOCKS5 proxy (`nori_core::proxy`). See
  [The profile's proxy](#the-profiles-proxy).
- **One connection pool** serves the API, covers and audio, so the radio wakes once. With `nori-http`,
  each profile has its own pool.
- **Network state.** `Transport::network` answers whether the network is metered.

### The profile's connection options

A profile's form lists its fields in `ProviderInfo::fields`, each plain or folded under the form's
advanced part. A client draws what `form_fields(info, platform, profile, adding)` returns, in that
order, and keeps no list of its own. `form_fields` leaves out these fields:

- Wi-Fi only on `Platform::Computer` and `Platform::Ipod`. The desktop and terminal apps say every
  network is unmetered, and the iPod has only Wi-Fi.
- The user and password, while an API key is given.
- The name, when the form edits a saved profile. A saved profile is renamed from its menu.

A saved profile's form opens its advanced part when `advanced_shown` says one of its fields has a value.
On Save, `form_taken` lays the fields shown over the saved profile, so a value the form
does not show stays.

Each field is one of three kinds of line. A text is typed, the extra headers as one line each. A switch
turns legacy authentication, a self-signed certificate or Wi-Fi only. An entry shows what is set and
opens a sheet: the client certificate and the proxy.

The options act in these places:

| Option | Where it acts |
| --- | --- |
| Second address | The core: `Client::choose_address` after a failed request, `login_check` when signing in |
| API key, legacy authentication | The core's request signing (`ServerConfig`) |
| Extra headers | The transport, on requests that `ServerHosts::policy` calls the server's |
| Self-signed certificate | The transport: no certificate check on the server's requests |
| Client certificate | The transport: the PKCS#12 file at `<data>/certs/<client_cert>`, sent to the server |
| Wi-Fi only | Android's transport, which refuses the server's requests on a metered network |

Each transport decides again at every hop of a redirect whether a request is the server's. A hop to
the server gets the headers and the TLS exceptions, and a hop to another host gets neither:

- `nori-http` (`Http::for_profile(profile, data)`) follows redirects itself. The server's hops go over an
  agent with the profile's TLS config, built once per profile, and every other hop over a plain agent.
- Android's OkHttp adds the headers in a network interceptor, which runs at each hop. Its trust manager
  and key manager ask `ServerHosts` about the host of each TLS handshake.

`nori-cert` reads the `.p12` file and checks its MAC first. It reads bags in the clear, PBES2 with AES
(OpenSSL 3), and PKCS#12's 3DES and RC2 with SHA-1 (older exports). Every client calls it:

| Function | What it does |
| --- | --- |
| `import_client_cert` | Checks a file with its password, or gives a `CertError`. Keeps it as `<profile id>-<hash>.p12`, so the file of the saved profile stays until Save |
| `client_identity` | Gives the key and chain a transport sends |
| `forget_unused_client_certs` | Removes the files that no saved profile uses. A client calls it as it starts, before a form can import a file |

### The profile's proxy

`SavedServer::proxy` is a SOCKS5 proxy, with an optional login (RFC 1929). The proxy looks up the
names, so nori never looks up a name to decide where a connection goes.

Every kind's form has the proxy as one advanced field, `FormField::Proxy` (`PROXY_FIELD`). A client
draws it as an entry under the form's advanced part. The entry shows `proxy_address`, or a word for
none. It opens a sheet with the proxy's address, user and password. The sheet hides the password as
the form hides the profile's own. On Save, the client calls `proxy_from_form`. The sheet stays open
and shows a `ProxyError` in the client's words, or closes and keeps the proxy for the form to send. An
empty address gives no proxy, so the sheet's Remove and an emptied address do the same.

A saved profile's form opens its advanced part when one of the advanced fields that the client draws
has a value (`advanced_shown`).

`skips_proxy(host)` decides which connections go around the proxy. A proxy elsewhere cannot reach the
user's own network, so local remote control and jams would break through it.

| Destination | Goes |
| --- | --- |
| `localhost`, and a `.local` name (mDNS) | Directly |
| A loopback, private (RFC 1918), shared (RFC 6598, `100.64.0.0/10`), link-local or unique-local address, IPv4 or IPv6 | Directly |
| Any other address | Through the proxy |
| Any other name | Through the proxy, which looks it up |

The clients apply the rule as follows:

- `Http::for_profile` makes `nori-http`'s agent for a profile. Its connector asks `skips_proxy`, and
  does the SOCKS5 handshake itself (`crates/http/src/socks.rs`). It applies the rule to a SOCKS5 proxy
  from the environment too.
- Android gives OkHttp a `ProxySelector` that asks `skips_proxy`. Java signs in to a SOCKS proxy only
  through the process's one `Authenticator`. So `Http` sets one when it first builds a client with a
  proxy that has a login, and it answers those proxies alone. The account of another profile, looked at
  apart, has a client of its own (`Http.transportOf`).
- Android's sign-in page goes through the proxy with androidx.webkit's `ProxyController`, without the
  login, and bypasses `proxy_bypass_rules()`. The desktop and terminal apps pass the proxy to the
  sign-in window, where wry gives it to WebKitGTK and WebView2 without a bypass list. Sign-in pages are
  public pages, so the bypass does not matter there.
- WKWebView takes a proxy only from macOS 14, and the desktop app runs from macOS 11. So on macOS, a
  profile with a proxy opens no sign-in window: `SignInWindow::open` returns
  `NoWindow::ProxyUnsupported`, and the client says that the account signs in by pasting.
- What reaches the web outside a session goes over the active profile's connections too: a jam joined
  (`Root::http` on the desktop and terminal apps, the session's `Client::transport` on the iPod), and
  plugins' requests before a session opens.
- A browser's `fetch` takes no proxy, so the web client has no proxy fields.

Plugins' sockets, the cast sinks, the LAN door and mDNS never use the proxy: they open their own
sockets. Plugins' HTTP requests, remote-control polls, a provider's streams and a moving cover use the
profile's transport, so the rule above applies to them. Plugins serve one profile at a time. A check
of another profile (`check_profile`) borrows them only for the check, and an account looked at apart
gives them back with `Client::take_plugins_back`.

A proxy's failure has a `FailureKind` of its own, which each client words:

| Kind | When |
| --- | --- |
| `ProxyUnreachable` | The proxy does not take the connection, or does not answer as SOCKS5 |
| `ProxyLogin` | The proxy refuses the login, or asks for one it was not given |
| `ProxyNoRoute` | The proxy cannot connect to the server |

`nori-http` returns these kinds from its connector. On Android, OkHttp reports each failure of a
connection through the proxy to `EventListener.connectFailed`, and only those become a
`ProxyFailureException`. Java's SOCKS client throws all of them as a plain `SocketException`, so
`Http.kt`'s `proxyFailure` tells the login and the proxy's reply apart by their texts.

### Sound

Implement `nori_engine::AudioOutput`, or use `nori-output-cpal` (PipeWire or ALSA, CoreAudio, WASAPI).
The output pulls from the engine's `Feed`, which is lock-free and does not allocate.

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

Three more parts are off unless the client gives them:

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

The media controls are `nori-mpris` on Linux and macOS, the media session on Android, and
`MPRemoteCommandCenter` with `MPNowPlayingInfoCenter` on the iPod. Media keys go to
`Handle::media_key`, and a toggle or a skip goes to the core's `media_button`, where a plugin can
decide.

While another device plays, MPRIS, Now Playing and the iPod's lock screen show nothing
(`Playback::shown_to_system`). The system takes them for sound that comes out of this device, and it
tells Bluetooth headsets so. A headset joined to both devices would then pause and resume this device,
when the presses belong to the other one. Android instead shows the other device as remote playback
(media3 `DeviceInfo.PLAYBACK_TYPE_REMOTE`, `remote/RemoteDevicePlayer.kt`), and its own player is
released meanwhile.

### Audio focus and the system's events

The client notices the event. The engine and the core decide what follows.

- **Audio focus.** Forward each focus change to `Engine::focus` (`nori_player::focus`). The engine pauses
  and resumes, asks a plugin (`audio-focus`), and sends `Event::Focus`. A platform with a focus to
  request, such as Android, gives the engine a `FocusRequest`.
- **Headphones pulled out and route changes.** The engine and the core answer with `pause`,
  `pause_now`, or the device's sound profile.
- **Going to the background.** Save the queue (`QueueMoment::Closing`).
- **A long pause.** The engine releases its output and sends `Event::LongPause`. A platform that keeps
  a playback session in the foreground releases it then.

A feature only one platform can have, such as Android Auto, widgets or the iPod's lock screen, may live
wholly in its client. What it shows still comes from the core.

### Covers

- Ask the `nori-covers` `Loader` for a cover at the view's pixel size, and draw what comes back. `Paint`
  says what a decoded cover becomes: tight RGBA (`Loader::new`) or a platform picture
  (`Loader::with_paint`). Dropping the `Ticket` cancels a request.
- The server is asked for two renditions only, 320 and 800 px (`cover_rendition`). So every view of a
  cover shares one download. A cover that is a provider's URL takes the smallest size its image
  server keeps ready at least each rendition (`Provider::cover_sizes`).
- A page's head shows the tile's picture while its larger rendition comes. `Loader::request_standing_in`,
  given the core's `cover_addresses`, answers with the smaller rendition first, from memory or the disk,
  then with the larger one. On `nori-host`, `Handle::cover` does this and sends page colors with each
  picture. When the larger picture's colors are alike the smaller one's (`CoverColours::alike`), it
  sends the smaller one's again, so the page changes color once. A client that keeps the pictures
  itself, as Android keeps Bitmaps, asks `nori_covers::stand_in` which of them stands in.
- A page opened from a tile shows what the tile knew (its title, cover, colors and counts) until the page
  is read. The client keeps the last few tiles opened, as Android's `Nav` does.
- Page colors come from `nori_look::cover::derive` (`nori_host::derive` on `nori-host`). Draw them and
  never compute them again.
- nori does not decode HEIF or AVIF. The view keeps its placeholder.

### The interface

- **Screens** take each page's contents from the core as data.
- **Words** are the client's: labels, counts and plurals, times, sizes. The core hands over counts,
  seconds and kinds (`PlaybackError`, `SongAction`, `ReleaseKind` and the like). Each client's words
  file is listed under [Rules that hold](architecture.md#rules-that-hold).
- **Settings.** `settings_pages` gives a client (`SettingsClient`) its pages, sections and rows, each
  with its spec, its value, and whether it shows and is enabled. The client words and draws them, and
  changes a setting by name (`setting_set`). It applies the `SettingChange` that answers to the engine
  (`Session::setting` does this on `nori-host`).
- **Backup.** The client picks the file and asks for the password. `backup_make` gives the sealed bytes,
  and `backup_restore` answers a `SettingChange` to apply, as a setting does. Backups need the `plugins`
  feature of `nori-core`, so the iPod app and the web client have none.
- **Motion.** The `motion` setting is Full, Reduced or E-ink, and nori's choice wins over the system's.
  The durations and curves of moves, fades and the page stack are the core's `Stage`. A gesture's feel
  is the client's.
- **A page that is still loading.** `sketch` gives the parts a loading page shows: a head, shelves, a
  grid or rows, each with the shape of its pictures. The client draws them where the real items will
  stand, in the look's `SKETCH` color. It shows them only after the read takes longer than the `Stage`'s
  `quick_load_ms`. With Motion Full, a band in `SKETCH_SHEEN` crosses each placeholder every `sheen_ms`.
  When the answer comes, it fades in over the placeholders in `fade_ms`. With Reduced or E-ink motion,
  the placeholders stay still and the answer replaces them directly. A page with a saved copy shows that
  copy at once, with no placeholders.
- **Appearance.** The `theme` setting is a `ThemeMode` and the `season` setting a `ThemeSeason`.
  `theme_palette` gives the palette for the date, and `covers_tint_pages` is false only for Midnight.
  The client resolves System from the OS. The iPod has no system scheme, so the iPod app reads System as
  Night.
- **Drawing cost.** Redraw only when something visible changes. A paused player draws nothing.
- **A perf recorder** is optional. It reads the platform's counters into `nori-perf` (the perf build in
  [Building](building.md)).

## What the core decides at each moment

The client notices the moment, asks the core, and does what the core answers. On `nori-host` the
session does all of this. Android calls the same functions from Kotlin.

| Moment | The core's answer | On `nori-host` |
|---|---|---|
| A song arrives | `queue::Session::song_arrived` gives `SongSteps`: save, autofill, bridge, measure | The session |
| The sleep timer | `queue::Session::sleep_pick`, `sleep_due`, `sleep_stopped` | `Handle::sleep_pick` |
| Where the music plays | `destination` gives one `Destination` | `Handle::destination` |
| The songs to fetch ahead | `Client::precache_targets` | The engine |
| A new output | `Core::device_arrive`, then `Client::device_curve` for AutoEQ | `Session::curve_answer` |
| A sound page in sight | `tuning::SoundTuning` gives the buffer depth | `Session::sound_page` |
| The queue to save | `rules::queue_keep(QueueMoment)` | The session |
| A song the network would not bring | `Core::bridge_take` gives a `BridgeTake` | The session |
| A song removed or the queue cleared | `undo::UNDO_MS`, `playlist_restore`, `playlist_restore_all` | `Session::put_back`, `put_back_cleared` |
| The queue's own actions | `menus::queue_actions` | The same |
| A screen's read | `Client::read_cached(read, PageShown)` | `crates/host/src/pages.rs` |
| A list read a page at a time | `paged::Paged` | The same |
| An artist page's about part, read after the page | `Read::ArtistAbout`, then `pages::ArtistAbout::of` | `PageAsk::ArtistAbout` |
| An artist page's top songs | `library::artist_top_songs` | Android only |
| A keystroke in search | `SearchSession`, whose `SearchView` holds a `SearchShown` | `search_typed`, `search_server` |
| A search sent (Enter) or a result of it used | `Core::search_remember_recent` | `Handle::search_used` |
| What an empty search offers | `Core::genre_tiles(search_genres_shown())`, `Core::browse_decades`; a decade's songs, `Core::decade_songs` | `PageAsk::Browse`, `PageAsk::Decades`, `PageAsk::Decade` |
| The search's results page | `SearchView::layout` (`search_layout`), `kinds` and `kind` for the chips | `search_pick`, `search_narrow` |
| Home's queue offer | `Client::queue_offer_kept` at once, then `Client::queue_offer`, kept by `offer::Offered` | `Handle::queue_offer_kept`, `Handle::queue_offer` |
| Lyrics for the song heard | `Client::lyrics_for`, `lyrics_follow::LyricsHeard` | `Handle::lyrics_heard`, `Said::Lyrics` |
| Adding a profile, signing in | `providers::flow`: `AddFlow`, `SignInFlow` | `Account::signed_in` |
| The first run | `providers::flow::WelcomeFlow`, then `setup::setup_steps`, `setup_answers`, `setup_settings` and `setup_covers` | The client |
| A favorite | `Client::star` | The same |
| A playlist kept downloaded | `Core::download_playlist`, `Client::kept_playlists_sync` | `Handle::download_playlist` |

These rules hold behind the answers:

- **A read shows the stored answer first.** The server's answer replaces it only when it differs. A
  failure is an error only when nothing was stored.
- **A paged list has one page on its way at a time.** A page that failed is read again on the next ask,
  so the list does not end.
- **A search asks the server once typing pauses.** Each keystroke is answered from the offline index,
  and an ask that newer typing replaced drops its request.
- **A search becomes a recent search only when it is used.** Enter sends it, or the user opens or plays
  one of its results. A typing pause is neither.
- **A song found by search plays alone.** The search's results are no album. `Handle::play` with a
  `Search` origin queues only that song.
- **The core lays out a search's results.** On All: the top result, the best few songs beside it, then
  a row of albums, artists and playlists. A kind chip shows that kind whole, in place; a new query starts
  on All. A client draws the layout and keeps no order of its own.
- **An undo lasts `UNDO_MS`.** A client that offers the undo shows it for that long. After that,
  `playlist_restore` refuses.
- **A favorite marks at once.** `Client::star_in` hands the client the profile's marks before it asks
  the item's source, and puts the old mark back if the source refuses. A client looks a seal up by the
  item's key (`StarMarks::starred`). Where the account cannot keep favorites, the core keeps them on this
  device ([Providers](providers.md)).
- **Only the device that plays looks lyrics up.** It hears the song to time them against. A device that
  shows another device's playback shows the lyrics that device sends.
- **Keeping a playlist deletes only what keeping brought.** A song that no kept playlist holds, and that
  keeping downloaded, is deleted.
- **The first run starts with no profile.** The welcome takes any number of sources: the kinds there
  are and the provider plugins that repos offer. The plugins picked install and turn on without their
  sheet. Then each source signs in: one picked opens its own page, several are listed, and the app opens
  under them once a profile is in. The questions follow (the mobile data one only where the device has
  it), each answer changed by name as any setting is. The AutoMix question plays the same four covers on
  every client, and the look is Settings' own appearance and season rows. Profiles added later, and
  restored ones, skip it.
- **The profiles keep the user's order** (`StoredPrefs::servers`). Every list and switcher shows them in
  that order, and switching keeps it.

## The clients today

Each client fills the same parts with its platform's pieces:

| Client | Session | Sound | HTTP | Media controls | Covers |
|---|---|---|---|---|---|
| Android | The core and `nori-engine`, from Kotlin | AudioTrack, MediaCodec, offload, bit-perfect USB | OkHttp through media3 | media3 session, Android Auto, widget | Bitmaps (`CoverPixels`) |
| The terminal app | `nori-host` | cpal | `nori-http` | MPRIS, macOS Now Playing | RGBA, through ratatui-image |
| The desktop app | `nori-host` | cpal | `nori-http` | MPRIS, macOS Now Playing | RGBA, in Slint |
| The iPod app | `nori-host`, behind a C ABI | AURemoteIO | `nori-http` | `MPRemoteCommandCenter` | RGBA, into `CGImage` |
| The web client | `nori-host`, its engine driven by a worker | AudioWorklet, WebCodecs | `fetch` | The Media Session API | The browser, from addresses |

[iPod internals](ipod-internals.md) describes the iPod app.

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
  no uniffi is built. Add the `plugins` feature for plugins' providers.
- **Android** uses uniffi's JNI generator (`uniffi-bindgen-kotlin-jni`), not JNA, which costs far more
  per call and leaves garbage. `crates/android/build.rs` generates the scaffolding, and
  `cargo run -p uniffi-bindgen -- bindings src:nori-android <dir>` writes the Kotlin.
- **Android's raw JNI doors** are for anything called per frame, per buffer or per list row. They take
  and return primitives, and they do not allocate. They are registered with `RegisterNatives` in
  `crates/android/src/lib.rs`, never exported by a `Java_` name. A primitive-only door is
  `@CriticalNative` (no env, no class). A short door over arrays or direct buffers that calls nothing
  back is `@FastNative`. The Kotlin `external fun` and the Rust function must agree. Every raw pointer
  is checked for null and length before a slice is made of it. Doors only convert.
- **The iPod app** calls a hand-written C ABI (`crates/ios/src/lib.rs`, `ios/Sources/nori_ios.h`). Coarse
  calls answer JSON. Per-buffer work is the render callback.
- **The web client** calls wasm-bindgen exports that answer JSON.

## What stays in the client

Work that a crossing would make slower stays in the client. Moving it needs a measurement that shows the
crossing is at least as fast. The `bench` test command of the Android debug build prints what each kind
of crossing costs. It also prints the same work in Kotlin. [Building](building.md) shows how to run it.

- **Gestures and per-frame animation math**: drags, springs, easing, flick velocity, and the decision a
  gesture ends in.
- **What a gesture drew ahead of its answer**, such as a row swiped away. The client holds it until the
  player answered the command, whatever the outcome, and never for a guessed time. On Android each
  skip, move and removal gets a ticket (`PlayerConnection.ticket`), and `data/Pending.kt` lets it go
  once the state shows its outcome.
- **Drawing**: layout, text measurement, glyph caches, bitmaps, the lyrics' fill and glow. When the page
  redraws and how far the singing is are the core's (`LyricClock`).
- **Glue around platform objects**: the media session, the HTTP client's TLS setup, the network monitor,
  notifications and the Bitmap memory cache. Sorting the platform's errors into the core's kinds is glue
  too.

### Twins of Kotlin logic

Android keeps some logic in Kotlin for speed. For each such piece, the core carries a twin with the
same answers, so the other clients get the rule without a port. Test vectors hold each twin to its original:

1. The originals are copied into `crates/core/testdata/twins/CoreTwins.kt`.
2. `tools/twins.sh` runs them on the JVM and writes `core_twins.tsv`.
3. `crates/core/tests/twins.rs` must match every row.

When an original changes, copy it again and run `tools/twins.sh`.
