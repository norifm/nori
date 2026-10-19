---
title: Terminal
description: The terminal client, with covers drawn in the terminal.
sidebar:
  order: 3
---

Applies to nori 0.6 on Linux and macOS terminals.

The terminal client is a whole music player in a terminal, drawn with ratatui. Start it with
Download the `nori-cli` archive for your system and processor, or build it with
`cargo run --release -p nori-cli` (see [Install](../start/install.md#desktop-and-terminal)).

## The window

Two rounded outlines frame the page. Along the top are the tabs, Home, Albums, Artists, Songs, Playlists and
Favorites (the open one in the accent color, bold and underlined), then search and settings. Along the
bottom is the player: the cover, the song; shuffle, previous, play, next and repeat with the seek bar under
them in the middle; on the right the seal, then ♫ playing, ≡ queue, ❝ lyrics, ⇄ devices and ⤢ full screen,
with the volume under them. Downloads, the equalizer and the server's state are in the line of keys under the
player.

A narrow terminal keeps what fits: under 80 columns the buttons after the seal, the volume and the tabs' names
go, under 60 the player is the song and play.

The page scrolls under the outlines and fades into the page color before them: the two rows nearest an
outline blend a third, then two thirds of the way. The terminal cannot blur, so this stands in for it, and
only where the page really is scrolled.

Clicking the song, ⤢ or `F` opens the full player as it was last: the cover, the title with its seal, the
seek bar, the transport and the volume on the left, and beside them the queue, the lyrics or the devices.
Playing (♫ or `P`) shows the song alone in the middle. `tab` moves between them, `esc` goes back.

## Colors

The page is painted in the colors of the season, which follows the calendar (a week before a season starts to a
week after, the colors blend) or stays on the one chosen in **Settings, Look** (picking a season there stops
following the calendar; **Follow the seasons** under it starts again), with light, dark or black
(AMOLED) appearance; "as the terminal" takes light or dark from the terminal's background (`COLORFGBG`). The
accent marks only what is current: the open tab, the song playing, shuffle or repeat while on, a liked seal.
A selected row is the raised surface, never the accent.

- Terminals that do not report true color (`COLORTERM`) get the nearest of the 256 colors.
- `NO_COLOR` shows no colors at all: weights, underline and reverse video only.
- `--no-paint`, or **Transparent background** in **Settings, Look**, leaves the terminal's own background and
  text, with only the season's accent on top.
- The seal 好 is the favorite: quiet when not liked, on the accent when liked.
- Pressing play, next, previous, shuffle or repeat (by key or mouse) shows the glyph reversed for a quarter of
  a second, and a like does the same to the seal. Nothing else moves, and nothing redraws on its own while the
  music is paused. `NORI_REDUCE_MOTION=1` turns both off.

## Covers

Covers are drawn in whichever picture protocol the terminal answers to: kitty graphics, sixel or
iTerm2's, and in half blocks anywhere else. `I` turns them off.

Under **tmux**, a tmux that draws sixel keeps the picture in the pane. Otherwise pictures pass through to
the outer terminal, and nori draws them again when the pane has focus back; it turns tmux's
`focus-events` on for that.

## Keys

`?` shows every key. The main ones:

| Key | Does |
|---|---|
| `space` | play or pause |
| `n` / `p` | next song, previous (or the start of this one) |
| `←` `→` or `,` `.` | back or forward 5 seconds; with shift, 30 seconds |
| `+` / `-` | volume |
| `s` / `r` | shuffle on or off; repeat off, all, one |
| `/` | search |
| `1` … `9` | Home, Albums, Artists, Songs, Playlists, Favorites, Downloads, Equalizer, Settings |
| `P` / `Q` / `L` / `C` | the full player on the song alone, its queue, lyrics or devices (again: back) |
| `tab` | in the full player, the next panel |
| `t` | radio from the playing song, where the server has it |
| `F` / `N` | the player over the whole window |
| `enter` / `l` | open, or play from here |
| `a` / `A` | add to the queue, or play next |
| `x` / `X` | play the whole page, or shuffle it |
| `D` / `f` | download; favorite |
| `i` / `o` | start a jam with the selected item; join one from its link |
| `m` | mouse on or off (off: the terminal selects text) |
| `q` | quit |

## Cost

Idle or paused, the client does not wake at all. Playing, its screen wakes about once a second for the
clock, and a few times a second with word-by-word lyrics on screen.
