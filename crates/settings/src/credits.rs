//! Third-party credits for the licences page.

/// One credit. `file` names the bundled licence text (`licences/<file>.txt`), None when there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Credit {
    pub name: String,
    pub what: String,
    pub copyright: String,
    pub licence: String,
    pub file: Option<String>,
}

/// A client whose licenses page lists credits: each lists only what it ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum CreditClient {
    Android,
    Web,
    Desktop,
    Terminal,
    Ipod,
}

use CreditClient::{Android as A, Desktop as D, Ipod as I, Terminal as T, Web as W};

const EVERY: &[CreditClient] = &[A, W, D, T, I];
/// The clients with a plugin host, backups and a camera's QR codes: all but the web and the iPod.
const PLUGIN_HOSTS: &[CreditClient] = &[A, D, T];
/// The clients with a window: all but the terminal, which draws in its own font.
const WINDOWED: &[CreditClient] = &[A, W, D, I];

/// One credit and where it ships. `crates` are the Cargo packages behind it, each in the dependency tree of
/// every client in `on` (`a|b`: either); empty for what is not a crate (an app library, a typeface, a
/// service). tests/credits_shipped.rs checks them. Add a line with each new dependency (and to NOTICE);
/// dual MIT/Apache crates show the MIT text.
struct Entry {
    on: &'static [CreditClient],
    crates: &'static [&'static str],
    name: &'static str,
    what: &'static str,
    copyright: &'static str,
    licence: &'static str,
    file: Option<&'static str>,
}

const fn entry(
    on: &'static [CreditClient],
    crates: &'static [&'static str],
    name: &'static str,
    what: &'static str,
    copyright: &'static str,
    licence: &'static str,
    file: Option<&'static str>,
) -> Entry {
    Entry { on, crates, name, what, copyright, licence, file }
}

const MIT: Option<&str> = Some("MIT");
const APACHE: Option<&str> = Some("Apache-2.0");

