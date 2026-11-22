# nori's words

Use one name for each thing, the one on the left. The right column lists names to avoid for it. App labels
always win: if the app's words (Android's `strings.xml`, the desktop's `words.rs`, the terminal's `text.rs`)
say otherwise, follow the app and fix this list.

## For listeners

| Term | Means | Do not write |
|---|---|---|
| nori | the app, always lowercase, even at the start of a sentence | Nori, NORI, the nori app |
| client | one of nori's apps, in developer docs | front end, frontend |
| Android | the Android app, phones and TVs | the mobile app |
| the desktop app | nori for Windows, macOS and Linux, with windows | the desktop, desktop client (in user docs) |
| the terminal app | nori in a terminal | the terminal, the CLI, the TUI (in user docs) |
| the iPod app | nori for the iPod touch | iOS app |
| server | the Navidrome, Subsonic, OpenSubsonic or octo-fiesta server you sign in to | backend, instance |
| profile | one account nori plays from: a server account, or a YouTube Music or SoundCloud account | connection, source, account (for the whole profile) |
| account | your sign-in at a service or server | login |
| plugin | an add-on you install in nori | extension, addon, mod |
| plugin repo | a list of plugins nori installs from | store, marketplace |
| favorite | a song, album or artist you marked | like, heart, star (except for YouTube Music's likes) |
| the seal 好 | the mark nori shows on a favorite | heart, star |
| queue | the songs that play next | playlist (for the queue), up next |
| **Play next**, **Add to queue** | the app's two queue actions, by their labels | play after, enqueue |
| autofill | songs nori adds when the queue ends | autoplay, infinite queue |
| download | a song kept on the device to play offline | offline copy, save |
| stream cache | songs nori kept while streaming, cleared as needed | cache (alone), buffer |
| AutoMix | nori's beat-matched transition between songs | auto-mix, Automix, DJ mode |
| crossfade | a plain overlap of two songs | blend (for crossfade) |
| gapless | songs that run into each other without a gap | seamless |
| volume leveling | playing songs at an even loudness | normalization, ReplayGain (for the feature) |
| ReplayGain | the tags in a file that say how loud it is | |
| output | where sound goes: speaker, headphones, Bluetooth, USB DAC, a car | device (for an output), sink (in user docs) |
| sound profile | the equalizer and effects nori keeps for one output | preset (that is an equalizer preset) |
| offload | the phone's audio chip decodes the music instead of the CPU | hardware decoding, passthrough |
| remote control | playing on one of your devices from another | casting, cast |
| jam | a shared listening session that others join with a link or QR code | party, session, room |
| host, guest | the device that runs a jam, and someone who joined it | owner, member (for a guest) |
| provider song | an octo-fiesta song from a streaming provider, not yet in your library | external song, `ext-` song (in user docs) |
| Day, Night, Midnight | nori's appearances. Midnight is true black | dark mode, AMOLED |
| season | Spring 春, Summer 夏, Autumn 秋, Winter 冬: the page color and accent | theme (for a season) |

## For developers

| Term | Means | Do not write |
|---|---|---|
| the core | `nori-core` and the domain crates it re-exports | backend, library (ambiguous with the music library) |
| `Core` | one server profile's state and calls | |
| the engine | `nori-engine`: the player on its own thread | audio thread (it is more than that) |
| `nori-host` | the session the desktop, terminal, iPod and web clients share | the host (ambiguous: say jam host or plugin host otherwise) |
| plugin host | `crates/plugins`, which installs and runs plugins | runtime |
| provider | where a profile's music comes from: a Subsonic server or a plugin's provider export | backend, source |
| sound chain | the equalizer, effects and stages a song's samples pass through | DSP pipeline |
| stage | one step of the sound chain, including a plugin's | effect (for a stage) |
| setting | one `StoredPrefs` field with its `#[setting(...)]` line | pref, option, config key |
| door | a JNI function registered in `crates/android/src/lib.rs` | binding (say "JNI door" on first use) |
| the virtual clock | the test clock in `nori-time` that a test moves | fake time, mock clock |
