# Sync server: pairing, profile sync, remote control and jams

Internal planning note, not published. Written 2026-11-24 against `master` at `9c658784`. Nothing here
is built. It records the owner's decisions from that day and the research behind them. Parts marked
**To test** depend on a check that has not run yet.

## What this replaces

Remote control away from home and jams of a server profile go through a relay in the owner's octo-fiesta
fork (`noriRemote.*`, crates/core/src/remote.rs). Profiles without a server (YouTube Music, SoundCloud)
pair by code and work on the local network only (crates/remote/src/lan). Each device signs in to every
account by itself.

The sync server replaces the relay for every kind of profile. With it comes pairing for all devices and
profile sync between them.

## Goals

- Remote control works on every profile, away from home, with no modified server.
- Jams work with an unmodified Navidrome and an unmodified octo-fiesta.
- The server reads nothing: every message is encrypted between devices.
- The server carries information only, never music.
- The public server stays free or close to free to run.

## Terms

| Term | Meaning |
|---|---|
| Sync server | The relay this note plans. One public instance run by nori, or a custom one |
| Group | The devices one person paired |
| Group key | The key all devices of a group hold. The server never has it |
| Group room | Where a group's devices meet on the server, named by a one-way fingerprint of the group key |
| Meeting room | A one-time room, open for about five minutes, that lets a new device into a group |
| Jam room | Where a jam's host and guests meet, with its key in the invite link after `#` |
| Kept copy | The group's profiles, encrypted with the group key, stored on the server if the group asked |

## Decisions

| Topic | Decision |
|---|---|
| Who runs it | nori runs one public server, the default, which settings can turn off or replace |
| Where a custom one runs | Cloudflare, or one container on the user's machine |
| What "my devices" means | Devices paired on purpose, by code or picked in the pairing screen |
| What pairing covers | Every profile |
| What syncs | Profiles with their sign-ins and plugins, not settings, local files profiles or downloads |
| Per profile | **Sync this profile** in the profile's Advanced settings, on by default |
| Removing a profile | Removes it on every paired device |
| Sign-ins that must not be copied | The plugin says so, and the profile syncs without the sign-in |
| Losing a device | **Reset pairing** leaves the group, with no remote removal |
| Kept copy | The user's choice, asked at the first pairing, with a switch beside the sync server setting |
| When a device is reachable | While nori runs: open, or playing in the background |
| Local network | Paired devices talk directly, and use the server only when they cannot |
| The octo-fiesta relay | Removed from nori in the release that ships the sync server |
| Jam music | Each guest gets it from the source, never through the sync server |
| Jam on Navidrome | One playlist and one share per jam, with guests getting the original file |
| Guest who cannot get the music | Requests only |
| Web client | A full paired member, for the profiles a browser can play |

## The sync server

It keeps rooms. A room holds the connections of its members and passes each message from one member to
the others. It answers time requests, so a jam's listeners share one clock. It serves the jam invite
page. It stores a group's kept copy when that group asked for one. It does nothing else.

It sees a member's network address while that member is connected, room names, how many members a room
has and when messages pass. It never sees names, songs, queues or sign-ins. It keeps no logs and stores
no network addresses. It counts connections and messages, so that the limits are seen coming.

### Limits

These keep it free to run and keep music off it.

- A message is at most about 64 KB.
- Messages and bytes per connection per minute are capped.
- A room holds at most about 20 members.
- A room with no member closes after a few minutes. A kept copy is deleted after some weeks without use.
- New rooms per network address per hour are capped, generously: a carrier or a campus puts thousands
  of people behind one address. The caps per room and per connection do the real work.

### One implementation, two shells

The room logic is one platform-free Rust crate: members, passing messages, the clock, the limits. It
reaches its platform through a small interface: connections, timers, the clock and a small store.

A Cloudflare room is unloaded from memory while idle, which is why an idle room costs nothing. So the
room logic keeps nothing in memory between messages: what it needs travels with each connection. A room
that never sleeps costs about 30 times more.

| Shell | Rooms | Store |
|---|---|---|
| Cloudflare | One Durable Object per room, with hibernating WebSockets | The object's own storage |
| Self-hosted | One process holding every room in memory | A file |

Checked on 2026-11-24 with a prototype run locally: the Cloudflare shell can be Rust (`worker` 0.9.0).
Hibernating connections, data kept with a connection, the automatic ping answer, alarms and storage
all exist there. The prototype is 162 KB of WebAssembly. A room that was unloaded carried on with its
members when the next message came.