/// What the Rust core links.
const CORE: &[Entry] = &[
    entry(&[A], &["uniffi"], "uniffi", "Generates the Kotlin bindings to the core and the JNI calls under them", "Mozilla Foundation", "MPL-2.0", Some("MPL-2.0")),
    entry(EVERY, &["rusqlite"], "rusqlite", "The library index, full-text search and caches", "Copyright (c) 2014 The rusqlite developers", "MIT", MIT),
    entry(
        EVERY,
        &["libsqlite3-sys|sqlite-wasm-rs"],
        "SQLite",
        "The database itself, bundled into the core",
        "D. Richard Hipp and the SQLite developers, dedicated to the public domain",
        "Public domain",
        None,
    ),
    entry(EVERY, &["rustfft"], "RustFFT", "The spectrum analysis behind tempo, beats and key", "Copyright (c) 2015 The RustFFT Developers", "MIT or Apache-2.0", MIT),
    entry(
        EVERY,
        &["nori-stretch"],
        "Signalsmith Stretch",
        "Time-stretching for beat-matched mixes",
        "Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd.; Signalsmith Linear Copyright (c) 2025 Signalsmith Audio; ported to Rust in nori",
        "MIT",
        MIT,
    ),
    entry(EVERY, &["symphonia"], "Symphonia", "Reading and decoding the music", "Copyright (c) Philip Deljanov and the Symphonia contributors", "MPL-2.0", Some("MPL-2.0")),
    entry(
        EVERY,
        &["opus-rs"],
        "opus-rs",
        "Decoding Opus, a port of libopus",
        "Copyright 2026 restsend.com; libopus Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, CSIRO, Gregory Maxwell, Mark Borgerding, Erik de Castro Lopo, Mozilla, Amazon",
        "BSD-3-Clause",
        Some("BSD-3-Clause"),
    ),
    entry(
        &[A],
        &["tract-onnx"],
        "tract",
        "Running the beat model behind \"Better beat detection\"",
        "Copyright Sonos, Inc. and the tract contributors",
        "MIT or Apache-2.0",
        MIT,
    ),
    entry(EVERY, &["serde", "serde_json"], "serde and serde_json", "Reading the server's answers", "Copyright (c) David Tolnay and the Serde developers", "MIT or Apache-2.0", MIT),
    entry(&[A], &["jni"], "jni", "The core's direct calls from the audio path", "Copyright (c) 2016 Prevoty, Inc. and jni-rs contributors", "MIT or Apache-2.0", MIT),
    entry(&[A], &["simd_cesu8"], "simd_cesu8", "Java's strings turned into the core's and back", "Copyright (c) Sean C. Roach", "MIT or Apache-2.0", MIT),
    entry(EVERY, &["md-5"], "md-5", "Signing requests the way the Subsonic API asks", "Copyright (c) RustCrypto Developers", "MIT or Apache-2.0", MIT),
    entry(EVERY, &["parking_lot"], "parking_lot", "Locks inside the core", "Copyright (c) 2016 The Rust Project Developers (Amanieu d'Antras)", "MIT or Apache-2.0", MIT),
    entry(EVERY, &["thiserror"], "thiserror", "Errors inside the core", "Copyright (c) David Tolnay", "MIT or Apache-2.0", MIT),
    entry(
        EVERY,
        &["nori-player"],
        "Media3 Sonic and silence skipping, ported",
        "Speed, pitch and shortened silences, ported line for line into the core",
        "Copyright The Android Open Source Project",
        "Apache-2.0",
        APACHE,
    ),
    entry(
        EVERY,
        &["nori-look"],
        "AndroidX Palette, ported",
        "The colour quantiser behind a page's accent, ported line for line into the core",
        "Copyright The Android Open Source Project",
        "Apache-2.0",
        APACHE,
    ),
    entry(
        EVERY,
        &["zune-jpeg", "jpeg-decoder", "png", "image-webp", "gif"],
        "zune-jpeg, jpeg-decoder, png, image-webp and gif",
        "Decoding cover art: JPEG, PNG, WebP and GIF",
        "Copyright (c) the zune-image, image-rs and jpeg-decoder developers",
        "MIT or Apache-2.0",
        MIT,
    ),
    entry(EVERY, &["yaml-rust2"], "yaml-rust2", "Reading LRCLIB's word-timed lyrics", "Copyright (c) 2015 Chen Yuheng; Copyright (c) 2023 Ethiraric", "MIT or Apache-2.0", MIT),
    entry(EVERY, &["roxmltree"], "roxmltree", "Reading word-timed lyrics written as TTML", "Copyright (c) 2018 Yevhenii Reizner", "MIT or Apache-2.0", MIT),
    entry(
        EVERY,
        &["miniz_oxide"],
        "miniz_oxide",
        "Unpacking KuGou's word-timed lyrics",
        "Copyright 2013-2014 RAD Game Tools and Valve Software; Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC",
        "MIT, Zlib or Apache-2.0",
        MIT,
    ),
    entry(EVERY, &["futures-util"], "futures-util", "Asking the lyrics services together", "Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors", "MIT or Apache-2.0", MIT),
    entry(EVERY, &["qrcode"], "qrcode", "The QR code of a jam's invite", "Copyright (c) 2016 kennytm", "MIT or Apache-2.0", MIT),
    entry(
        &[A, D, T, I],
        &["rqrr"],
        "rqrr",
        "Reading a pairing QR code from the camera",
        "Copyright (c) WanzenBug and the rqrr contributors; quirc Copyright (C) 2010-2012 Daniel Beer",
        "MIT or Apache-2.0, and ISC",
        MIT,
    ),
    entry(
        &[A, D, T, I],
        &["aes", "cbc", "des", "rc2"],
        "aes, cbc, des and rc2",
        "Opening a profile's client certificate",
        "Copyright (c) RustCrypto Developers",
        "MIT or Apache-2.0",
        MIT,
    ),
    entry(&[A, D, T, I], &["rustls"], "rustls", "The encryption of network connections", "Copyright (c) 2016 Joseph Birr-Pixton", "MIT or Apache-2.0", MIT),
    entry(&[D, T, I], &["ureq"], "ureq", "Every network request", "Copyright (c) 2019 Martin Algesten", "MIT or Apache-2.0", MIT),
    entry(
        PLUGIN_HOSTS,
        &["wasmtime", "cranelift-codegen"],
        "Wasmtime and Cranelift",
        "Plugins compiled and run",
        "Copyright the Bytecode Alliance and the Wasmtime Project Developers",
        "Apache-2.0 with LLVM exception",
        APACHE,
    ),
    entry(&[A, W, D, T], &["ed25519-dalek"], "ed25519-dalek", "Checking a plugin package's signature", "Copyright (c) 2017-2019 isis agora lovecruft", "BSD-3-Clause", Some("BSD-3-Clause")),
    entry(&[A, W, D, T], &["pulldown-cmark"], "pulldown-cmark", "A plugin's readme", "Copyright 2015 Google Inc.", "MIT", MIT),
    entry(PLUGIN_HOSTS, &["zip"], "zip", "Plugin packages and backups", "Copyright (c) 2014 Mathijs van de Nes", "MIT", MIT),
    entry(PLUGIN_HOSTS, &["argon2", "chacha20poly1305"], "argon2 and chacha20poly1305", "Sealing a backup with its password", "Copyright (c) RustCrypto Developers", "MIT or Apache-2.0", MIT),
];

