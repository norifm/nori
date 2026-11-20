<p align="center">
  <img src="docs/brand/nori-wordmark.svg" alt="nori" width="280">
</p>

<p align="center">
  A music player for your own Navidrome or Subsonic server, YouTube Music and SoundCloud, on Android
  phones and TVs, Windows, macOS, Linux, the terminal and the iPod touch.
</p>

<p align="center">
  <a href="https://github.com/norifm/nori/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/norifm/nori?style=flat-square&label=release&color=b64d1d"></a>
  <img alt="Android · Android TV · Windows · macOS · Linux · terminal · iPod" src="https://img.shields.io/badge/Android%20%C2%B7%20Android%20TV%20%C2%B7%20Windows%20%C2%B7%20macOS%20%C2%B7%20Linux%20%C2%B7%20terminal%20%C2%B7%20iPod-555?style=flat-square">
  <a href="LICENSE"><img alt="AGPL-3.0-or-later" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-555?style=flat-square"></a>
</p>

<p align="center">
  <img src="docs/screenshots/casiopea-album-page.png" width="260" alt="Casiopea's album playing Space Road">&nbsp;
  <img src="docs/screenshots/casiopea-full-player.png" width="260" alt="The full player halfway through Space Road">
</p>

- **Sleeps between bursts.** Music is decoded ahead and the CPU sleeps in between. Nothing ticks with
  the screen off, and on Android the audio chip can decode on its own.
- **Sounds right.** Gapless albums, AutoMix transitions on the beat, an equalizer with AutoEQ curves,
  and a sound profile for each output.
- **Finds the words.** Your server's lyrics and eleven online services, asked at once and timed
  against the singing, word by word where they have it.
- **Plays together.** Control one device from another, or host a jam that guests join with a link or
  QR code.
- **Grows with plugins.** YouTube Music and SoundCloud fill your library with their albums, playlists
  and likes. Others add battery and noise control for Soundcore and Sony headphones, new sounds and
  AutoMix tuning, from nori's own repo or one you trust.
- **One core, every screen.** One Rust core plays, queues and decides; each app only draws. Every app
  wears the same design, Nori Seasons.
- **Yours.** No nori account and no analytics. Downloads play offline, and each profile can scrobble to
  ListenBrainz, Libre.fm or Last.fm.

nori is and always will be free. You can support it from Settings in the app.

## Install

Every file is on the [latest release](https://github.com/norifm/nori/releases/latest), and the
[download page](https://norifm.com/#download) picks the right one for you.

| client | get it |
|---|---|
| Android 8.0 or later, phones and Android TV | `nori-music-<version>-arm64-v8a.apk` for most devices, `armeabi-v7a` for 32-bit phones and TV boxes, `x86_64` for emulators and Chromebooks |
| desktop and terminal on Windows, x86-64 | `nori-desktop-<version>-windows-x86_64.zip` or `nori-cli-…`; unpack and run |
| desktop and terminal on macOS and Linux, x86-64 and ARM64 | `.dmg`, `.deb`, AppImage, Flatpak or archive; see the [install guide](https://norifm.com/docs/start/install/#desktop-and-terminal) |
| iPod touch 6th generation, iOS 12.2 or later, jailbroken | `nori-ipod-<version>.ipa`; see the [iPod guide](https://norifm.com/docs/clients/ipod/) |

A web client is in development in `web/` and is not released yet.

## Links

[Website](https://norifm.com) · [Docs](https://norifm.com/docs/) · [Changelog](CHANGELOG.md) ·
[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Issues](https://github.com/norifm/nori/issues)

## Build

```sh
cargo test -j4 --workspace                        # Rust tests, doctests included
cargo run --release -p nori-desktop               # the desktop client
cargo run --release -p nori-cli                   # the terminal client
tools/check.sh android -PrustTargets=arm64-v8a    # a debug Android APK
```

- Run Cargo with at most four jobs.
- Android needs the Android SDK and NDK, the Rust Android targets and `cargo-ndk`.
- The desktop on Debian or Ubuntu needs `pkg-config` and `libasound2-dev libfontconfig1-dev
  libfreetype6-dev libegl1-mesa-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libx11-dev
  libwebkit2gtk-4.1-dev libgtk-3-dev`.
- Windows needs the MSVC build tools (Visual Studio's C++ workload).
- More, including the optional Bazel backend: [Building](docs/developers/building.md).

## License

AGPL-3.0-or-later. You may use, change and sell nori, but whoever ships it or serves it over a network
must publish their source under the same license. See [LICENSE](LICENSE). Third-party licenses are in
[NOTICE](NOTICE) and in the app under Settings, About, Licenses.