### The protocol stays quiet

Cloudflare bills every incoming message and every connection. So nothing is sent on a timer: no
application heartbeat and no position updates. A watching device works out the position from the clock.
Keep-alives are WebSocket pings, which Cloudflare answers without waking the room and without charge.
A browser cannot send those, so the web client sends a short text message, which may count as a request.
Cloudflare closes connections when it updates its runtime, so clients reconnect quietly, after a random
short wait.

### Price to run

One paired user costs about 20 connections and a few hundred small messages a day. The path the owner
agreed to:

| Stage | Runs on | Cost |
|---|---|---|
| Start | Cloudflare free plan | $0, up to about 3,000 daily users by estimate |
| First day the free limit is hit | Cloudflare paid plan | $5 to $10 a month |
| Bill above about $20 a month | A rented server with the self-hosted shell | About $5 a month |

At the free limit the server refuses everything until midnight UTC. Clients treat a refusal as a normal
state: devices on one network keep working.

**To test:** whether Cloudflare counts 20 incoming messages as one request against the free daily limit
too. Its documents say so only for billing. If it does not, the free plan covers a few hundred devices.

## Pairing

The code holds nothing secret, so a photographed or forwarded code is useless.

1. The new device makes a one-time lock and key, picks a random meeting room and waits there. It shows a
   code: the server's address, the room's name, its lock and its name.
2. A device that has the user's profiles scans the code, or takes it as a pasted link.
3. That device asks: "Give 'MacBook' your sign-ins for 3 profiles?"
4. On **Pair**, it sends the group key, sealed with the new device's lock, into the meeting room. On one
   network it sends it directly.
5. The new device opens it, leaves the meeting room and joins the group room.
6. Every other device of the group shows "New device paired: MacBook".

The first pairing creates the group key on the device that has the profiles. It also asks whether to keep
an encrypted copy on the server, and says that nobody running the server can read it.

A code that nobody scanned within about five minutes is dead. The code carries the server's address, so
all devices of a group use one server.

The code reaches the other device in one of three ways:

| Way | How | Check |
|---|---|---|
| Camera | The device with profiles scans the QR code | None needed |
| Link | The user pastes the link, for devices without a camera | Both screens show the same three words |
| Local network | The pairing screen lists waiting devices that have the same server account | Both screens show the same three words |

The three words come from the new device's lock. They show that a link or an announcement was not swapped
on the way. A device announces itself only while its own pairing screen is open, and the list shows only
inside the pairing screen. So no device is ever prompted by another: nothing can be sent to a device
whose user did not open pairing.

When the new device already belongs to a group, it leaves that group. The confirmation says so:
"MacBook will stop syncing with its 1 other device."

Settings list the group's devices with when each was last seen. With one shared key a device names
itself, so the list informs and does not prove.

A backup does not hold the pairing. After a restore, nori says that pairing was not restored and offers
**Pair a device**.

**Reset pairing** makes a new group key on that device. A confirmation says that the other devices keep
their data and only stop syncing with this one. The user then pairs again the devices they still have.
A lost device keeps the sign-ins it had: the screen says to change those passwords.

The research suggests a key per device, so that a lost device could be removed without pairing the rest
again. The owner chose one shared key for its simplicity.

## Profile sync

Paired devices hold the same profiles. Each device keeps its own choice of which profile is open.

A synced profile has an identity of its own, apart from the profile's id on each device. Downloads,
history and the saved queue stay with the device's own profile.

- **What travels:** a profile's kind, name, address, sign-in, advanced options and sources, and which
  plugins it needs. A plugin a profile needs is installed on the receiving device.
- **Sync this profile, off:** the profile lives on that device only. The other devices drop it.
- **A profile a device cannot play** is listed grayed as Unavailable (a plugin's profile in a browser).
  Its sign-in is not sent there.

### Merging

- Added on two devices while apart: both are kept.
- The same profile changed on both: the newer change wins, for the whole profile.
- Removed: a removal marker stays, so the profile does not come back from another device.
- Removed on one device and changed later on another: the later action wins.
- The first sync after pairing is a union of both lists. Two profiles are one when their kind, server
  address and user match, or their plugin's account identifier. When nori cannot tell (a home address on
  one device, a public one on the other), it shows both and offers **Merge**.
