---
title: Performance
description: How nori keeps the CPU asleep while music plays, how it is measured, and the results so far.
sidebar:
  order: 1
---

Applies to nori 0.6. The model is the same on every client; the measurements are Android's.

## Android 0.6: real music

Measured October 14, 2026, on one Android 16 ARM64 emulator with four virtual cores and 2.5 GiB RAM,
hosted on an Apple M4 Pro running macOS 26.6.2. All four apps use the same isolated Navidrome 0.64.0
server, 97-song library, and playlist. Builds: nori 0.6.0-beta.5, Symfonium 15.1.0, Musly 2.0.2,
and Navic v1.0.0-alpha59.

The input contains real-music excerpts from the existing background-test corpus. Two five-minute
passes cover MP3 at 44.1 and 48 kHz, plus FLAC at 44.1, 48, and 96 kHz. The FLAC variants are derived
from MP3 sources to exercise decoding and resampling; they are not original high-resolution masters.
Every format was observed playing in both passes for every app. Short clips change tracks every
minute; uninterrupted album playback can have different costs.

| Metric | nori | Symfonium | Musly | Navic |
|---|---:|---:|---:|---:|
| App CPU | 0.81% | 7.89% | 3.91% | 4.28% |
| App + Android audio/codec CPU | 1.55% | 8.64% | 5.93% | 6.08% |
| Observed quiet app intervals | 268 / 300 | 0 / 300 | 0 / 300 | 0 / 300 |
| Observed voluntary app thread switches / second | 32.68 | 1037.29 | 326.46 | 254.5 |
| Cold activity launch, median of five | 182 ms | 219 ms | 561 ms | 314 ms |

CPU is a percentage of one virtual core. Process counters include work by threads that exit between
samples. The combined row adds `audioserver`, `media.swcodec`, and `media.extractor`; it is not
whole-device CPU. Quiet intervals and observed thread switches concern app threads only and do not
measure CPU sleep residency or complete device wakeups. The screen is off and music volume is zero.
Cold launches measure the first activity frame, with library and filesystem caches warm.

nori uses its optimized, minified perf APK with recording disabled. Symfonium and Navic use official
release APKs. Musly is the documented release build with its emulator startup guard removed and
playback code unchanged. APK hashes, source-file hashes, every playback window, all five launches,
MediaSession checks, and compressed counters are in the
[raw music report](../performance/raw/android-emulator-music-2026-10-14/README.md).

These are emulator measurements, not phone battery-life figures. The earlier
[generated-tone report](../performance/raw/android-emulator-2026-10-14/README.md) remains archived;
it uses a different input and CPU accounting method and is not combined with these results.

## Burst playback

Most players decode a little audio, hand it to the speaker, and wake again a few milliseconds later.
nori decodes a stretch of music ahead instead, then lets the CPU sleep until that stretch is nearly
played.

- On Android the player writes into a deep AudioTrack buffer, about ten seconds of music. The player's
  thread and the track's writer both wake about once every ten seconds while music plays, and nothing
  on the audio path wakes in between.
- With nothing that needs the samples (no equalizer, no speed change, no AutoMix), songs can go to the
  phone's audio chip as they are: **offload**. The CPU then sleeps for minutes between top-ups. See
  [Offload and processing](sound/offload.md).
- Nothing polls or ticks while music plays with the screen off. The seek bar's timer runs only while
  the player is on screen.
- An optional feature that is switched off costs nothing: it is not started, holds no listener and
  opens no connection.

## How it is measured

The comparisons below were made on one Android emulator (`sdk_gphone64_x86_64`, Android 14) against one
local Navidrome server, with the same tracks for every app, media volume at 0, the screen off and no
touches. nori was the release build; the other apps were their store releases.

- **CPU** is the share of one core the app used.
- **Quiet seconds** approximate background inactivity using thread context-switch counters; they
  do not measure physical CPU sleep residency.
- Playback rows are 90-second windows over a 10-minute MP3 at 320 kbps and a FLAC; the equalizer row
  adds a treble boost in each app that has one.
- An emulator has no real battery and decodes in software, so the absolute numbers do not carry over to
  a phone. The order between apps on physical hardware can also differ.

The perf build measures a real phone in real use, without a computer attached: see
[Building](developers/building.md#the-perf-build).

## Results

:::caution[Historical]
These numbers were measured on nori 0.3.x in September 2026 and are separate from the 0.6 comparison above. The raw
runs, with every thread and profile, are in the repository under
[`docs/performance/raw`](../performance/raw/benchmarks.md).
:::

| Measured on nori 0.3.1 | nori | Symfonium 15.0.1 | Musly 2.0.2 | Navic alpha55 |
|---|---|---|---|---|
| CPU playing MP3 | 1.26 % | 10.7 % | 4.01 % | 3.10 % |
| CPU playing FLAC | 1.30 % | 8.74 % | 4.70 % | 3.72 % |
| CPU playing with the equalizer | 1.35 % | 13.4 % | no equalizer | 2.92 % |
| Quiet seconds, MP3 | 75 of 90 | 1 of 90 | 0 of 90 | 1 of 90 |
| Cold start | about 590 ms | about 900 ms | about 1050 ms | about 700 ms |
| Wake lock while paused | none | none | held | none |

Musly has no equalizer, and Navic has no transitions, so those rows compare features as much as cost.
