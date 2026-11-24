# Pairing and profile sync

`nori-sync` (`crates/sync`) pairs a person's devices into a group and keeps their profiles the same on
each of them. The devices of a group share one key. They talk through rooms on a sync server, and they
seal every message with the key, so the server passes on what it cannot read. The crate does no I/O: a
client hands it the platform's WebSockets and gives back what they hear. A device that never paired, or
has sync off, loads nothing, holds no key, and runs no thread, socket or timer.

The plan behind it, with the owner's decisions, is `docs/research/sync-server.md`.

## The parts

| File | What it holds |
|---|---|
| `room.rs` | The server as the crate sees it: the `Relay` trait, what the server says (`Heard`), refusals |
| `wire.rs` | The protocol's frames, close codes and room addresses, version 1 |
| `link.rs` | A `Relay` to a real server over the platform's WebSockets (`Link`, `Sockets`) |
| `syncing.rs` | A device's life with its server, the API every client calls (`Syncing`, `SyncClient`) |
| `presence.rs` | When a device holds its connection, and how its keep-alives go |
| `hub.rs` | A sync server in memory, for tests (the `testing` feature) |
| `keys.rs` | The group key, the room name, sealing, a new device's one-time lock |
| `words.rs` | The three words both screens show while pairing |
| `pairing.rs` | The code, the new device's side (`Joining`), the other device's side (`Admitting`) |
| `group.rs` | The group as a device keeps it, and where it is kept |
| `session.rs` | A paired device in its group's room (`Session`) |
| `profile.rs` | A profile as it travels, and whose sign-in goes where |
| `book.rs` | The merge: sync identities, the order of changes, removal markers |

The core re-exports the crate as `nori_core::sync`. `Kinds::sync_here` gives a client its `Here`: the
profiles it plays, the kinds whose sign-in may be copied, and its installed plugins.

## The server contract

A `Relay` is one server. Every call returns at once, and the answers come back as `Heard`, in order. The
whole contract with the server is `PROTOCOL.md` in the server's project (`nori-sync`, its own
repository). `wire.rs` encodes it, and its tests pin the bytes.

- A room is named by 43 base64url characters. Whoever knows the name may join.
- A member gets a number when it joins, and hears who is there, who arrives and who leaves.
- A message goes to every other member or to one, and arrives unchanged with the sender's number. The
  server stores nothing and replays nothing.
- A meeting room holds two members and closes after about five minutes. A group room may hold one kept
  copy, with put, get and delete. The first put sets a write secret that later puts and deletes present.
- The server answers time requests, in the room they were asked in.
- Any request can be refused (`Refusal`): a limit, the daily wall, the protocol version, a wrong write
  secret, a server down, a bug. A refusal is a normal state: the session reports it and goes
  on.

## The connection

`Link` is the `Relay` for a real server. Each room is one WebSocket, which the platform opens, reads and
writes through `Sockets`. Android uses OkHttp, the web worker the browser's WebSocket, and the desktop,
the terminal and the iPod `nori-http`'s `WebSockets`.

- What the crate sends to a room before its welcome waits for the welcome. A time request is stamped
  when it leaves, so the server's clock is read from the way there and back, as NTP does.
- A connection that drops comes back after a random wait of 1 to 5 s. The wait doubles while connecting
  fails, up to 5 minutes. A welcome starts the waits again. Its members are told as having left.
- A refusal is a `RoomState::Waiting` with the time to try again. The daily wall waits until midnight
  UTC plus up to 30 minutes, too many rooms a minute, the rest the growing wait. A version the
  server does not speak, a frame this version cannot read, and `bad_request` stop the room until it is
  joined again.
- A meeting room's end (`room_closed`) is `Heard::Closed`.
- A keep-alive goes out after 50 s without traffic. The link only says when one is due.

`Syncing` runs a device's life with the link. It starts when the device is paired and sync is on, or a
pairing starts. It then makes its executor and one task, which sleeps until the next thing due: a
reconnect, a keep-alive, letting go. Sockets' frames are handled on the thread that reads them, under
one lock, in order. What sync brings is kept in the settings at once, so a second change never starts
from a list the client has not saved yet. `ProfilesSynced` tells the client which profiles went and
whether the one in use changed. Profile changes made here reach it through `Settings::watch_profiles`,
however the client saved them.

### When a device is connected

| Device | Connected | Keep-alive |
|---|---|---|
| A computer (desktop, terminal, web) | While nori runs | A timer of its own |
| A phone (Android, iPod), in the foreground | Yes | A timer of its own |
| A phone playing in the background | Yes | When the audio wakes the device (`Syncing::woke`) |
| A phone paused in the background | 15 minutes, or until the connection drops | None |

