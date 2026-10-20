---
title: AutoMix and crossfade
description: How nori blends one song into the next, and when it does not.
sidebar:
  order: 4
---

Applies to nori 0.7.0 and later on every client. Before 0.7.0 every part of a mix had its own switch;
from 0.7.0 they are all on, and a crossfade's curve and fade lengths are gone (a crossfade set before
becomes the longer of its two fades). **Keep albums gapless** stays a switch and keeps the choice made
before.

## AutoMix

**AutoMix** (off by default) blends songs like a DJ, matching the beat. nori measures each song on the
device as it arrives: its beats, bars, tempo, key and loudness. With both songs of a transition
measured, it plans a mix of up to 12 seconds: it nudges the next song's speed (by up to 6 %, keeping the
pitch) so the beats line up, swaps the bass, fades the outgoing song out muffled, and ends clashing
vocals in an echo.

Songs are measured once, when first played or coming up next, whether AutoMix is on or not: the same
measurement checks the lyrics' timing against the singing. It costs a little CPU once per song.

**Better beat detection** (off by default) runs a small neural beat tracker (Beat This!) over the start
and end of each song, for mixes that land on the beat more often, and over downloaded songs as they
arrive. It uses more battery. Its one-time download of about 8 MB from the model's authors comes over any
network.

### When AutoMix does not mix

- **The songs are not measured yet.** A song is measured as it streams or downloads. Until both songs
  are, the transition is a plain fade.
- **An album plays in order.** With **Keep albums gapless** (on by default), songs of one album that
  follow each other in order join without a gap, as the album was made. Shuffled, they mix. Switched
  off, they mix like any others.
- **The tempos are too far apart.** Beyond the biggest speed change, nori does not force a beat match.
- **Provider songs** from octo-fiesta are measured only as they play or once downloaded, never fetched
  for it (from nori 0.7.0; before, never measured).

## Crossfade

Without AutoMix, **Crossfade** overlaps the end of one song with the start of the next for 1 to 12
seconds (off by default), at equal power so the level holds through the overlap. Songs of one album in
order are not crossfaded either while **Keep albums gapless** is on. **Fade on play and pause** softens starts and stops (off, or 150 ms
to 1 s).

Gapless playback needs no setting: songs that run into each other join sample for sample.