/// The clients' own libraries.
const CLIENT: &[Entry] = &[
    // The Android app's.
    entry(&[A], &[], "AndroidX Media3", "Playback, the media session and the notification", "Copyright The Android Open Source Project", "Apache-2.0", APACHE),
    entry(&[A], &[], "Jetpack Compose and Material 3", "The user interface toolkit", "Copyright The Android Open Source Project", "Apache-2.0", APACHE),
    entry(&[A], &[], "AndroidX Navigation, Lifecycle, Activity, Core, WebKit", "The app's plumbing", "Copyright The Android Open Source Project", "Apache-2.0", APACHE),
    entry(&[A], &[], "OkHttp", "Every network request", "Copyright Square, Inc.", "Apache-2.0", APACHE),
    entry(
        &[A],
        &[],
        "kotlinx.coroutines",
        "The concurrency the app is written in",
        "Copyright JetBrains s.r.o. and Kotlin Programming Language contributors",
        "Apache-2.0",
        APACHE,
    ),
    entry(&[A], &[], "Guava", "Futures between the media session and the app", "Copyright The Guava Authors", "Apache-2.0", APACHE),
    entry(&[A], &[], "Stripe Android", "The Support page's payment sheet", "Copyright (c) 2011- Stripe, Inc. (https://stripe.com)", "MIT", MIT),
    // What the desktop and the terminal play and find devices through.
    entry(&[D, T], &["cpal"], "cpal", "Playing through the sound card", "Copyright the CPAL contributors", "Apache-2.0", APACHE),
    entry(&[D, T], &["zbus"], "zbus", "The media controls on Linux", "Copyright (c) 2024 Zeeshan Ali Khan & zbus contributors", "MIT", MIT),
    entry(&[D, T], &["mdns-sd"], "mdns-sd", "Finding nori on the local network", "Copyright (c) 2021-2022 Han Xu", "MIT or Apache-2.0", MIT),
    entry(
        &[D, T],
        &["wry", "tao"],
        "wry and tao",
        "A provider's sign-in page, in the system's web view",
        "Copyright (c) 2020-2023 Ngo Iok Ui & Tauri Programme within The Commons Conservancy",
        "Apache-2.0",
        APACHE,
    ),
    // The desktop client's own libraries.
    entry(&[D], &["slint"], "Slint", "The user interface toolkit", "Copyright © SixtyFPS GmbH", "GPL-3.0", Some("GPL-3.0")),
    entry(
        &[D],
        &["skia-safe"],
        "Skia and rust-skia",
        "Drawing the window: its text, shapes and covers",
        "Copyright (c) 2011 Google Inc.; rust-skia by Armin Sander and its contributors",
        "BSD-3-Clause, rust-skia MIT",
        Some("BSD-3-Clause"),
    ),
    entry(&[D], &["wgpu"], "wgpu", "The graphics card the window is drawn on", "Copyright (c) 2025 The gfx-rs developers", "MIT or Apache-2.0", MIT),
    entry(&[D], &["winit", "glutin"], "winit and glutin", "The window, its keys and its pointer", "Copyright the winit contributors and Kirill Chibisov", "Apache-2.0", APACHE),
    entry(&[D], &["arboard"], "arboard", "Copying to the clipboard", "Copyright (c) 2022 The Arboard contributors", "MIT or Apache-2.0", MIT),
    entry(
        &[D],
        &["openh264"],
        "OpenH264",
        "A plugin's moving cover, decoded in software",
        "Copyright (c) 2013, Cisco Systems; the Rust bindings by Ralf Biedert",
        "BSD-2-Clause",
        Some("BSD-2-Clause"),
    ),
    // The terminal client's own libraries.
    entry(
        &[T],
        &["ratatui", "crossterm"],
        "ratatui and crossterm",
        "The screen and the keys",
        "Copyright (c) 2016-2022 Florian Dehau; Copyright (c) 2023-2025 The Ratatui Developers; Copyright (c) 2019 Timon",
        "MIT",
        MIT,
    ),
    entry(
        &[T],
        &["ratatui-image"],
        "ratatui-image",
        "Covers in the terminal: kitty, sixel, iTerm2 or half blocks",
        "Copyright (c) 2022 Atanas Yankov; Copyright (c) 2022 wooster0; Copyright (c) 2023 Benjamin Große",
        "MIT",
        MIT,
    ),
    entry(&[T], &["image", "icy_sixel"], "image and icy_sixel", "Pictures handed to the terminal", "Copyright the image-rs developers and Mike Krüger", "MIT or Apache-2.0", MIT),
    // The web client's libraries, Rust's and the page's.
    entry(&[W], &[], "Svelte", "The user interface", "Copyright (c) 2016-2025 Svelte Contributors", "MIT", MIT),
    entry(
        &[W],
        &["wasm-bindgen", "js-sys", "web-sys"],
        "wasm-bindgen, js-sys and web-sys",
        "The core's calls into the browser and back",
        "Copyright (c) 2014 Alex Crichton and the wasm-bindgen developers",
        "MIT or Apache-2.0",
        MIT,
    ),
    entry(
        &[W],
        &["sqlite-wasm-rs", "sqlite-wasm-vfs"],
        "sqlite-wasm-rs and sqlite-wasm-vfs",
        "SQLite built for the browser, its files kept in the browser's own storage",
        "Copyright (c) 2024 Spxg",
        "MIT",
        MIT,
    ),
    entry(
        &[W],
        &[],
        "jco",
        "Plugins made into JavaScript, the way a browser runs them",
        "Copyright the Bytecode Alliance and the jco contributors",
        "Apache-2.0 with LLVM exception",
        APACHE,
    ),
    entry(&[W], &[], "hls.js", "A plugin's moving cover where the browser can't play its stream itself", "Copyright (c) 2017 Dailymotion", "Apache-2.0", APACHE),
];

