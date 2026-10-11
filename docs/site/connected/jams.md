---
title: Jams
description: Letting friends join with a link, ask for songs and listen along.
sidebar:
  order: 2
---

Applies to nori 0.6 on Android, the desktop, the terminal and the iPod touch. Jams need octo-fiesta.

A jam is a session others join without an account. **Remote control and jams** is enabled by default (Settings, Library, Other devices).
Start a jam from the devices button, or from a song's, album's or playlist's menu.

## Inviting

**Invite** shows a QR code and a link. Others join by scanning the code with nori, or by opening the
link. If your server has only a home address, the invite says so: it then works only on your home
network, and a public address in the server settings lets guests join from elsewhere.

## Roles

| Role | Can |
|---|---|
| Host | play, accept or refuse requests, make a guest an admin, remove anyone |
| Admin | add songs straight to the queue |
| Guest | ask for songs; each guest may have five requests waiting at once |

A song a guest asks for waits under **Asked for** until the host or an admin accepts it. Provider songs
asked for are not fetched until they are accepted. **Allow remote songs** is off for each new room;
the host must enable it before guests or admins can request songs from external providers.

## Listening along

With **Let guests listen along** on (the default), guests can play the jam's music on their own devices,
in step with the host, its speed, skipped silence and transitions included. Change it in the room settings next to **Invite**. These settings apply only to the current room.
In the terminal, press s in the invite to open room settings.

In the terminal, `i` starts a jam around the selected item and `o` joins one from its link.
