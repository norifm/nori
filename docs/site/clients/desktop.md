---
title: Desktop
description: The desktop client for Linux and macOS.
sidebar:
  order: 2
---

Applies to nori 0.6 on Linux and macOS.

The desktop client is a window over the same core, drawn with Slint. It plays through the system's sound
(PipeWire or ALSA on Linux, CoreAudio on macOS), shows its controls, the song's cover and its seeks to the desktop (MPRIS on Linux), and
has the library, search, the queue, lyrics, the equalizer, settings, [remote control](../connected/remote-control.md)
and [jams](../connected/jams.md).

## Layout

The window has no sidebar. A rounded outline runs across the top: Home, Albums, Artists, Songs, Playlists and
Favorites (the current one named and lit), then the search field, which widens as you type and fills the page
with what it finds, and the settings menu, which holds your account, the
equalizer, your other devices, joining a jam, and the season and appearance. Playlists opens your playlists
under it. A second outline along the bottom is the player: the song on the left; shuffle, previous, play, next
and repeat with the seek bar under them; and on the right the seal, the speaker (your devices), lyrics, the
queue and the volume. Pages scroll under both and fade out before them. A narrow window drops the tab names,
then the volume slider and the panel buttons.

A favorite is a seal: an outline until you like a song, then filled and slightly tilted. Lyrics, the queue and
your devices open in a sheet that floats on the right between the two outlines, over the page, which stays
where it is; tabs at its top switch between them. In the full player the same tabs head the panel beside the
cover. The queue shows the songs already played, dimmed above the one playing, then what plays next. With
nothing typed, the search page lists your recent searches.

## Look

nori wears one of four seasons, Spring (春), Summer (夏), Autumn (秋) and Winter (冬), in one of four modes:
Follow system, Day, Night or Midnight (true black). By default it follows the calendar and blends from one season to the next over two weeks
(southern-hemisphere zones turn half a year later). Choose a season and the appearance in Settings, General, or
in the settings menu. The accent color marks only what is current: the open tab, the song playing, a lit
shuffle or repeat, a liked seal. Album and artist pages and Now Playing wear their cover's colors, except in Midnight; the side sheet wears the page's own.
With Motion set to Reduced in nori's settings, nothing slides, stamps or fades.

## YouTube Music

Sign in from Settings, Servers, the profile's Edit: Sign in with Google opens Google's own sign-in page in a
small window (the system's web view: WebKit on macOS, WebKitGTK on Linux) and closes it once you are back on
YouTube Music signed in. Nothing of that window is kept. On Linux nori uses the system's WebKitGTK 4.1 (see
[Install](../start/install.md#desktop-and-terminal)); Paste cookies instead takes cookies copied from a
browser. Tapping a song's seal while signed out opens the same sheet. When YouTube ends the session, the sheet says so at once and each time nori starts,
until you sign in or choose Don't show again.

## Install

Download a Linux package or macOS app bundle: see [Install](../start/install.md#desktop-and-terminal). It shares its data
directory with the terminal client.

## Limits

- Signing in takes an address, user and password or an API key, and a second address; extra HTTP headers
  and self-signed certificates are Android's only for now.
- HE-AAC radio stations play without their highest frequencies (above 11 to 12 kHz), since no free
  decoder for that part is linked.
