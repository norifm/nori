---
title: Jams
description: Letting friends join with a link, ask for songs and listen along.
sidebar:
  order: 2
---

Applies to nori 0.6 on Android, the desktop, the terminal and the iPod touch. Server profiles need octo-fiesta. Current development builds also support YouTube Music jams on the local network.

A jam is a session others join without an account. **Remote control and jams** is enabled by default (Settings, Library, Other devices).
Start a jam from the devices button, or from a song's, album's or playlist's menu.

## Inviting

**Invite** shows a QR code and a link. Others join by scanning the code with nori, or by opening the
link. If your server has only a home address, the invite says so: it then works only on your home
network, and a public address in the server settings lets guests join from elsewhere.

## YouTube Music on the LAN

A YouTube Music host can start a jam without a server. Its invite works only on the same network.
Guests use a temporary, signed-out YouTube Music profile and resolve each song themselves. Joining
preserves the profile they were using; leaving restores it. Google cookies and account identifiers are
never included in the invitation or shared with guests.

The invite and each guest's credential are separate from paired-device control. Guests keep the same
request and admin roles shown below; a guest cannot control the host as an owner. Removing a guest
revokes its credential, and ending the jam revokes every guest and its invite. Guests must join a new
room with a new invitation after it ends.

## Roles

| Role | Can |
|---|---|
| Host | play, accept or refuse requests, make a guest an admin, remove anyone |
| Admin | play songs, pause, skip, seek, rearrange the queue, add songs directly, accept or refuse requests |
| Guest | ask for songs; each guest may have five requests waiting at once |

A song a guest asks for waits under **Asked for** until the host or an admin accepts it. Provider songs
asked for are not fetched until they are accepted. **Allow remote songs** is off for each new room;
the host must enable it before guests or admins can request songs from external providers.

## Listening along

With **Let guests listen along** on (the default), guests can play the jam's music on their own devices,
in step with the host, its speed, skipped silence and transitions included. Change it in the room settings next to **Invite**. These settings apply only to the current room.
In the terminal, press s in the invite to open room settings.

On Android and the iPod, **Not playing here** means the jam is shown without playing audio on that
device. Choose **Listen here** to hear it. The host's song, progress, and **Mixing** indicator still
update while watching.

In current 0.6 development builds, an admin's tap on a song plays it immediately. **Play next** and **Add to queue**
add it directly, without an approval request. Guests still request songs from the host.

Listening devices use the host's transition plan and timing, including joining or resuming partway
through a mix. Each device keeps its own equalizer and output processing. Controlling another device
also shows that device's **Mixing** indicator.

In the terminal, `i` starts a jam around the selected item and `o` joins one from its link.