So nothing of sync's ticks while music plays with the screen off. The engine calls `woke` at each of its
wakes, which the audio already causes. It sends a ping only when one is due. A phone that let go
(`SyncState::Resting`) connects again when nori is opened or plays.

## The keys

| Purpose | Primitive | Crate |
|---|---|---|
| The group key | 256 random bits | `getrandom` |
| The group room's name | HKDF-SHA256 of the key | `hkdf`, `sha2` |
| What the group's devices say, and the kept copy | XChaCha20-Poly1305 under a key from HKDF | `chacha20poly1305` |
| The kept copy's write secret | HKDF-SHA256 of the key | `hkdf` |
| The group key sent to a new device | HPKE (RFC 9180): X25519, HKDF-SHA256, ChaCha20-Poly1305 | `hpke`, `x25519-dalek` |
| The three words | Argon2id of the lock, 19 MiB, 2 passes | `argon2` |

Each use of the key gets its own derived key. A seal opens only as what it was sealed as: a message does
not open as a kept copy. The HPKE context names the meeting room and the lock, so a sealed key opens
only in the meeting it was sent to.

The words come from the EFF's short word list 2.0 (`words.txt`, CC BY 3.0 US). No two of its 1,296 words
are within two letters' change of each other. Three words are 31 bits. Argon2id makes each guess cost a
fraction of a second, so nobody can search for a lock with the same words while a code lives. The words
are English on every client, so two screens in different languages still compare.

## Pairing

The code holds nothing secret: a photographed or forwarded code is useless.

1. The new device starts `Joining`. It makes a one-time lock, joins a random meeting room and asks the
   server's time. It then shows the code (`JoinStep::Code`) and its three words.
2. The device with the profiles reads the code with `Admitting::read`: scanned, pasted, or from a
   nearby device's announcement (`Came`). It joins the meeting room and asks the server's time there,
   and a code past its expiry is refused. It waits in the room while the user decides, so it knows
   whether the new device is still there.
3. It answers a `Confirmation` for the user. It names the new device, the profiles it receives, and the
   group it leaves. It holds the words unless the code came by camera, and asks about a kept copy at the
   first pairing.
4. On a yes, `admit` seals the key to the lock and sends it into the meeting room. It answers `Gone`
   when the new device left. At the first pairing, `admit_first` makes the group first.
5. The new device opens it (`JoinStep::Paired`). `Welcomed::join` says goodbye to its old group, if any.
   It keeps the device's profile sync and stores the new group.
6. The new device joins the group room, and its first hello makes the others report `NewDevice`.

The code is one link, also the QR code's text:

```text
nori://pair#v=1&s=<server>&r=<meeting room>&l=<lock>&n=<name>&p=<s|a>&e=<expiry>[&g=<devices>&t=<tag>]
```

`s` and `n` are base64url text. `p` says whether the device plays plugins' profiles. `e` is the server's
time in Unix ms when the code dies, five minutes after it was made. `g` and `t` come from a device in a
group: its other devices, and a tag made from that group's key and the lock. A device of the same group
reads the tag and answers `AlreadyPaired`. The tag tells nothing about the key.

Every refusal is a `PairingRefused`. The new device refuses these keys, and goes on waiting after the
first two:

- `Tampered`: changed on the way, or sealed for another meeting.
- `Spent`: a key after the one that paired it.
- `Expired`: a key after five minutes by its own clock.

The other device refuses a code that does not read (`NotACode`), one from a newer nori (`Newer`) and a
dead one (`Expired`). It answers `Gone` when the new device left before the key was sent.

## The group on a device

`Kept` holds the group (`Group`) and the profile sync (`Book`). It is JSON in the app database's
`app_kv` table under `syncGroup`, beside the saved profiles and their sign-ins. It has the same
protection as they do: the app's own storage on each platform.

A backup never holds it. `nori-backup` notes only whether the device was paired. A restore on a device
that is not paired answers `pairing_not_restored`, and the client offers **Pair a device**.

The group holds the key, the server, this device's id and name, and the kept copy's choice. It also
lists the devices it saw, with when each was last seen. A device names itself, so the list informs and does not prove.

`Session::reset` is Reset pairing: the device says goodbye, makes a new key and starts a group of its
own. Its profiles and their sync stay. `reset_notice` gives the confirmation its data: how many devices
stop syncing with this one, and which profiles' sign-ins they hold.

## Profile sync

A synced profile has a sync identity, the same on every device. Each device keeps its own id for the
profile, so its downloads, history and queue stay with it.

### What travels

A profile travels as the profile list stores it (`server_json` in `nori-settings`). What stays on each
device is left out: its id, its client certificate and Sync this profile. Its sign-in (`password`, `apiKey`)
travels only to a device that plays the profile, and only when the provider lets it go. A profile also
names the plugin it needs and, for a sign-in that is not copied, the account it is signed in to.

