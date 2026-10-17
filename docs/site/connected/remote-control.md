---
title: Remote control
description: Playing on one of your devices and controlling it from the others.
sidebar:
  order: 1
---

Applies to nori 0.6 on Android, the desktop, the terminal and the iPod touch.

With **Remote control and jams** on (Settings, Library, Other devices; on by default), your devices
with nori, signed in to the same server account, see each other. One plays at a time; the others control it.

- **Moving the music.** The output button lists this device, your other devices and this device's own
  outputs. Choosing another device moves the playback there: the queue, its order, shuffle, repeat and
  the place in the song. On 0.6.0-beta.6 and later, the listening time and the original listen start
  move too, so a qualifying listen is scrobbled once, by the device that finishes it. Both devices
  need 0.6.0-beta.6; with an older one, each device counts its own part of the song.
- **Controlling it.** While another device plays, the player, the notification, the lock screen, the car
  and the volume keys show it and control it. The seek bar and **Mixing** indicator follow the playing
  device. The device you hold stops its own playback.
- **The terminal** lists your devices in its right panel (`C`).
- **Several clients on one computer.** On 0.6.3 and later, the desktop app and the terminal client
  running on one computer are separate devices, as is a second copy of either. Devices that share a name
  are listed with their kind, as "MacBook (computer)" and "MacBook (terminal)". The first client to start
  after the update keeps the computer's existing device.

On Android 11 and newer, the system media output picker also lists your nori devices while you
control remote playback. Choose **This phone** there to bring playback back without opening nori.

Existing installs keep the feature enabled if either remote control or jams was enabled before
the switches were merged. Turning the merged switch off ends hosted rooms and disables discovery.

On 0.6.0-beta.3 and later, a skip accepted by the playing device stays on the requested song while
that device loads it. It does not briefly return to the previous song when loading takes longer.

## How devices reach each other

On the same network, devices talk straight to each other, found by mDNS and signed with your Subsonic
credentials. Elsewhere they go through octo-fiesta's relay on your server. Nothing goes through any other
service.

Switched on, it costs one held request a minute while music plays or another device is shown; off,
nothing at all.

In 0.6.0-beta.3 and later, nearby devices signed in to the same account share song, album, and artist favorite changes while Remote control is on. The receiving device updates its hearts and favorite lists without a notification. Updates use the authenticated local connection; older clients keep their existing remote playback controls. Discovery for these updates runs while the app is in the foreground and stops during background playback.
