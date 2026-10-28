# LAN jams served by the host

A LAN jam's guest today plays the host's songs through its own copy of the host's provider: a YouTube
Music host's guest needs the YouTube Music plugin (`JamJoin::NeedsPlugin`). This plan makes the host
serve what a guest needs, so any native device joins any LAN jam with no plugin and no account.

## Where things stand

- LAN jams are the jams of profiles without a Subsonic server: `jam_open` opens one only when
  `Core::serverless()` (crates/core/src/remote.rs). In practice a LAN jam's host is a plugin provider's
  profile. Subsonic hosts jam through octo-fiesta's relay; nothing here changes those.
- The LAN door (crates/remote/src/lan/door.rs) is hand-written HTTP/1.1: a thread per connection, signed
  requests (HMAC over method, target and body), JSON answers held in memory, no ranges, 1 MiB bodies, 32
  connections and 32 members.
- A guest reads (search, pages) and streams through its own provider. It finds what the host plays in
  the host's published `DeviceState.entries` and follows it with the engine's follow mode (`Lead`, slips,
  the clock exchange through the door); that part needs no change.
- A guest who doesn't listen along is already control-only (`Listening::Watching`).

## The plan: the host answers for its provider

The guest profile of a LAN jam gets a provider of its own, **the host's**: a `Provider` whose reads and
stream lookups are door requests that the host carries out with its own provider and answers.

- `noriRemote.read`: a `Read` (search, album, artist, playlist, song), answered with the host's `Page`.
  The host applies its jam rules first (what a guest may browse).
- `noriRemote.source?id=&use=`: the URL a song streams from, its length and the part size (a plugin's
  `source`, the host's `ResolveStream`), for the guest to fetch itself.
- Covers are addresses already (`{size}` templates) and need nothing.

Requests then carry songs whose ids the host knows, from the host's own provider, so `Op::Request` and
the host's `Decide` work as they do.

### Why the guest fetches the audio itself

The alternative is the host sending the bytes. A song is about 1.2 MB a minute (YouTube's Opus, 160
kbit/s): ten listening guests are 12 MB a minute, 720 MB an hour, through the host phone's radio and
battery. Fetching from the URL the host resolved costs the host one small request per song.

It works because a plugin's stream URL is fetched as it is (`stream-source` has no headers), and
YouTube's are bound to the public address that asked: the host's and its guests' on one home network.
A guest whose fetch is refused (another public address: a VPN, a guest on mobile data) gets a plain
"can't listen here" and stays in the jam control-only. The host serving bytes is a later fallback,
only if that turns out common.

What the guest fetches is the same file the host plays (the same URL), which listen-along's sync wants.

## What changes

1. `crates/remote` door: the two endpoints, signed by the member's key, through the jam handler; answers
   stay JSON.
2. `crates/core`: a `HostedProvider` for LAN guest profiles (kind `jam-host`, never saved past the jam),
   its reads as door requests, its `ResolveStream` as `noriRemote.source`. The host side answers them
   through its provider, under the jam's guest rules.
3. `jam_join` stops returning `NeedsPlugin` for LAN jams; the plugin offer stays for a guest whose jam
   needs a plugin it can't otherwise have (none, once this is in).
4. Clients: nothing new to draw. The iPod and the web become able to listen to a plugin host's jam
   (the web still can't reach a LAN door: browsers can't sign into a LAN address over plain HTTP from
   an HTTPS page; it stays as it is).

## Costs and limits

- Host: one door request per guest search or page, and one `source` per song a guest plays.
- Guests' searches run on the host's plugin instance, one call at a time: ten guests typing at once
  queue behind each other. The plugin worker skips calls whose callers gave up (a search typed over).
- 32 connections across members and controllers: a poll each; no long bodies, so enough.

## Tests

- Rust: a LAN jam (host/tests/lan_jam.rs) whose guest has no plugin searches, requests a song the host
  accepts, and resolves its source through the door; the fixture plugin's provider answers on the host.
- Device: the jam section with a guest on the desktop (the e2e peer) and no plugin.

## Open questions

- Whether YouTube refuses a URL fetched by another device behind the same public address (it shouldn't:
  the address is what it checks). To try before building, with two devices on one network.
- Whether other plugins' sources are as portable; a plugin could say its sources are only its own
  (`source` answering "only here") and the host would then serve bytes or refuse listening.
