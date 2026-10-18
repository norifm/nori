---
title: Timing, translations and display
description: Nudging lyrics that drift, showing translations, and how lyrics are drawn.
sidebar:
  order: 3
---

Applies to nori 0.6.3. The display settings are Android's; the terminal's keys are listed below.

## Lyrics that drift

nori checks lyrics against the song's voice once the song is analyzed, and shifts them when they run
late. A song playing for the first time is analyzed once all of it has arrived, whether AutoMix is on
or not; lyrics already on screen are checked then, while it plays. Lyrics from your server are shown as they are. On Android, **Lyrics** in the player's ⋯ menu (there when the song has lyrics) says where they came
from, whether they are timed by the line or the word, and how their timing was set: not checked yet,
adjusted automatically, or adjusted by you. The **−** and **+** buttons beside the offset move them
later or sooner in 50 ms steps (250 ms before 0.6.3). Manual offsets round to the nearest 50 ms. Your timing replaces
the automatic one, is kept for that song and those lyrics, and comes back
the next time they play; **Reset** returns to the automatic timing. Lyrics from another service, or
the same service's lyrics timed again, start without your timing.

In the terminal, `[` and `]` move the words later or sooner for as long as the song plays, and `0`
puts them back.

## Translations

**Show translations** (on by default) shows a translation under each line when your server has one.

## Display

| Setting | Default | What it does |
|---|---|---|
| Fill in words as they're sung | on | for lyrics timed word by word |
| Text size | medium | small, medium or large |
| Keep the screen on | on | while lyrics are shown and music plays |

Tap a line to play from it.
