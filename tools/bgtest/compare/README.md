# Device comparison: nori, Symfonium, Musly, Navic

Applies to the nori Android perf build at 0.8. The published numbers are on
[norifm.com](https://norifm.com/docs/performance/), measured on the bench phone (a Galaxy S21 FE).

Each app plays the same playlist on one device, with the screen off and the media volume at its lowest
step (some phones mute the stream at 0). After 15 seconds of playback, `collector.py` samples every
second for two 5-minute windows:

- CPU of the app, and of Android's audio services (`audioserver`, `media.swcodec`, `media.extractor`);
- voluntary context switches (wakeups) of the app's threads;
- quiet intervals: seconds with fewer than 12 wakeups and no thread started or ended;
- PSS at the end of each window.

Then `cold-start.py` times five cold launches.

## Builds

| app | build |
|---|---|
| nori | the perf APK (`dev.nori.music.perf`) |
| Symfonium, Musly, Navic | the official APKs in `sources.json` (Symfonium's from its beta repository) |

## Run

1. Start a separate Navidrome on host port 4541 with the corpus at `/music`, an `admin`/`admin`
   account, and a playlist named `website-bench-mixed`: the entries of `workload.json` in order, then
   the first one again. `tools/bgtest/server.py` builds the corpus from a source library.
2. Install the four apps. `start.py` clears each app's data. On a phone, `run.py` keeps its settings
   (keyboards, screen timeout, volume, app languages) and puts them back however the run ends. Nothing
   else may drive the phone meanwhile: another UiAutomation client (a device mirror's helper) makes
   every screen read fail.
3. From the repository root, one app at a time:

```sh
python3 tools/bgtest/compare/run.py nori --serial <phone> --server http://<host LAN address>:4541 --output build/website/phone-music-replay
python3 tools/bgtest/compare/run.py symfonium --serial <phone> --server ... --output ...
python3 tools/bgtest/compare/run.py musly --serial <phone> --server ... --output ...
python3 tools/bgtest/compare/run.py navic --serial <phone> --server ... --output ...
```

Without `--serial` and `--server` it runs on `emulator-5556` against `http://10.0.2.2:4541`.

Results go to `<output>/<app>/`. Before comparing an app's two windows, check that its session audit
(`sessions.json`) covers all five formats. The published summary is `src/data/benchmarks/` in the
website's repository.
