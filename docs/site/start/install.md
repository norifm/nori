---
title: Install
description: Getting nori onto an Android phone, a desktop, a terminal or an iPod touch.
sidebar:
  order: 2
---

Applies to nori 0.6. Releases are on [GitHub](https://github.com/norifm/nori/releases).

## Android

Android 8.0 or later. Download `nori-music-<version>.apk` from the latest release and open it on the
phone; Android asks once to allow installing from the app you opened it with. The app checks GitHub for
newer releases at most once a day, unless you turn off Check automatically in the Updates section of
Settings.

**Android Auto** hides apps that were not installed from the Play Store. To see nori in the car, open
the Android Auto settings on the phone, tap Version ten times to unlock developer settings, then in the ⋮
menu open Developer settings and turn on Unknown sources.

## Desktop and terminal

Releases include desktop and terminal packages for Linux and macOS, on x86-64 and ARM64.
Choose `nori-desktop` for the window or `nori-cli` for the terminal. Linux archives, `.deb`
packages, and AppImages use Ubuntu 22.04 and need glibc 2.35 or later. Linux `.deb` packages install with
`sudo apt install ./nori-desktop_<version>_<architecture>.deb` (or the CLI package). Extract a
Linux archive and run its binary on other distributions. For AppImage, download the desktop
`.AppImage` for your processor, make it executable with `chmod +x`, and open it. The desktop and terminal
archives and the AppImage use the system's WebKitGTK 4.1 (`libwebkit2gtk-4.1-0` on Debian and Ubuntu,
`webkit2gtk4.1` on Fedora) for the sign-in window; most desktops have it, and the `.deb` packages and
Flatpak bring it. For a machine without a desktop, build the terminal client without the window
(`cargo build --release -p nori-cli --no-default-features`); it then signs in with pasted cookies.
For Flatpak, install the desktop `.flatpak` with `flatpak install --user ./nori-desktop-<version>-linux-<architecture>.flatpak`
and run `flatpak run dev.nori.desktop`. The bundle also includes the terminal client:
`flatpak run --command=nori-cli dev.nori.desktop`. Flatpak installs its GNOME runtime
from Flathub and keeps its data inside the app sandbox. On macOS, open the desktop `.dmg` and follow
the arrow: drag nori onto the Applications folder. Then open nori from Applications and eject the
disk image. The CLI archive contains `nori-cli`, which you can place on your PATH.
macOS bundles are ad-hoc signed; approve the downloaded app in Privacy & Security when macOS asks.

To build from source, with [Rust](https://rustup.rs) installed:

```sh
git clone https://github.com/norifm/nori
cd nori
cargo run --release -p nori-desktop     # the desktop client
cargo run --release -p nori-cli         # the terminal client
```

Both keep their data in `$XDG_DATA_HOME/nori` (or `~/.local/share/nori`) and share it, so a server added
in one is there in the other.

## iPod touch

The iPod touch 6th generation, on iOS 12.2 or later, jailbroken. The release carries
`nori-ipod-<version>.ipa`, fake-signed for AppSync Unified. See [iPod touch](../clients/ipod.md) for the
requirements and how to install it.