- A device keeps, unchanged, what it does not understand in a profile from a newer version of nori. When
  two devices cannot talk, each says which one needs an update.

Order comes from a counter kept with each profile, not from a device's clock alone. Removing a
profile asks first, and says that it is removed from every paired device.

### Sign-ins that must not be copied

A provider plugin states how its sign-in behaves: it can be copied, or each device must sign in itself.
YouTube Music is the second kind. Its sign-in is a browser session that YouTube keeps replacing, and one
session used from several devices looks like a stolen one.

Such a profile syncs without the sign-in. The other device plays it without an account and shows **Sign
in**. Only the same account is accepted there. The plugin gives a stable account identifier for that
check, not the display name.

**To test:** whether the SoundCloud plugin's sign-in survives being used from two devices. The
official SoundCloud sign-in does not, but the plugin uses the website's token.

### The kept copy

With no kept copy, a device catches up only when another device of the group runs nori at the same
time. With one, it catches up whenever it connects. The copy is the group's latest profiles, encrypted
with the group key. The choice belongs to the whole group and syncs like a profile. Turning it off
deletes the copy.

## Remote control

A device's player shows the group's other devices and plays on one of them, as today. The protocol is
the existing one (crates/remote/src/wire.rs), sealed with the group key. The group room carries it, or
two devices on one network exchange it directly.

- **Reachable:** while nori is open or playing. A phone paused in the background lets go after 15
  minutes and connects again when nori is opened or plays. A computer stays reachable while nori runs.
- **Keep-alive:** on a phone, it rides on the wake-ups the audio already causes. It never wakes the
  phone by itself.
- **A profile without its sign-in** on the receiving device plays without an account.
- **A profile that does not sync** is not offered for remote control. Local files profiles are among
  them.

## Jams

### Starting and joining

**Start a jam** opens a sheet with the jam's settings:

| Setting | Default | Shown |
|---|---|---|
| **Who can join**: **Anyone with the link** or **Ask me first** | Anyone with the link | Always |
| **Let guests listen along** | On | Always |
| **Allow remote songs** | Off | When the server is octo-fiesta |

The sheet says that guests stream from the host's server, at original quality. Each setting stays
changeable during the jam. The invite is a link and a QR code to a page the sync server
serves. The jam's key is after `#`, which a browser never sends. On the public server the link opens
nori directly. On a custom one the page has an **Open in nori** button.

### The host stays connected

Hosting a jam keeps the host's device connected, also while it is paused in the background. When the
host is gone for a few minutes, the jam ends for everyone, and its share expires.

### A guest reads through the host

A guest's profile plays from the host's provider, as a LAN jam's guest does today (`HostedProvider`,
docs/research/jam-host-stream.md). Its searches and pages are requests the host's device answers. Over
the sync server they are jam messages. With **Allow remote songs** off, the host leaves provider songs
out of its answers.

### Where a guest's music comes from

| Host's source | The guest gets | Needs |
|---|---|---|
| Navidrome | A public address from the jam's share | Sharing on, the server reachable by the guest |
| A server where the guest has an account | The song with its own account | Nothing |
| A plugin whose addresses work anywhere | The address the host sends | Nothing |
| A plugin whose addresses work only for the device that asked | The song's id, looked up with the plugin | The plugin, no account |
| Another server without an account, local files | Nothing | Requests only |

A plugin states which of the two plugin rows it is. A guest installs a plugin only from the default
repo or one they added, never from an address the host names. Joining says that the plugin is installed
for the jam, and it stays installed afterwards. A guest who cannot get the music still joins, sees
the queue and asks for songs. A sheet says why they cannot listen when they try.

Checked on 2026-11-24 from two networks in one country:

| Plugin | An address fetched elsewhere | So |
|---|---|---|
| YouTube Music | Works for 6 hours | The host sends the address, and the guest's plugin is the fallback |
| SoundCloud | Dies after about 5 minutes | The guest looks the song up with the plugin |

A YouTube address names the network address of the device that asked for it, but YouTube did not enforce
that. It could start to without warning. Another country is untested.

### A jam on Navidrome

1. The host's app makes a playlist named `nori-jam-<hash>` and one share of it. nori hides playlists
   with that name.
