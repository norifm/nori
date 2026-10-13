---
title: Performance
description: How nori keeps the CPU asleep while music plays, how it is measured, and the results so far.
sidebar:
  order: 1
---

Applies to nori 0.6. The model is the same on every client; the measurements are Android's.

## Android 0.6 emulator snapshot

On October 14, 2026, four Android apps were measured on the same ARM64 Android 16 emulator
with four guest cores and 2.5 GiB RAM, hosted on an Apple M4 Pro. Each used the same local
Navidrome server, account, library, and generated ten-minute 440 Hz stereo MP3 at 320 kbps.
Apps ran one at a time with media volume at zero and the screen off.

| Metric | nori 0.6.0-beta.5 | Symfonium 15.1.0 | Musly 2.0.2 | Navic v1.0.0-alpha59 |
| --- | ---: | ---: | ---: | ---: |
| CPU, share of one core | 0.1% | 8.02% | 2.81% | 3.35% |
| Voluntary context switches per second | 0.44 | 1023.81 | 391.94 | 307.1 |
| Low-wakeup intervals, out of 90 | 87 | 0 | 0 | 0 |
| Cold launch, median of five | 163 ms | 209 ms | 557 ms | 314 ms |
| PSS, MiB | 95.6 | 115.0 | 168.7 | 87.7 |

Playback values average two 90-second windows after at least fifteen seconds of playback.
CPU and context switches use app thread counters divided by actual elapsed time, excluding shared
Android audio services. A low-wakeup interval has fewer than twelve voluntary context switches
and an unchanged thread set; it does not measure physical CPU sleep residency. Cold launch uses
Android's `am start -W` first-frame `TotalTime`, with filesystem and library caches retained.

nori used the optimized, minified perf build with its recorder disabled. Symfonium and Navic used
official release APKs. Musly used a release build from v2.0.2 with its emulator startup guard
removed; playback code was unchanged. Individual readings, APK hashes, the build diff, collectors,
and compressed thread samples are in
[the raw measurement report](../performance/raw/android-emulator-2026-10-14/README.md).

These emulator results do not establish phone battery life or performance ordering on physical
hardware. The synthetic tone does not represent every real music file. Older comparisons below
used a different device, architecture, and nori version.

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
