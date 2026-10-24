# bgtest: battery tests on a real phone

Applies to the nori Android perf build (`dev.nori.music.perf`) at 0.7, against Musly, Navic and
Symfonium.

Each run wipes a player (`pm clear`), logs it in, sets one variant (equalizer, AutoMix, offload,
crossfade...), plays the same playlist in the same order, skips a few songs and measures the same
stretch, either with the screen off or with the full-screen player on screen. Every step checks the
screen it should reach and takes a screenshot when it does not. The phone is put back as it was however
the run ends.

The emulator comparison behind the published performance page is in [compare/](compare/README.md).

## Run

```sh
tools/bgtest/run.py                      # in a terminal: one screen to pick the phone, runs, playlists
tools/bgtest/run.py --list-devices
tools/bgtest/run.py --serial <serial> --wifi --plan quick       # every app, default settings, 3 min each
tools/bgtest/run.py --serial <serial> --wifi --plan matrix      # every setting that changes the playback path
tools/bgtest/run.py --serial <serial> --wifi --server local --playlist bg-mp3,bg-quick \
    --runs nori:plain,nori:eq-automix,musly:default,navic:default,symfonium:default --minutes 20 --repeat 3
tools/bgtest/run.py --resume             # continue the newest session that stopped
tools/bgtest/bgtest.py --list            # the apps and their variants
tools/bgtest/bgtest.py --restore         # only put the phone back as a stopped session left it
tools/bgtest/server.py                   # start the local test server
```

| option | default | what |
|---|---|---|
| `--scenario` | `screen-off` | `screen-off`, `player` (full-screen player on screen), or both, comma separated |
| `--order` (bgtest.py) | `play` | `play`: the list in order in every app; `shuffle`: each app's own shuffle |
| `--volume` | `1` | media volume step for every run, or `keep`; put back afterwards |
| `--brightness` | `1` | screen brightness 1 to 255 in the player scenario |
| `--nori-pkg` | `dev.nori.music.perf` | or `dev.nori.music` for a normal build |
| `--stay-plugged` | off | measure on the cable (no real current then) |

- With `--wifi`, a phone on a cable moves to adb over Wi-Fi and the run waits for the cable to be
  pulled, so it measures on battery.
- One run per phone at a time: a second one on the same serial refuses to start (a lock in
  `build/bgtest/`).
- With no terminal (an agent), `run.py` never asks: every choice is a flag. `ACTION:` lines are for the
  person at the phone, and the last line is `RESULTS: <folder>`.

## When the phone drops

- Before every run, adb over Wi-Fi reconnects (`adb disconnect`, `adb connect`).
- An adb call that finds the phone gone reconnects and waits up to 3 minutes for it. A run whose
  batterystats still cannot be read is recorded as failed.
- A phone that never comes back stops the session cleanly (exit code 3) and keeps the phone's record,
  so `--resume` or `--restore` can put it back.
- `--resume` shows what the session ran, what is left and when it would end, asks, then runs the rest
  with the session's own settings into the same folder. Each session keeps its settings and plan in
  `session.json`.

## Files

| file | what |
|---|---|
| `run.py` | the entry point: picks the phone, moves it to Wi-Fi, starts the local server, runs `bgtest.py` |
| `tui.py` | the picker `run.py` shows with no options |
| `bgtest.py` | the runs, the measuring (batterystats, real current, AudioFlinger's view of the app's track, frames, network) and the tables |
| `apps/` | one module per player: log in, variants, start and download the playlist, open the player |
| `phone.py` | what is changed on the phone, remembered first and put back |
| `ui.py` | adb, and a UI driver that finds things by what the screen says |
| `server.py` | a local Navidrome (port 4540, admin/admin), below |
| [`SETTINGS.md`](SETTINGS.md) | where each player keeps each setting, and each app's variants |

Kept out of git, under `build/bgtest/`:

- `apks/<package>.apk`: installed when a player is missing.
- `server/`: the test server's music and database.
- `results/<time>/`: `results.md`, `runs.jsonl`, and each run's batterystats, log and screenshots.

## The test server

`server.py` downloads songs once from the server in `~/.music.pass` and builds these playlists:

| playlist | songs |
|---|---|
| `bg-mp3` | the MP3s as they are (320 kbps, 44.1 kHz) |
| `bg-flac-44` | FLAC 16-bit 44.1 kHz |
| `bg-flac-48` | FLAC 24-bit 48 kHz |
| `bg-flac-96` | FLAC 24-bit 96 kHz |
| `bg-48k` | the original 48 kHz MP3s |
| `bg-mixed` | a bit of everything, in turn |
| `bg-quick` | 3 short fillers for the skips, then one-minute clips in each format, twice: a 5-minute run hears every format |

## What the numbers are

- batterystats' figures are estimates from CPU and wakelock time. Its fixed audio-hardware and screen
  models are kept apart: they are the same for every app and charged to an app only on some paths.
- The real current (`mA`, `hours`) is measured only on battery.
- Musly and Navic decode in Android's mediacodec service. Its figure is added to theirs.
