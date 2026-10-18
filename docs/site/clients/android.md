---
title: Android
description: The Android app, the main nori client.
sidebar:
  order: 1
---

Applies to nori 0.6 on Android 8.0 and later.

The Android app is the main client and has every feature: the full settings, the equalizer and effects,
per-output sound, [widgets](../using/widgets.md), [Android Auto](../using/android-auto.md), and the
system's media controls, notification and lock screen.

- **Install**: see [Install](../start/install.md#android).
- **Look**: pages take their color from the album cover, with light, dark or system theme and pure
  black for OLED screens. From 0.6.3 the theme is one of four seasons, each a palette of traditional
  Japanese colors: spring's plum, summer's dayflower blue, autumn's persimmon and winter's ink with one
  camellia red. By default it **follows the seasons**, turning on the first days of the old calendar's
  seasons (4 February, 5 May, 7 August, 7 November; half a year round south of the equator) over a
  week. The theme's accent marks the current tab, a favorite and what is playing, on every page and in
  the player, whatever the cover; a favorite is a seal stamped with 好. The now-playing bar and the tabs
  share one outline over the page, which shows blurred inside it (Android 12 and later); on its side
  they sit in one row along the bottom. **Colors
  from the cover** is two switches, Pages and Player: where one is off it stays black in dark mode.
  **Keep the screen on** and hiding the status or navigation bar work the same way: each condition
  (sideways, upright, charging, burn-in protection) is a button of its own, and any one lit that holds
  is enough. Text follows Android's font size; with a large display size the layout keeps a phone's proportions.
  Burn-in pixel shifting and idle dimming are off by default from 0.6.3. Upgrading resets the old
  automatic pixel shifting and one-minute dimming once; other dimming intervals are kept.
- **Motion** (from 0.6.3): Full, Reduced (short fades instead of slides and zooms) or E-ink (nothing
  moves, the player redraws only when something changes). nori's choice holds whatever Android's own
  animation setting says; the playing bars move in every mode but E-ink.
- **Battery**: music plays from a deep buffer, or through the phone's audio chip when nothing needs the
  samples. See [Offload and processing](../sound/offload.md).
- **Updates**: the app checks GitHub for a newer release at most once a day and offers to install it.
  Turn this off in the Updates section of Settings, or opt into beta releases there.
- **Moving covers** (off by default): some albums on Apple Music have a short looping video of their
  cover, and the player can play it in place of the still cover, only while the player is open and at
  rest. Finding the video asks Apple's public search and its catalog for the album, sending the artist
  and album name; it is unofficial. One choice sets it: Off, On Wi-Fi or Always (from 0.6.3; before, a
  switch and a separate mobile data switch). See [Privacy and network](../privacy.md).