/// The typefaces and icons a client with a window draws with, then the third-party data and services.
const DATA: &[Entry] = &[
    entry(WINDOWED, &[], "Inter", "The typeface", "Copyright (c) 2016 The Inter Project Authors (Rasmus Andersson)", "OFL-1.1", Some("OFL-1.1")),
    entry(
        WINDOWED,
        &[],
        "Literata",
        "The typeface of page titles",
        "Copyright 2017 The Literata Project Authors (https://github.com/googlefonts/literata)",
        "OFL-1.1",
        Some("OFL-1.1"),
    ),
    entry(
        WINDOWED,
        &[],
        "Shippori Mincho",
        "The seal and the seasons' kanji, drawn from its outlines",
        "Copyright 2020 The Shippori Mincho Project Authors (https://github.com/fontdasu/ShipporiMincho)",
        "OFL-1.1",
        Some("OFL-1.1"),
    ),
    entry(WINDOWED, &[], "Iconoir", "The icons", "Copyright (c) 2021 Luca Burgio", "MIT", MIT),
    // The third-party data and services.
    entry(
        EVERY,
        &[],
        "AutoEQ",
        "Headphone correction curves: the list kept on Wi-Fi, a curve fetched when it is chosen",
        "Copyright (c) 2018 Jaakko Pasanen",
        "MIT",
        MIT,
    ),
    entry(EVERY, &[], "LRCLIB", "Timed lyrics for songs your server has none for, asked only when switched on", "lrclib.net; lyrics belong to their authors and contributors", "Service", None),
    entry(
        EVERY,
        &[],
        "Unison",
        "Lyrics written and timed by listeners, asked only when switched on",
        "Lyrics from Unison (https://unison.boidu.dev), under the Open Database License (ODbL-1.0); lyrics belong to their authors",
        "ODbL-1.0",
        None,
    ),
    entry(
        EVERY,
        &[],
        "BiniLyrics",
        "Lyrics timed syllable by syllable, asked only when switched on",
        "binimum.org, a volunteer's copy of Apple Music's lyrics; lyrics belong to their authors",
        "Service",
        None,
    ),
    entry(EVERY, &[], "BetterLyrics", "Lyrics timed syllable by syllable, asked only when switched on", "betterlyrics.org; lyrics belong to their authors", "Service", None),
    entry(
        EVERY,
        &[],
        "PaxSenix",
        "Apple Music's, Spotify's and Musixmatch's lyrics, asked only when switched on (the last two with your own key)",
        "paxsenix.org, with songs found through Apple's iTunes Search API; lyrics belong to their authors",
        "Service",
        None,
    ),
    entry(
        EVERY,
        &[],
        "LyricsPlus",
        "Lyrics timed syllable by syllable, asked only when switched on",
        "The YouLy+ project's volunteer servers; lyrics belong to their authors",
        "Service",
        None,
    ),
    entry(EVERY, &[], "NetEase Cloud Music", "Lyrics, often timed word by word, asked only when switched on", "music.163.com; lyrics belong to their authors", "Service", None),
    entry(EVERY, &[], "KuGou", "Lyrics, often timed word by word, asked only when switched on", "kugou.com; lyrics belong to their authors", "Service", None),
    entry(EVERY, &[], "YouTube Music", "The lyrics YouTube Music shows for a song, asked only when switched on", "Google LLC; lyrics belong to their authors", "Service", None),
    entry(EVERY, &[], "iTunes Search API", "Finding a song's Apple Music id for PaxSenix, asked only when switched on", "Apple Inc.", "Service", None),
    entry(
        &[A],
        &["tract-onnx"],
        "Beat This!",
        "The neural beat tracker behind \"Better beat detection\": its network comes with the app, its weights from the authors' server when switched on",
        "Copyright (c) 2024 Institute of Computational Perception, JKU Linz, Austria (Foscarin, Schlüter and Widmer)",
        "MIT",
        MIT,
    ),
];

