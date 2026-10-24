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
playback code unchanged. Versions, APK hashes and each app's results are in the
[summary](https://github.com/norifm/nori/blob/master/docs/performance/raw/android-emulator-music-2026-10-14/summary.json); the scripts that
measure it are in [tools/bgtest/compare](../../tools/bgtest/compare/README.md).

These are emulator measurements, not phone battery-life figures.

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

The perf build measures a real phone in real use, without a computer attached: see
[Building](developers/building.md#the-perf-build).
