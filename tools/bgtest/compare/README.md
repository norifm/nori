# Emulator comparison: nori, Symfonium, Musly, Navic

Plays the same playlist in each app on one emulator, screen off and media volume 0, and records CPU,
context switches, low-wakeup intervals and PSS over two 90-second windows, then five cold launches.
The published numbers are on [norifm.com](https://norifm.com/docs/performance/).

## Builds

- nori: the perf APK (`dev.nori.music.perf`) with its recorder off.
- Symfonium and Navic: the official release APKs (`sources.json`).
- Musly: built from the tag in `sources.json` with the Flutter and Python versions listed there. Its
  release APK refuses emulators; `musly-emulator.patch` removes the startup guard and leaves playback
  untouched.

## Run

Use a dedicated emulator on port 5556; `start.py` clears each app's data. Start a separate Navidrome on
host port 4541 with the corpus at `/music`, an admin/admin account, and a playlist named
`website-bench-mixed`: the entries of `workload.json` in order, then the first one again.
`tools/bgtest/server.py` builds the corpus from a source library.

From the repository root, one app at a time:

```sh
python3 tools/bgtest/compare/run.py nori
python3 tools/bgtest/compare/run.py symfonium
python3 tools/bgtest/compare/run.py musly
python3 tools/bgtest/compare/run.py navic
```

Results go to `build/website/emulator-music-replay/<app>/`; the published summary is `src/data/benchmarks/` in the website's repository. Check each session audit covers all five
formats before comparing an app's two windows.