fn credits(client: CreditClient, group: &[Entry]) -> Vec<Credit> {
    group
        .iter()
        .filter(|e| e.on.contains(&client))
        .map(|e| Credit { name: e.name.to_string(), what: e.what.to_string(), copyright: e.copyright.to_string(), licence: e.licence.to_string(), file: e.file.map(str::to_string) })
        .collect()
}

/// What `client`'s build of the Rust core links.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn core_credits(client: CreditClient) -> Vec<Credit> {
    credits(client, CORE)
}

/// `client`'s own libraries.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn client_credits(client: CreditClient) -> Vec<Credit> {
    credits(client, CLIENT)
}

/// The typefaces and icons `client` draws with, and the data and services it reaches.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn data_credits(client: CreditClient) -> Vec<Credit> {
    credits(client, DATA)
}

/// Each of `client`'s credits by name, with the Cargo packages behind it (tests/credits_shipped.rs).
pub fn credited_crates(client: CreditClient) -> Vec<(&'static str, &'static [&'static str])> {
    [CORE, CLIENT, DATA].into_iter().flatten().filter(|e| e.on.contains(&client)).map(|e| (e.name, e.crates)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_complete() {
        // Every licence named is bundled for the licences page, no client's page lists one twice, and every
        // licence text bundled is named by a credit.
        let texts = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../android/app/src/main/assets/licences"));
        for client in [A, W, D, T, I] {
            let all = [core_credits(client), client_credits(client), data_credits(client)].concat();
            let missing: Vec<&String> = all.iter().filter_map(|c| c.file.as_ref()).filter(|f| !texts.join(format!("{f}.txt")).is_file()).collect();
            assert!(missing.is_empty(), "{client:?}: licence texts named but not in android/app/src/main/assets/licences: {missing:?}");
            let mut names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
            names.sort_unstable();
            let twice: Vec<&&str> = names.windows(2).filter(|w| w[0] == w[1]).map(|w| &w[0]).collect();
            assert!(twice.is_empty(), "{client:?}: credited twice: {twice:?}");
            assert!(all.iter().all(|c| !c.what.is_empty() && !c.copyright.is_empty() && !c.licence.is_empty()), "{client:?}: every credit says what, whose and under what terms");
        }
        let named: std::collections::BTreeSet<&str> = [CORE, CLIENT, DATA].into_iter().flatten().filter_map(|e| e.file).collect();
        let bundled: std::collections::BTreeSet<String> =
            std::fs::read_dir(texts).expect("licences").map(|f| f.expect("entry").file_name().to_string_lossy().trim_end_matches(".txt").to_string()).collect();
        assert_eq!(bundled.iter().map(String::as_str).collect::<std::collections::BTreeSet<_>>(), named, "bundled licence texts no credit names, or named ones missing");
    }
}