A device keeps what a newer nori added, unchanged: unknown keys in the profile and beside it. Settings,
local data and downloads never travel. Jam guests' profiles never sync.

### Order and merge

Each change gets a `Stamp`. Its counter moves past the highest one the device has seen. The wall clock and
the device's id only settle a tie. The rules:

- A profile added on two devices while apart is kept on both.
- The same profile changed on two devices: the newer change wins, for the whole profile.
- A removed profile leaves a marker, so it does not come back from another device.
- A removal and a change on another device: the later action wins.
- Two profiles are one when their kind, address and user match, or their account. Their sync identity
  becomes the lower of the two, on every device. So the first sync after pairing is a union.
- Two profiles with one kind and user at two addresses may be one. That is when one address is reached
  only at home, or is the other's second address. `Session::offers` lists them. `merge` makes one of them
  on every device, and each device keeps the profile its user added there. `keep_apart` stops the offer.

When a device sees a hello, it sends that device everything it holds. A device that hears a state answers
with what it holds newer, so two devices agree after one exchange.

### Sync this profile

`SavedServer::sync_off` keeps a profile on its device only. Before a client turns it off, `synced`
says whether to ask about the other devices, and `Session::sync_off` takes the answer. With the answer
yes, the profile is removed there. Otherwise the others keep it, and what they say about it stops
reaching this device. `synced` also tells the removal confirmation that the profile goes from every
paired device.

### Sign-ins that are not copied

A provider says how its sign-in behaves in `ProviderInfo::sign_in_copy`. Subsonic servers are `Copied`.
A plugin's provider says it in its manifest, and is `EachDevice` when it says nothing. Such a profile
syncs without its sign-in, and each device keeps its own. After a sign-in, the client passes
`Client::account_id` to `Session::signed_in`, which checks it against the account the profile syncs
with:

| `SignInCheck` | When |
|---|---|
| `Accepted` | The same account, or the first one named, which becomes the profile's |
| `OtherAccount` | Another account than the profile's |
| `Unchecked` | The provider names no account, such as a plugin built before `account` |

An unchecked sign-in stays, and the profile's account stays unknown.

### A device that cannot play a profile

A browser and the iPod play servers only (`Plays::Servers`). They list a plugin's profile, which waits
as `ProfileWait::NoPlugins`. Its sign-in is never sent to them, and they strip it from the kept copy.

### The kept copy

When the group chose to keep one, the copy is the group's whole state, sealed with the key. A device
writes it after its own changes and reads it when it joins the room. A copy older than what the device
holds changes nothing, and the device writes back what is newer. Turning the choice off deletes the copy.
The choice syncs like a profile, and the newer choice wins.

## What a client does

The client owns the sockets, the screens and the words. It makes one `Syncing` when the app starts,
with its `Sockets`, its `SyncClient` and its `Reach`, and calls these:

| Moment | Call |
|---|---|
| A socket heard a frame, or ended | `frame`, `ended` |
| The `sync` or `syncServer` setting changed | `settings_changed` |
| nori came on screen or left it, started or stopped playing | `foreground`, `playing` |
| The audio woke the device (Android, iPod) | `woke` |
| A screen shows where sync stands | `state` (`SyncState`) |
| Pairing on the new device | `pair_show`, then `pairing` shows the code |
| Pairing on the other device | `pair_take`, then `pairing` asks to confirm, `pair_confirm` |
| Stopping a pairing | `pair_cancel` |
| The device list | `devices` |
| The kept copy switch | `keeps_copy`, `keep_copy` |
| Reset pairing | `reset_notice`, then `reset` |
| Merge offers | `offers`, `merge`, `keep_apart` |
| Sync this profile, sign-ins | `synced`, `sync_off`, `signed_in` |

`SyncClient` hears what to do:

- `profiles`: sync changed the saved profiles. Forget a removed profile's rows, and open the profile in
  use again when it changed.
- `install`: plugins a profile needs.
- `event`: something to word.
- `changed`: `state` or `pairing` changed.

These calls never come while sync holds its lock, so they may call it back.

A paired device uses its group's server. The `syncServer` setting names the server a new code is made
on: nori's own unless another is named. With `sync` off, nothing connects and pairing answers
`SyncOff`.

## Tests

`cargo test -j4 -p nori-sync` runs two or three devices through the in-memory hub. The tests pair them,
sync them, and move the server's clock by hand, with no network and no sleeping. `syncing/tests.rs` runs
whole devices, each on a simulated executor, through a stand-in server that speaks the protocol over
the hub. They pair, sync both ways, drop and come back, and meet the daily wall. A phone playing with the
screen off has no timer waiting. `hub.rs` is also open
to other crates' tests through the `testing` feature, as `nori-backup` uses it.