2. Each queued song is added to the playlist. The share follows the playlist.
3. The host sends each song's public address and cover address to the guests.
4. The share expires after a few hours. The host extends it while the jam runs.
5. A provider song being prepared shows as preparing in the queue, for the host and the guests.
6. When the jam ends, the host's app deletes the share and the playlist. A playlist left behind by an
   app that died is deleted when nori next starts.

Details the research found:

- A share names the address Navidrome believes it has, which can be an internal one. The host builds
  each address from its own profile's address and the share's path.
- `getAlbumInfo2` gives public cover addresses. Guests fetch covers from the server. For a song with no
  such address, the host sends a small picture through the jam.
- Guests get the original file. The standard `createShare` call cannot ask for another quality.
- Navidrome before 0.63.0 has sharing off unless the admin turned it on. The host is told to turn it on
  to host jams.
- Deleting a share lets a running stream finish and refuses new requests.

### Playing a shared song

A guest plays a shared song as nori plays any stream, starting where it needs to. When the host's server
is octo-fiesta, the guest fetches each song whole from the start. Asked for the middle of such a file,
octo-fiesta sends the middle labeled as the whole. That was checked on the original and on the fork.

### A guest in a browser

A browser lets the web client fetch a shared song and its cover from Navidrome. It hides the header
that says which part of the file came back. The web client needs that header for any start past the
beginning, which joining mid-song is. So for a share address the web client takes the start to be the
byte it asked for. Navidrome exposing that header would be a one-line change to offer upstream.

### Provider songs

When the host accepts a provider song, the app adds it to the jam playlist with its `ext-` id. The
original octo-fiesta then downloads it into the library, in either of its storage modes, waits for the
scan and adds the library song. The song is ready before its turn. When that fails, guests skip the
song.

Checked on 2026-11-24 with real YouTube Music songs, on the original octo-fiesta (`fe01a58`) and on the
fork, with Navidrome 0.64.2. A provider server for the test is at `/Users/pilif/projects/octo-ytmusic`.

| What | Result |
|---|---|
| Accepts that worked | 26 of 26 |
| Time for one accept | 2 to 5 s, or about 30 s within 30 s of the one before |
| The share after an accept | Lists the song at once |
| A guest's stream from Navidrome | Whole file, and parts of it, without an account |
| The end of the jam | A running stream finishes, a new request gets 404 |

Rules the check gives:

- The call that adds a provider song can take long. The app allows 150 s for it. octo-fiesta starts at
  most one library scan per 30 s, and the call waits for the scan.
- Error 70 means the song arrived but the scan was missed. That happens when another scan is running
  inside those 30 s. The app tries once more after about 35 s. A second failure makes guests skip the song.
- Error 10 means the provider failed. Guests skip the song, and the host is told.
- octo-fiesta's library scan must be on, which is its default.
- With octo-fiesta set to download whole albums, accepting one song downloads its album.

### The clock

The sync server answers time requests. Each listener measures its distance to that clock when it joins
and again every few minutes.

## Moving from the relay

One release ships everything in this note, profile sync included, and removes the relay from nori. After the update the device list says
once that devices pair by code. On an octo-fiesta server it adds that remote control and jams do not
use octo-fiesta's relay. Old and new versions do not control each other. The fork keeps its relay for a while, for people
who update the server before the app.

## To test before building

| Check | Decides |
|---|---|
| The SoundCloud sign-in on two devices | Whether it can be copied |
| The 20-to-1 rule on Cloudflare's free daily limit | The free plan's size |
| Cloudflare's idle time before it closes a connection | The keep-alive interval |

Each needs an account. The steps for each are in `checks.md` (see Research).

## Open points

- Android 17 asks for a permission before an app reaches the local network. Ask for it only when the
  user paired a device, and use the sync server when it is refused.
- Which of a plugin's own settings belong to a profile and sync with it.
- Two changes to offer the original octo-fiesta, neither required. One answers requests for part of a
  file correctly on forwarded addresses. The other resolves `ext-` ids in `createShare`.

## Research

Three reports from 2026-11-24 are kept outside the repository, at
`/Users/pilif/tmp/nori-sync-research/`.
`checks.md` holds the checks run for this note and the steps for the three that need an account.
`sync-server-research.md` compares the design with Signal, WhatsApp, Spotify Connect, Syncthing and
others, with sources. `octo-fiesta-remote-songs.md` traces provider songs in octo-fiesta and records the
share-link checks run against Navidrome 0.64.2.
