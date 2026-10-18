---
title: Downloads and offline
description: Downloading music, the stream cache, and playing on when the server is out of reach.
sidebar:
  order: 4
---

Applies to nori 0.6 on every client, except where a line names one.

## Downloads

Download a song, an album, a playlist, everything by an artist, or the whole library. Downloads play
without a connection and are listed on the Downloads page with their progress, speed and time left, and
a retry for any that failed.

Android, Android Auto, the iPod touch, and the CLI group finished downloads by album, with the most
recently downloaded album first. Open an album to see and play only its downloaded tracks in disc
and track order. Songs without an album stay in a separate list. On Android, **Download queue**
opens the progress and retry controls.

| Setting | Default |
|---|---|
| Quality for downloads | Original (or MP3 320, or Opus 192, 128, 96 or 64, transcoded by the server) |

Four songs download at once (from nori 0.6.3; before, it was a setting); the rest wait their turn.

On the iPod touch, downloads run while the app is open or playing. Downloaded album pages use
the saved tracks, and cached covers decode without reaching the server. In 0.6.0-beta.3 and later,
unavailable artwork finishes loading with a placeholder.

## The stream cache

Songs you stream are kept in a cache on the device (1 GB by default; 256 MB to 16 GB), so a song played
again is not fetched again. The songs coming up next are fetched ahead: three on Wi-Fi, the next one on
mobile data, and the covers of the next five (from nori 0.6.3; before, these were settings).

## Offline

Without a network, nori opens on what it has stored: your library, its covers and your downloads. What
you do meanwhile (favorites, plays, playlist edits) is queued and sent when the server is back.

**Play downloads when offline** (off by default): if the server drops mid-queue, nori jumps to a
download further on in the queue, or plays from your downloads, and comes back to your queue when the
server does.
