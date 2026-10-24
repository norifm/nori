# iPod internals

Applies to nori 0.7 on the iPod touch 6th generation, iOS 12.2 or later, jailbroken. The user guide is
[iPod touch](https://norifm.com/docs/clients/ipod/).

The iPod client is a UIKit app (`ios/`) over a Rust static library (`crates/ios`, `nori-ios`) that links
`nori-host`, the session the terminal and desktop clients share. Rust decides; Swift draws, words and
holds the OS hooks. When they collide, the sound comes first, then the feel of the interface, then the
battery.

## The device

| | |
|---|---|
| Model | `iPod7,1` (`N102AP`), iPod touch 6th generation |
| OS | iOS 12.5.8 is the last. Jailbroken with checkra1n (semi-tethered) |
| SoC | Apple A8: 2 cores at 1.1 GHz, ARMv8-A with NEON; PowerVR GX6450 GPU |
| Memory | 1 GB, most of it held by the system |
| Audio | Cirrus Logic CS42L81: 3.5 mm jack, one mono speaker, Bluetooth 4.1 (A2DP) |
| Screen | 4", 1136 × 640 @2x: 320 × 568 pt, home button |
| Not there | Haptics, 3D Touch, Touch ID, cellular, GPS |
| Jailbreak tools used | AppSync Unified (fake-signed apps install), `uicache`, `uiopen`, dropbear SSH. No `sftp-server`, `awk` or `pgrep` |
| Swift runtime | In the OS from iOS 12.2, so the app ships none |

iOS 12 has UIKit with large titles, `UIViewPropertyAnimator`, `UIFontMetrics`, drag and drop and
`AVRoutePickerView`. It has no SwiftUI, SF Symbols, diffable data sources, context menus, system dark
mode or sheet presentation controller.

## Architecture

```
┌──────────────── nori.app (Swift, UIKit, iOS 12.2+) ────────────────┐
│ screens, gestures, words (Say.swift), MPNowPlayingInfoCenter,      │
│ MPRemoteCommandCenter, Bonjour (NetService), the system volume     │
├──────────── C ABI: ios/Sources/nori_ios.h, kept in step by hand ───┤
│ crates/ios (libnori_ios.a)                                         │
│   session slot · pages as JSON · covers · IosOutput · doors        │
├────────────────────────────────────────────────────────────────────┤
│ ios/Sound/NoriAudio.m: AVAudioSession, AURemoteIO, route,          │
│ interruption and media-reset notifications → Rust                  │
├────────────────────────────────────────────────────────────────────┤
│ nori-host → nori-engine (core) → nori-core and every domain crate; │
│ nori-http; nori-covers; nori-look                                  │
└────────────────────────────────────────────────────────────────────┘
```

- **The ABI is C.** `crates/uniffi-bindgen` generates Kotlin only, and the render callback cannot cross
  a generated binding, so every door is `extern "C"`. Coarse calls answer one page as JSON; per-buffer
  work is `nori_ios_render`.
- **`nori-host` without its `desktop` feature**: no cpal, no MPRIS, no mdns-sd. The client hands the
  session its `IosOutput`, `nori_http::Http` and Bonjour.
- **HTTP is Rust** (`nori-http`, ureq over rustls with webpki roots): one pool for API, covers and audio,
  the same as the desktop.
- **No `neural-beats`**: "Better beat detection" is not built for this device.
- **Data** lives in `Library/Application Support/nori/` (`nori.db`, the stream cache and downloads,
  `covers/`, `nori.log`), excluded from backup. Installed in `/Applications` and run as `mobile`, that is
  `/var/mobile/Library/Application Support/nori`.

### crates/ios

| Module | What it holds |
|---|---|
| `lib.rs` | The version and `nori_ios_free` |
| `session.rs` | The one open `nori_host::Session` in a process-wide slot (C entry points carry no handle); reports to Swift as `NoriReport` on a thread the library owns; the controls; background, foreground and memory warnings |
| `output.rs` | `IosOutput` over a `Sink`, and the render callback |
| `pages.rs` | Every page through `nori_ios_read(token, kind, arg)`, answered as JSON; song lists kept under the token, so play, enqueue and download name a token and an index; covers, lyrics, settings, the equalizer |
| `menu.rs` | The song menu (`menus::song_menu`), playlists, the sleep timer |
| `sound.rs` | Each output's sound and the AutoEQ browser, through the core's `device_*` calls |
| `lyrics.rs` | The core's `LyricClock` behind a handle the lyrics page holds |
| `theme.rs` | The `theme` and `season` settings worn on the device's date and zone, and pages dyed with a cover |
| `account.rs` | The login form (`providers`, `log_in`) and saved servers |
| `offer.rs` | The queue another device left on the server, offered on Home (`Client::queue_offer`) |
| `remote.rs` | Remote control: Bonjour through the app, the devices the music can move to |
| `jam.rs` | A jam this iPod is a guest in |

### ios/

| File | What it draws or does |
|---|---|
| `AppDelegate.swift` | The data directory, opening the session off the main thread, background and memory warnings, `nori://jam` links |
| `Bridge.swift` | `Core`: the reports turned into notifications, page and cover requests |
| `Shell.swift` | The tabs, the bottom chrome, the mini player, Search, Downloads |
| `Home.swift`, `Library.swift`, `Pages.swift`, `PageController.swift`, `Hero.swift` | Home's shelves and the queue offer, the library's sections, album, artist, playlist, genre, mix and smart pages |
| `Player.swift` | The player card, its transition, the lyrics sheet, `NowPlaying` |
| `Queue.swift` | The queue sheet, and a jam guest's view of the host's queue |
| `SongMenu.swift` | The song menu and the sleep timer |
| `Settings.swift`, `Sound.swift`, `Servers.swift`, `Login.swift`, `About.swift` | Settings, Appearance, the equalizer, per-output sound and AutoEQ, servers, the login form, credits and licenses |
| `Devices.swift` | The output picker, the devices sheet, joining a jam, Bonjour, the system volume |
| `Covers.swift` | `CoverCache` and `CoverView` |
| `Design.swift`, `Theme.swift`, `Glyphs.swift`, `Iconoir.swift` | Shapes, colors, fonts and the drawn icons |
| `Say.swift` | Every word, and `Fmt` for times and sizes |
| `nori_ios.h` | The bridging header |

## Building

Everything builds in Docker (`tools/ios-build/Dockerfile`), on Linux or a Mac, with no Xcode: swift.org's
Swift and clang, lld's Mach-O linker, Rust with the `aarch64-apple-ios` target, sccache and Procursus'
ldid. A Mac needs a Docker runtime (Docker Desktop, or `brew install colima docker`).

```sh
tools/ipod.sh                 # build; with an iPod on USB, also install and run
tools/ipod.sh build           # build/ios/nori.app and build/ios/nori-ipod-<version>.ipa
tools/ipod.sh install run     # just those steps
```

`install` and `run` need `sshpass` and libimobiledevice's `iproxy`. The iPod is reached over USB:
`iproxy` forwards port 2244 (`NORI_IPOD_PORT`) to checkra1n's dropbear on port 44, with password auth
(`NORI_IPOD_PASSWORD`, checkra1n's default unless set), because this dropbear stalls after accepting a key.

What `tools/ios-build/build.sh` does:

1. **Fetches once** into the Docker volume `nori-ios-cache`, checked by SHA-256: theos' iPhoneOS 16.5 SDK,
   and from swift.org's macOS toolchain the `libswiftCompatibility*.a` a target below iOS 13 links,
   Darwin's API notes and clang's `libclang_rt.ios.a`. The cache and cargo's target directory stay in
   the volume: the SDK is an archive of symlinks that cannot be unpacked onto a folder Docker shares from
   a Mac, and rustc dies with SIGBUS on mapped files there.
2. **Rust**: `cargo build --profile dist --target aarch64-apple-ios -p nori-ios` with `SDKROOT` and
   `IPHONEOS_DEPLOYMENT_TARGET=12.2`.
3. **The app**: one `clang` call for `NoriAudio.m` and one `swiftc` call (`-O -wmo`, the bridging header,
   `-lnori_ios -lc++`, lld, `-dead_strip`). There is no Xcode project. Linux's Swift resource directory
   carries its own `Dispatch` module, which clashes with the SDK's, so the build assembles one from its
   `shims` and `clang` and the iOS parts.
4. **Resources**: the launch image (`UILaunchImages`, so no Interface Builder), the icons, Inter and
   Literata (`UIAppFonts`), and the license texts from `app/src/main/assets/licences`.
5. **Signing**: `ldid -S ios/entitlements.plist`, then `Payload/nori.app` zipped as the `.ipa`.
   `tools/release.sh` builds the same `.ipa` beside the APK (`--no-ipod` leaves it out).

`install` copies the app to `/Applications/nori.app` and runs `uicache --path --respring`; `run` opens it
with `uiopen -b dev.nori.music` and prints the process and the data directory.

Toolchain traps the build answers:

- **The linker**: Xcode 26's produces binaries that crash on iOS 12.5. lld writes the classic load
  commands (dyld info, no chained fixups).
- **The Swift optimizer**: swift.org's compiler hoists the release of the empty array's storage above its
  retains, which only an immortal storage survives, and iOS 12's runtime has none (an abort in malloc
  after launch). The build turns off `retain-sinking` and `release-hoisting` (`-sil-disable-pass`). Check
  a new toolchain with `otool -tV -p '_$s4nori4CoreCACycfc'`: no `swift_release` after the
  `dictionaryLiteral` calls.
- **System-app entitlements**: launchd never spawns an app in `/Applications` without
  `platform-application` and `com.apple.private.security.no-container` (no crash report; `uiopen`
  returns 0 regardless). `ios/entitlements.plist` carries them, as Zebra and Filza do, plus
  `skip-library-validation` and `get-task-allow`.
- **macOS tar** adds AppleDouble `._` files unless `COPYFILE_DISABLE=1`; `iproxy` is started detached
  so it outlives the shell.

**Debugging**: no Xcode attaches to iOS 12. Read the core's log (`nori.log` in the data directory) over
SSH, crash reports in `/var/mobile/Library/Logs/CrashReporter/`, and symbolicate with `atos` against the
unstripped executable.

## Sound

iOS has no deep buffer like Android's AudioTrack: every path ends in the in-process I/O thread that the
media server wakes once per I/O buffer. So the output is AURemoteIO directly, with the longest buffer the
session grants.

`IosOutput` (`crates/ios/src/output.rs`) over the shim (`ios/Sound/NoriAudio.m`):

| | |
|---|---|
| Session | Category `playback`; preferred rate the song's, preferred I/O buffer 93 ms (about 11 wakes a second) |
| Rate | The unit's input is the rate Rust asks for, and RemoteIO converts if the hardware differs. The hardware moves to the preferred rate late, so the shim rebuilds the unit whenever its output side changes |
| Format | Float32 always (`takes_float`): high quality output on is float from decoder to DAC with no dither of nori's |
| Latency | `latency_us` is when the music pulled will be heard: the render timestamp's host time plus the route's `outputLatency`. `mixed_us` is the route's latency on Bluetooth |
| Equalizer in sight | `shallow` asks for a 10 ms buffer, so a band moved is heard at once; back to 93 ms after (`rules::sound_tuning`, through `nori_ios_tuning`) |
| Fades | The ring runs them per sample: `ramp` false, `flush` a no-op, `bursts` false; the pull that crosses the ring's low mark wakes the engine |
| Routes | `watch` maps the route's port to Speaker, Wired, Bluetooth (with its name) or Other, for per-output sound and the AutoEQ offer |
| Failure | After a media-services reset the unit is rebuilt once; if that fails, `failed` tells the engine |
| Pause | Stops the unit. After the engine's idle release (5 minutes paused) the output closes and the session goes inactive, so iOS can suspend the app |

The render path pulls and stamps; it neither locks nor allocates. The shim decides nothing: a route
lost (headphones pulled, Bluetooth gone) pauses, an interruption pauses and, when iOS says so, resumes,
both decided in Rust (`session::audio_changed`).

**Volume**: iOS keeps it. nori multiplies no samples for volume. The system volume (`outputVolume`,
observed) is passed to `nori_ios_volume` so other devices see it, and a volume set from another device
moves the system's through a hidden `MPVolumeView` (`SystemVolume`).

**Lock screen and controls** (`NowPlaying` in `Player.swift`): `MPRemoteCommandCenter` for play, pause,
toggle, next, previous and changing the position (the EarPods clicker, the lock screen, Control Center,
Bluetooth). `MPNowPlayingInfoCenter` gets the title, artist, album, duration, elapsed time, rate, place in
the queue and artwork at the player card's size, updated on each change, never on a timer.

Everything the engine does plays here too: gapless, crossfade, AutoMix, the equalizer, limiter and
headphone sound, volume leveling, the sleep timer, the offline bridge and fetching ahead in one burst.

## The interface

Nori Seasons, with Android as the reference; the layouts are the iPod's own.

- **Shell** (`ShellController`): Home, Library, Settings and Search, each a navigation stack with large
  titles in Literata. At the foot, one outlined pill holds the mini player over the tabs, drawn flat
  with no blur. The Settings tab shows only when the profile opens some settings.
- **Mini player**: cover, song, output, seal and play/pause, with no progress line. Tap or drag it up
  for the player card; swipe it sideways for the next or previous song.
- **Player card**: the cover edge to edge from the top, melting into the page behind the title; the
  title row with the seal and more; the seek bar and times; lyrics, previous, play, next and queue; the
  output line (the device's glyph and name, `AVRoutePickerView` behind it while remote control is off).
  The page is the season's paper dyed with the cover (not in Midnight). The times tick twice a second
  only while the card is shown, the app is active and music plays. No volume slider: the hardware keys
  set it.
- **Sheets**: the queue (History, the song playing, Playing next with Clear; hold and drag to move, swipe
  to remove with Undo; shuffle and repeat), the lyrics, the devices. The card and the sheets open with
  `CardTransition` under the finger and close with a drag down or a pull past the list's top
  (`DragToClose`).
- **Lyrics**: lines lit by `line_strength`, the sung line filled word by word with CoreText when the
  words are timed. Paced by the core's `LyricClock`: a display link only while a word fill moves, a
  one-shot timer between, nothing while paused, buffering or hidden. A tap seeks.
- **Pages**: Home's shelves and the queue offer; the library's sections as chips with Favorites,
  Downloads and Shuffle on a card above; album, artist, playlist, genre, mix and smart pages with the
  core's Play and Shuffle (`hero_buttons`); row swipes; the song menu on a long press; Search (the index
  at every key, the server after the core's delay, recent searches); Downloads.
- **Settings**: grouped as Android's (Playback, Sound, Appearance, Lyrics, Library, About) over the core's
  settings model (`nori_ios_settings`, `nori_ios_set`). Phone-only settings (metered network, offload,
  bit-perfect, USB) are left out. Also the equalizer, per-output sound and AutoEQ, servers and the login
  form, credits and licenses.
- **Covers**: asked at the view's pixel size and handed to Swift as RGBA that becomes a `CGImage`
  without a copy (`nori_ios_cover_release` lets go). The server is asked for the two renditions only
  (320 and 800 px). `CoverCache` (16 MB) shows the sharpest picture already decoded while a size comes,
  and empties on a memory warning. At each song change the covers around it are warmed into the disk
  cache.
- **Theme**: `Theme.swift` reads the palette from `nori_ios_look` and rebuilds the views when the theme
  or season changes and when the app becomes active on a new day. iOS 12 has no scheme of its own, so
  System is Night. Midnight is true black and wears no cover colors.
- **Type and icons**: Inter for everything and Literata for page titles (Android's variable fonts),
  scaled by Dynamic Type through `UIFontMetrics`. The icons are Iconoir's, with the seal and the seasons'
  kanji from Shippori Mincho's outlines, written as path data into `Iconoir.swift` by `tools/glyphs.py`
  and drawn by `Glyphs.swift`.
- **Motion**: nothing animates unless touched; the system's Reduce Motion turns fades and slides into
  direct changes. Animations run as Core Animation layer properties on the render server.
- **Accessibility**: VoiceOver labels on glyph buttons, rows and the drawn lyric lines.

## Remote control and jams

- **Remote control** (the `remoteControl` setting; off, nothing runs): the session's remote serves the
  LAN door and the relay as on the desktop. Bonjour (`NetService`, `Devices.swift`) announces the door
  and, while the core asks (`nori_ios_on_bonjour`), finds the account's other devices. The place
  published is the one heard. While another device plays, the player, the queue sheet and the lock
  screen show its music and control it. Paused in the background, iOS suspends the app and its door
  with it.
- **Jams**: the iPod joins as a guest. An invite link pasted in, or a `nori://jam` link opened from the
  relay's invite page, opens a guest profile in place of the user's own (`nori_host::jam_joined`);
  Leave in the queue sheet opens the user's again. The guest's app follows `ProfileRules`
  (`nori_ios_rules`: only this iPod's own settings, and no favorites, playlists or downloads). A tap or
  Play next asks the host. Listen Here plays the jam along through the engine's follow path.

## Memory, storage and network

- **Memory**: the engine may hold 96 MB of audio (`MEMORY_MB`), so a song's window is bounded by
  `load_control`. Decoded covers are capped at 16 MB in Swift. A memory warning empties `CoverCache` and
  the core's decoded covers (`nori_ios_memory_warning`). Pages are read, drawn and dropped.
- **Storage**: the stream cache is held to the `cache_mb` setting, downloads have no cap; both use the
  core's eviction order.
- **Network**: Wi-Fi only, so the network is always unmetered.
- **Background**: `applicationDidEnterBackground` saves the queue (`QueueMoment::Closing`). Downloads and
  AutoMix measuring run only while the app is open or playing; the Downloads page says so.

## Testing and measuring

- `cargo test -p nori-ios` runs the output on a simulated sink, the session on `WavOutput`, the doors
  the screens read, and remote control on the virtual clock. [Testing](testing.md#the-ipod) lists what
  only the device shows.
- `tools/ipod-bench.sh nori|music` measures battery and CPU for 30 minutes, screen off, unplugged, over
  SSH on Wi-Fi (`NORI_IPOD_HOST`), once for nori and once for Apple's Music app on the same songs.

## Still open

- **Settings pages**: the iPod picks its rows in Swift (`SettingsGroup`, `Say.settings`) instead of
  reading the core's `settings_pages`; there is no `SettingsClient` for it.
- **Motion setting**: the iPod follows the system's Reduce Motion, not nori's `motion` (Full, Reduced,
  E-ink).
- **Profiles**: no YouTube Music sign-in, custom headers, self-signed certificates or client
  certificates (`nori-http` applies none of them).
- **Jams**: the iPod cannot host one.
- **Background downloads**: none while the app is suspended (no `NSURLSession` background session).
- **Offload**: none. An `OffloadOutput` over `AVQueuePlayer` could let the media server play whole files
  while nothing touches the samples; it is untried.
- **Perf recorder**: not wired. iOS gives no per-thread count of context switches, so the bench reads
  the app's CPU and memory from outside.
- **Words**: English only, in `Say.swift`; `Fmt` has no test vectors against Android's `Fmt.kt`.
