# Pairing and profile sync

`nori-sync` (`crates/sync`) pairs a person's devices into a group and keeps their profiles the same on
each of them. The devices of a group share one key. They talk through rooms on a sync server, and they
seal every message with the key, so the server passes on what it cannot read. The crate does no I/O. A
client hands it a `Relay` to one server and gives back what the server said. A device that never paired
loads nothing, holds no key and does no work.

The plan behind it, with the owner's decisions, is `docs/research/sync-server.md`.

## The parts

| File | What it holds |
|---|---|
| `room.rs` | The server as the crate sees it: the `Relay` trait, what the server says (`Heard`), refusals |
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

A `Relay` is one connection to one server. Every call returns at once, and the answers come back as
`Heard`, in order. The transport owns the wire encoding.

- A room is named by 43 base64url characters. Whoever knows the name may join.
- A member gets a number when it joins, and hears who is there, who arrives and who leaves.
- A message goes to every other member or to one, and arrives unchanged with the sender's number. The
  server stores nothing and replays nothing.
- A meeting room holds two members and closes after about five minutes. A group room may hold one kept
  copy, with put, get and delete. The first put sets a write secret that later puts and deletes present.
- The server answers time requests.
- Any request can be refused (`Refusal`): a limit, the daily wall, the protocol version, a wrong write
  secret. A refusal is a normal state: the session reports it and goes on.

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
   nearby device's announcement (`Came`). It asks the code's server its time, and a code past its
   expiry is refused.
3. It answers a `Confirmation` for the user. It names the new device, the profiles it receives, and the
   group it leaves. It holds the words unless the code came by camera, and asks about a kept copy at the
   first pairing.
4. On a yes, `admit` seals the key to the lock and sends it into the meeting room. At the first pairing,
   `admit_first` makes the group first.
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
A plugin's provider is `EachDevice` until the plugin interface lets it say otherwise. Such a profile
syncs without its sign-in, and each device keeps its own. `Session::signed_in` checks a sign-in here
against the account the profile syncs with, and refuses another account (`OtherAccount`). The first known
account becomes the profile's.

### A device that cannot play a profile

A browser and the iPod play servers only (`Plays::Servers`). They list a plugin's profile, which waits
as `ProfileWait::NoPlugins`. Its sign-in is never sent to them, and they strip it from the kept copy.

### The kept copy

When the group chose to keep one, the copy is the group's whole state, sealed with the key. A device
writes it after its own changes and reads it when it joins the room. A copy older than what the device
holds changes nothing, and the device writes back what is newer. Turning the choice off deletes the copy.
The choice syncs like a profile, and the newer choice wins.

## What a client does

The client owns the connection, the screens and the words. It calls these:

| Moment | Call |
|---|---|
| The app starts | `Session::open`, then a `Relay` to `server()` and `connect` |
| The server says something | `Session::heard`, then keep `Outcome::servers`, install `Outcome::install`, word `Outcome::events` |
| The user changed profiles | `Session::changed` |
| Pairing on the new device | `Joining::start`, `Joining::heard`, `Welcomed::join` |
| Pairing on the other device | `Admitting::read`, `check`, `heard`, `admit` or `admit_first` |
| The device list | `Session::devices` |
| The kept copy switch | `Session::keep_copy` |
| Reset pairing | `Session::reset_notice`, then `Session::reset` |
| Merge offers | `Session::offers`, `merge`, `keep_apart` |

`sync_server` gives the server from the `sync` and `syncServer` settings: nori's own unless another is
named, and none when sync is off.

## Tests

`cargo test -j4 -p nori-sync` runs two or three devices through the in-memory hub. The tests pair them,
sync them, and move the server's clock by hand, with no network and no sleeping. `hub.rs` is also open
to other crates' tests through the `testing` feature, as `nori-backup` uses it.
