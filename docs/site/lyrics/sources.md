---
title: Lyrics sources
description: Where nori finds lyrics, in what order, and what it sends to find them.
sidebar:
  order: 1
---

Applies to nori 0.6.3 and later on every client. The list of sources and its order are edited on Android, the
desktop and the terminal.

## Your server first

Your server's own lyrics always come first. Navidrome and other OpenSubsonic servers can serve them
timed by line or by word. When the server has timed lyrics, no other source is asked.

## SourWhen the server has no timed lyrics, **Find missing lyrics online** asks the lyrics services below. It is
on by default and sits under **Look things up online**; turning either off means nothing is asked. A lookup
sends the artist, song and album name, and the song's length, and happens only when you open the lyrics.
Provider songs from octo-fiesta are never looked up.

:::caution[Unofficial sources]
Only Unison and LRCLIB are open services with a documented API. The others are unofficial: they serve
lyrics taken from other apps and sites, which did not agree to it. Most are on by default; switch off any
you would rather not use, or turn off Find missing lyrics online altogether.
:::

The sources, in their default order:

| # | Source | Finest timing | Asked |
|---|---|---|---|
| 1 | LyricsPlus | words | first |
| 2 | BetterLyrics | words | first |
| 3 | LRCLIB | words where published, else lines | first |
| 4 | KuGou | words | second |
| 5 | NetEase Cloud Music | words | second |
| 6 | BiniLyrics | words | second |
| 7 | PaxSenix | words | second |
| 8 | Unison | words | second |
| 9 | PaxSenix: Musixmatch | words | off until you give your PaxSenix key |
| 10 | PaxSenix: Spotify | words where supplied, else lines | off until you give your PaxSenix key |
| 11 | YouTube Music | untimed | off by default |

nori 0.6.3 removed SimpMusic, YouTube captions, Megalobiz, BetterLyrics Portato and Genius: measured over
some three hundred songs, they answered nothing, or nothing the others did not answer better. A list you
reordered keeps its order without them; a list left as it came takes the new default.

ntimed | |

## How the best answer is chosen

The first sources are asked together; the second only when those find nothing good. Every
answer is scored on how well it matches the song (title, artist, album and length), how finely it is
timed, whether its times make sense, and whether the other sources agree with it. The best one is shown,
and an answer that scores too low is never shown. With **Prefer word-by-word lyrics** on (the default),
nori keeps looking past lyrics timed line by line.

The lyrics chosen are kept on the device with their source, so a song played again asks nobody. Settings,
Storage, Lyrics shows their size and clears them.

## Changing the order

In Settings, Lyrics, Lyrics sources, each source has a switch, and you can hold a row and drag it to
reorder. The order settles near ties between answers; the score decides the rest.
