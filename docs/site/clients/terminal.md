---
title: Terminal
description: The terminal client, with covers drawn in the terminal.
sidebar:
  order: 3
---

Applies to nori 0.7 on Linux and macOS terminals.

The terminal client is a whole music player in a terminal, drawn with ratatui. Download the `nori-cli`
archive for your system and processor, or build it with `cargo run --release -p nori-cli` (see
[Install](../start/install.md#desktop-and-terminal)).

## The window

The window is lists of rows, with as little around them as works:

- The first row names the tabs, Home, Library, Settings and Search (the open one in the accent color), and
  on the right the server, with "offline" or "unreachable" before it when it is.
- The page fills the middle.
- Two rows on a slightly raised paper show the song playing: play or pause, the title, its artist and
  album, and on the right shuffle and repeat while they are on and the seal; under them the seek bar, where
  the music plays and the volume. A narrow window drops where it plays, then the volume; a short one keeps
  only the song's row.
- The last row shows the keys of what has them, or a note for a few seconds. A click on it shows every key.

Clicking the song or pressing `F` opens the full player as it was last: the panels' names along the top
(Playing, Queue, Lyrics and, with remote control on, Devices), and the cover, the title with its seal, the
seek bar, previous, play and next, where the music plays and the volume beside the panel. Playing (`P`)
shows the song alone. `tab` moves between the panels, `esc` goes back. The queue's head says what is left
and holds Shuffle and Repeat.

## Home

Home lists, in order:

- The queue another device or app left on your server, when it stopped elsewhere: "Continue", the song and
  where it stands. Enter or **Resume** plays it from there, `d` or **Close** turns it down; either way it is
  not offered again (**Continue from other devices** in Settings, Library).
- For you: Favorites and the mixes made for you. Enter opens a mix's page.
- Home's rows (Recently added, Most played albums, Favorite albums and so on), each with its first five entries. Enter on a row's name lists all of it,
  read a page at a time as you go down.

## Library

The Library tab shows its sections, Albums, Artists, Songs and Playlists (`[` and `]` go to the one before
or after). Each starts with the library's places, Favorites, Downloads (with what
they weigh), History and Shuffle, then its entries, the first of them selected.

Albums and songs have an order, named at the top right: `S` goes to the next and it is kept. Albums go by
Favorites, Title, Artist, Year, Added, Most played and Random; in Artist order each artist's albums are
listed under their name, which opens their page. Songs go by Favorites (only those), Title, Artist, Album,
Year, Added, Most played and Longest.

## Pages

An album's, artist's, playlist's or mix's page starts with its name and seal, then who made it with Play
and Shuffle (and a mix's Save as playlist, which names the playlist after the mix, as "nori discover" or "nori discover 2"
if that name is taken), then the list's head: what it holds and what can be done with it, as Download (or
"Download the other 3", or "Remove downloads"), Add to queue and a mix's New mix. An artist's albums are
listed under their shelves: Albums, EPs, Singles and so on.

`m` opens the menu of the song selected, its lines grouped as on the phone (the less used under More),
followed on a page by the page's own. On an album or a playlist in a list it holds Play, Play next and Add
to queue. In the full player it is the menu of the song playing.

## Colors

The page is painted in the colors of the season, which follows the calendar (a week before a season starts
to a week after, the colors blend) or stays on the one picked in **Settings, Appearance**: the four seasons
are listed with their kanji and their paper, accent and on colors, and picking one stops following the
calendar (**Follow the calendar** under them starts again). **Appearance** is one of four modes: System
(Day or Night from the terminal's background, `COLORFGBG`), Day, Night or Midnight (true black, and the
player wears no cover colors). Otherwise the full player's paper is the season's dyed with the cover. The
accent marks only what is yours, now: the open tab, the song playing, shuffle or repeat while on, a liked
seal. A selected row is raised, never the accent.

- Terminals that do not report true color (`COLORTERM`) get the nearest of the 256 colors.
- `NO_COLOR` shows no colors at all: weights, underline and reverse video only.
- `--no-paint`, or **Transparent background** in **Settings, Terminal**, leaves the terminal's own
  background and text, with only the season's accent on top.
- The seal 好 is the favorite: quiet when not liked, on the accent when liked.
- Pressing play, next or previous (by key or mouse) shows the glyph reversed for a quarter of a second, and a
  like does the same to the seal. Nothing else moves, and nothing redraws on its own while the music is
  paused. `NORI_REDUCE_MOTION=1` turns both off.

## Covers

The full player draws the playing song's cover in whichever picture protocol the terminal answers to:
kitty graphics, sixel or iTerm2's, and in half blocks anywhere else. `I` turns covers off. Lists show no
pictures.

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
| `[` / `]` | the library's section, or the settings' group, before or after |
| `S` | the albums' or songs' next order |
| `P` / `Q` / `L` / `C` | the full player on the song alone, its queue, lyrics or devices (again: back) |
| `tab` | in the full player, the next panel |
| `t` | radio from the playing song, where the server has it |
| `F` / `N` | the player over the whole window |
| `enter` / `l` | open, or play from here |
| `m` | the menu of what is selected, or of the song playing in the full player |
| `a` / `A` | add to the queue, or play next |
| `x` / `X` | play the whole page, or shuffle it |
| `D` / `f` | download; favorite |
| `d` | on Home, close the queue offered; in the queue, take a song out |
| `i` / `o` | start a jam with the selected item; join one from its link |
| `M` | mouse on or off (off: the terminal selects text) |
| `q` | quit |

## Cost

Idle or paused, the client does not wake at all. Playing, its screen wakes about once a second for the
clock, and a few times a second with word-by-word lyrics on screen.

## YouTube Music

Settings, Servers, Enter on the YouTube Music profile in use opens its account sheet: Enter signs in with
Google in a small window that the terminal's own program opens (the system's web view, as on the desktop),
**p** pastes cookies instead, and a signed-in account's sheet signs out. Pressing **f** on a song while
signed out opens the same sheet. When YouTube ends the session, the sheet says so at once and each time
the terminal starts, until you sign in or press **d** (don't show again). See
[YouTube Music](../start/youtube-music.md#signing-in-optional).
