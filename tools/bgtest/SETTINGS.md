# What each player offers, and where

Applies to the nori Android perf build at 0.7. The other players were mapped by hand on a Galaxy S21 FE
(SM-G990B, Android 16) in English (`cmd locale set-app-locales <pkg> --locales en-US`): Musly and
Navic as installed then, Symfonium 15.1.0 from repo.symfonium.app. "Default" is a fresh install
(`pm clear`). Paths start from the app's home screen.

## Settings

| Feature | nori | Musly | Navic | Symfonium |
|---|---|---|---|---|
| **Crossfade** | Settings > Playback > Crossfade: Off or 1 to 12 s (default Off; grayed out while AutoMix is on) | Settings > Playback > Smart Crossfade slider, Off to 12 s in 1 s steps (default Off) | none | Settings > Playback > Transitions > Crossfade (default off; "disabled when playing albums in order") |
| **Beat-aware mixing** | Playback > AutoMix, one switch (default off) | none ("Auto DJ" only picks what plays next) | none | Transitions > Smart fades (default off): "best crossfade from the waveform, may increase battery usage" |
| **Neural beat detection** | Playback > Better beat detection, under AutoMix (default off, 8 MB model download) | Storage > BPM analysis: caches BPMs, "Cache All BPMs" (on demand) | none | none |
| **Queue autofill** | Playback > Keep playing when the queue ends (default on: songs, similar music) | Playback > Auto DJ Mode: Off / Shuffle Library / Similar Songs / Same Genre / Same Artist / Smart Mix (default Off) | Playback > Auto-fill queue | Smart flow (up to 12 tracks) |
| **Gapless** | always on; Playback > Keep albums gapless keeps crossfades and AutoMix out of albums (default on) | Playback > Gapless Playback (default on) | Playback > Audio effects > Gapless playback (default on, experimental, needs restart) | built in |
| **Fade on play/pause** | always 150 ms, no setting | Playback > Enable Fade In/Out | none | Transitions > Fade in / Fade out (default disabled) |
| **Equalizer** | Settings > Sound > Equalizer and crossfeed: 10-band graphic or parametric, presets (Bass boost among them), pre-amp, balance, mono, limiter, AutoEQ (default off); crossfeed is Sound > Headphone sound: Off / Natural / Wide | **none** (the player menu has only sleep timer, speed, pitch) | Playback > Audio effects > Equaliser > source: Disabled / Built-in / External (default Disabled) | Settings > Playback > Output settings > Telefon > Equalizer: parametric and graphic EQ, bass boost, volume boost, compressor, limiter, virtualizer, crossfeed, skip silence (default off) |
| **ReplayGain** | Sound > Volume leveling: Off / Per song / Per album / Automatic (default Automatic), at a Quiet / Normal / Loud level | Playback > Volume Normalization (ReplayGain) > Mode (default Off) | Audio effects > ReplayGain mode: Off / Track / Album / Dynamic (default Off) | Output settings > Telefon > Equalizer > Replay gain (default Off) |
| **Audio offload** (the audio chip decodes, the CPU sleeps) | Sound > Save battery while playing (default on; paused while anything touches the samples: equalizer, effects, crossfade, AutoMix; never with USB audio) | none | Audio effects > Audio offload (default **off**, experimental, needs restart) | none found |
| **Decoder** | nori's Rust engine (Symphonia); HE-AAC through Android's MediaCodec | Android MediaCodec via ExoPlayer | Android MediaCodec via ExoPlayer | its own; Output > "Prefer hardware-accelerated codecs" (default off) |
| **Streaming quality** | Downloads and storage > Quality on Wi-Fi / on mobile data (default Original for both) | Playback > Enable Transcoding (default off = original) | Playback > Streaming quality: Wi-Fi and cellular Low / Medium / High / Lossless (default Lossless) | Playback > Decoding and transcoding: Wi-Fi / mobile maximum bitrate (default Original) |
| **Load ahead / cache** | Downloads and storage > Prepare songs ahead on Wi-Fi (default 3 songs) / on mobile data (default next song); Space for streamed music (default 1 GB) | Storage > Songs & Streaming Cache | ExoPlayer default buffer | Offline, cache, and download > Playback > Playback cache size (default **Disabled**) |
| **Download for offline** | a playlist's More > Download; Downloads and storage > Download library | Storage > Offline downloads; playlist page "Download playlist" | Data & Storage > Download entire library | Manage offline files |
| **Lyrics from the internet** | Settings > Lyrics > Find missing lyrics online (default **on**; 11 services, 8 on by default) | Playback > Fetch lyrics from LRCLIB (default **on**) | Now Playing > Configure lyric providers: Subsonic on, LRCLIB off, LyricsPlus off | not found in settings |
| **Keep screen on with lyrics** | Lyrics > Keep the screen on (default on) | Storage > "Keep Screen On" is for downloads only | Now Playing > Keep screen on in lyrics view | — |
| **Scrobbling** | Library > Tell the server what you play (default on) | — | Playback > Enable scrobbling | Playback > play percentages |

## Variants

What each app module in `apps/` sets (`bgtest.py --list`). `--plan matrix` runs every variant marked
in the matrix column, plus each app's first variant (`plain` or `default`) with the playlist downloaded first.

| app | variant | sets | matrix |
|---|---|---|---|
| nori | `plain` | nothing: offloaded when the song's format allows | yes |
| nori | `eq` | the equalizer on, Bass boost preset: the CPU path, no offload | yes |
| nori | `automix` | AutoMix on | yes |
| nori | `automix-ml` | AutoMix and Better beat detection on (waits for the model download) | yes |
| nori | `eq-automix`, `eq-automix-ml` | the equalizer with AutoMix, and with the beat model | no |
| Musly | `default`, `crossfade` | Smart Crossfade at 6 s | yes |
| Navic | `default` | nothing | yes |
| Navic | `offload` | Audio offload on, gapless off, no skips (see below), app restarted | yes |
| Navic | `eq` | Built-in equalizer, the first two of five bands raised | yes |
| Navic | `static-ui` | Now Playing: Background style Static, Slider style Flat; for the player scenario | no |
| Symfonium | `default` | nothing | yes |
| Symfonium | `crossfade` | Transitions: Smooth fade-out and fade-in curves (6.0 s) | yes |
| Symfonium | `smartfades` | the crossfade with Smart fades on | yes |
| Symfonium | `eq` | the graphic equalizer, 31.5 and 63 Hz raised | yes |
| Symfonium | `smartfades-eq` | Smart fades and the equalizer | no |

## Found while testing

- **Navic's equalizer and offload exclude each other.** With Audio offload on, its Equaliser page cannot
  be opened, so there is no "EQ + offload" variant.
- **Navic offload needs gapless off.** The S21 FE offloads without gapless support. Then it plays
  offloaded, but after "next" it stays in BUFFERING for good, so its runs start at the first song with
  no skips.
- **Musly and Navic decode in Android's mediacodec service** (UID 1046), which is charged to that
  service, not to the app. The harness adds it ("app + decoder + audioserver").

## Driving notes

- Navic's switches don't report their state to uiautomator: the harness reads the switch color from a
  screenshot.
- Musly's settings are one Flutter page with tabs ("Playback\nTab 1 of 6"...). Its Auto DJ and
  ReplayGain choices are drop-downs opened by tapping the arrow at the row's right; crossfade is a
  slider.
- Symfonium's pages scroll far: go back to the top before looking for an entry.
