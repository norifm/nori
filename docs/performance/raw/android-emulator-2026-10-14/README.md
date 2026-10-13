# Android 0.6 emulator comparison

Measured October 14, 2026 (Europe/Warsaw), on the same `a16b` ARM64 Android 16 emulator:
four guest CPU cores, 2.5 GiB RAM, host graphics, Apple M4 Pro, macOS 26.6.2. Every app used the
same local Navidrome 0.64.2 server and account, the same library, and the same generated
10-minute 440 Hz stereo MP3 at 44.1 kHz and 320 kbps, "Bench Pulse MP3".

Apps ran one at a time, media volume zero, screen off, without touches during measurement.
Host build jobs were excluded through `tools/with-resource.py build`. Playback was established
for at least 15 seconds before sampling. nori was its optimized, minified perf build at `be9f2dc6`,
with the recorder disabled. The other apps used release builds. No effects were explicitly enabled.

| Metric | nori 0.6.0-beta.5 | Symfonium 15.1.0 | Musly 2.0.2 | Navic v1.0.0-alpha59 |
| --- | ---: | ---: | ---: | ---: |
| CPU, share of one core | 0.1% | 8.02% | 2.81% | 3.35% |
| Voluntary context switches per second | 0.44 | 1023.81 | 391.94 | 307.1 |
| Low-wakeup intervals, out of 90 | 87 | 0 | 0 | 0 |
| Cold launch, median of five | 163 ms | 209 ms | 557 ms | 314 ms |
| PSS, MiB | 95.6 | 115.0 | 168.7 | 87.7 |

Playback values average two 90-second windows; startup values are medians of five launches.
Individual runs, versions, APK SHA-256 hashes, and track SHA-256 are in `summary.json`.
Each app's directory contains both full compressed thread sample streams. Session dumps are also
retained for the nori and Symfonium runs.

## Counters

CPU uses `/proc/<pid>/task/<tid>/stat` user and system ticks, with the device's 100 Hz clock.
Context switches use each thread's `voluntary_ctxt_switches` counter. Adjacent sample deltas include
observed work from threads which retire during a run. Values divide by actual monotonic elapsed
time, rather than nominal sleep duration. Threads which start and exit between reads can be missed.
The scope is the app's processes, excluding shared Android audio services.

There are 91 samples per run, producing 90 intervals. A low-wakeup interval has fewer than 12
voluntary switches and an unchanged thread set. This is a wakeup proxy, not CPU sleep residency
or device-wide wakeups. MediaSession state and track metadata are checked before and after each run.
The warm-up uses the playback position projected from its Android elapsed-time reference; a player
can leave that reference unchanged while playing. AudioFlinger also showed an active Symfonium track.

Cold launch force-stops the app, then calls `am start -W`, five times. Every launch reports
`LaunchState: COLD`. Filesystem and library caches are retained. `TotalTime` measures the first
activity frame, not completed library synchronization or readiness for every interaction.

The generated tone keeps the input identical; it does not represent every real music file.
Emulators cannot establish phone battery life or performance ordering on physical devices.
The earlier Android 14 x86-64 results are a separate experiment.

## App sources

Symfonium came from its official phone F-Droid feed; Navic came from its official GitHub release.
Musly was compiled from v2.0.2 (`f69a03010ab6ede8d8032a6c3bcd4c47b5f0f5bb`) with Flutter 3.41.7
(the release workflow's version) and Python 3.11.15. Its official APK refuses emulators. At the
owner's request, the five-line startup guard was removed; playback code is unchanged. Flutter
also resolved its SDK-pinned `meta` and `test_api` versions. The complete diff is in
`musly-emulator.patch`, and download URLs are in `sources.json`. All four installed builds were
verified as not debuggable.

## Reproduce

Generate the track with:

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=600' -ac 2 \
  -c:a libmp3lame -b:a 320k -metadata title='Bench Pulse MP3' \
  -metadata artist='Nori Bench' -metadata album=Playback 'Bench Pulse.mp3'
```

Use a rooted test emulator so thread counters are readable. Install the recorded app versions,
sign in to the same server, start the track, stop the other music apps, and run from the repo root
(replace the package and component for each app):

```sh
python3 tools/with-resource.py build,device:emulator-5556 python3 \
  docs/performance/raw/android-emulator-2026-10-14/collector.py \
  --package dev.nori.music.perf --output build/website/bench/nori
python3 tools/with-resource.py build,device:emulator-5556 python3 \
  docs/performance/raw/android-emulator-2026-10-14/cold-start.py \
  --package dev.nori.music.perf --component dev.nori.music.perf/dev.nori.music.app.MainActivity \
  --output build/website/bench/nori/cold-start.json
```
