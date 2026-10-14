# Android emulator comparison with real music — October 14, 2026

All four apps use the same Android 16 ARM64 emulator, `a16b`, with four virtual CPU cores, host
rendering, and 2.5 GiB of guest RAM. The host is an Apple M4 Pro running macOS 26.6.2. Only one
emulator runs during measurement. These results describe this emulator; they do not establish
phone battery life or ordering on physical hardware.

## Music and library

The library is the owner's existing `tools/bgtest/server.py` corpus: real songs copied from a music
library, plus derived format variants and one-minute excerpts. The isolated Navidrome 0.64.0
server mounts `build/bgtest/server/music` read-only and has its own database. All clients index the
same 97-song library and play the same `website-bench-mixed` playlist in order. The stream endpoint
was checked for successful range responses containing MP3 or FLAC headers before measurement.

The playlist contains two rounds of five one-minute music excerpts: MP3 at 44.1 kHz, MP3 at 48 kHz,
FLAC 16-bit/44.1 kHz, FLAC 24-bit/48 kHz, and FLAC 24-bit/96 kHz. The FLAC variants are derived from
MP3 sources to exercise format handling and resampling; they are not original high-resolution
masters. An extra copy of the first excerpt keeps playback running past the measurement's end.
The manifest identifies the source files and their hashes. Audio files are not distributed here.

## Playback

Each app gets a clean local profile, the same server account, and its default playback settings.
The library is available before playback starts. Every competing test app is force-stopped.
The music volume is zero and the screen is off. Recording and build work stop during measurement.

After at least 15 seconds of playback, the collector takes two consecutive 300-second windows,
with one sample per second on the host's monotonic clock. Each window traverses every format.
MediaSession snapshots every 15 seconds confirm which excerpts are playing; a run is accepted
only when all five formats are observed playing in both windows.

- **App CPU** comes from process user and system tick counters. Process counters include work by
  threads that exited between samples. PID start times distinguish process lifetimes.
- **Android audio/codec CPU** records `audioserver`, `media.swcodec`, and `media.extractor`
  separately. These shared services can contain background work; their combined total is not a
  whole-device CPU measurement. The landing page names the processes its CPU metric includes.
- **Observed voluntary thread switches** sum deltas for app threads readable in adjacent samples.
  A thread that exits during a read is omitted from that snapshot. Work between observations can
  be missed, so these counts are not a complete device wakeup count.
- **Quiet app intervals** have the same observed thread IDs at both ends and fewer than 12 observed
  voluntary switches. They are a sampling proxy, not CPU sleep residency.
- **PSS** is a post-window app memory reading. Android's shared audio services are excluded.

The device reports 100 clock ticks per second. CPU is expressed as a percentage of one virtual
core, using the actual elapsed time between snapshots. Results retain individual windows as well
as their arithmetic means. The compressed samples contain the original counters.

## Launch

After playback measurement, five force-stopped launches use `am start -W`. Each must report
`LaunchState: COLD`. The reported value is the median `TotalTime`. Library and filesystem caches
remain warm. This measures the first activity frame, not complete library readiness.

## Builds

nori uses the optimized, minified beta.5 perf APK with its recorder disabled. Symfonium and Navic
use their official release APKs. Musly uses the previously documented v2.0.2 release build with
its emulator startup guard removed; its playback code is unchanged. APK hashes and versions are
retained in `summary.json`. The Musly build diff and download provenance are in the earlier
[synthetic-input report](../android-emulator-2026-10-14/README.md).

The earlier tone measurements are retained for provenance. They are a different workload and
must not be combined with these music measurements.

## Reproduce

Use a dedicated test emulator on port 5556. `start.py` clears each measured app's local data. Install
the documented APKs first. Start a separate Navidrome on host port 4541 with the corpus mounted at
`/music`, an admin/admin fixture account, and a playlist named `website-bench-mixed`. Its order is
the ten entries in `workload.json`, followed by the first entry again. `tools/bgtest/server.py`
creates the corpus from the owner's existing source library; readers need their own music files.

From the repository root, run each app sequentially:

```sh
python3 docs/performance/raw/android-emulator-music-2026-10-14/run.py nori
python3 docs/performance/raw/android-emulator-music-2026-10-14/run.py symfonium
python3 docs/performance/raw/android-emulator-music-2026-10-14/run.py musly
python3 docs/performance/raw/android-emulator-music-2026-10-14/run.py navic
```

New recordings go to `build/website/emulator-music-replay/`, leaving the published snapshot intact.
The collector emits playback JSON and uncompressed samples. Published samples are gzip-compressed.
Check every session audit for all five formats before comparing an app's two windows.
