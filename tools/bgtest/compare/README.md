# Emulator comparison: nori, Symfonium, Musly, Navic

Applies to the nori Android perf build at 0.7. The published numbers are on
[norifm.com](https://norifm.com/docs/performance/).

Each app plays the same playlist on one emulator, with the screen off and the media volume at 0. After
15 seconds of playback, `collector.py` samples every second for two 5-minute windows:

- CPU of the app, and of Android's audio services (`audioserver`, `media.swcodec`, `media.extractor`);
- voluntary context switches (wakeups) of the app's threads;
- quiet intervals: seconds with fewer than 12 wakeups and no thread started or ended;
- PSS at the end of each window.

Then `cold-start.py` times five cold launches.

## Builds

| app | build |
|---|---|
| nori | the perf APK (`dev.nori.music.perf`) |
| Symfonium, Navic | the official release APKs in `sources.json` |
| Musly | built from the tag in `sources.json`, with the Flutter and Python versions listed there. Its release APK refuses emulators; `musly-emulator.patch` removes that startup check and leaves playback untouched |

## Run

1. Start a dedicated emulator on port 5556 (`emulator-5556`). `start.py` clears each app's data.
2. Start a separate Navidrome on host port 4541 with the corpus at `/music`, an `admin`/`admin`
   account, and a playlist named `website-bench-mixed`: the entries of `workload.json` in order, then
   the first one again. `tools/bgtest/server.py` builds the corpus from a source library.
3. From the repository root, one app at a time:

```sh
python3 tools/bgtest/compare/run.py nori
python3 tools/bgtest/compare/run.py symfonium
python3 tools/bgtest/compare/run.py musly
python3 tools/bgtest/compare/run.py navic
```

Results go to `build/website/emulator-music-replay/<app>/`. Before comparing an app's two windows,
check that its session audit (`sessions.json`) covers all five formats. The published summary is
`src/data/benchmarks/` in the website's repository.
