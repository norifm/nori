# A jam's share on Navidrome

A jam's guests on a Navidrome profile get the music straight from the host's server, with no account. The
host's app keeps one hidden playlist and one public share of it for the jam, and tells the guests each
song's public address. This page is for the people who wire jams to it. The share is in
`crates/core/src/jam_share.rs`, and the Subsonic calls it makes are in
`crates/core/src/providers/subsonic/sharing.rs`.

## The moments of a jam

The jam drives one `JamShare` from the host's `Client`. Each moment is one call:

| Moment | Call | What the server gets |
|---|---|---|
| The profile opens | `Client::clean_jam_leftovers` | The share and playlist of a jam this profile never ended, deleted |
| The jam starts | `Client::start_jam_share` | A ping, a playlist named `nori-jam-<8 hex>`, and one share of it |
| A song joins the queue | `JamShare::add` | The song added to the end of the playlist |
| A song leaves the queue | `JamShare::remove` | One copy of the song taken out of the playlist |
| The jam runs | None: a timer | The share's expiry extended |
| The jam ends | `JamShare::end` | The share and the playlist deleted |

The jam numbers its queue entries, and `add` and `remove` take that number. What becomes of each entry
comes back to the jam's `JamShareHeard` as a `JamShareEvent`:

- `Song { entry, state }`: the entry is `Preparing`, `Ready` with its address and cover address, or
  `Skipped` with a `Skip` reason. The share tells an entry again only when its state changed.
- `Fetch { how }`: how guests fetch the songs, told once, from the first song shared. See
  [How a guest fetches](#how-a-guest-fetches).

The core says no words. The jam's clients word the states and the reasons.

## Starting

`start_jam_share` pings the server first. If the answer's `type` is not `navidrome`, the start returns
`ShareError::NotNavidrome` and makes nothing. The ping passes through octo-fiesta to the Navidrome
behind it, so a profile on octo-fiesta counts as Navidrome.

A Navidrome with sharing off answers the share calls with HTTP 501. The start then deletes its playlist
and returns `ShareError::Off`. Its `off_by_default` is true when the server is older than 0.63.0, where
sharing is off unless the admin turned it on. The jam can then tell the host what to do.

The share lasts 4 hours. While the jam runs, the share extends it to 4 hours from then, every hour. A
profile that never hosts a jam pays nothing: no timer runs, and no request goes out.

## Addresses and covers

A share names the address that Navidrome believes it has. Behind octo-fiesta, that is the address
octo-fiesta uses to reach Navidrome, which guests cannot reach. So the share keeps only the path from
`/share/` on, and puts it after the profile's first address (`Client::public_address`).

After each change to the playlist, the share reads the playlist (`getPlaylist`) and the share's public
`m3u` list. Both list the songs in one order, so the share pairs them by position. Songs that
octo-fiesta adds meanwhile come only at the end. One lock keeps the share's own edits and reads in order.

A song's address holds the share's expiry as it was when the address was made. So after each extension
the share tells every entry's new address. An address that a guest holds stays good for three more hours.

A cover address comes from `getAlbumInfo2`, asked once per album and put after the profile's address in
the same way. An album with no public cover gives `cover: None`, and the jam sends a small picture instead.

## Provider songs

`add` with a provider's song adds its `ext-` id to the playlist. In answer, octo-fiesta
downloads the song into the library, waits for the library scan, and adds the library song. The entry is
`Preparing` meanwhile. Afterwards `getSong` on the `ext-` id gives the library song, which is shared like
any other.

Only add a provider's song that the host accepted: adding it makes the server download it.

| octo-fiesta answers | The share does |
|---|---|
| OK | Shares the library song |
| Error 70: the song arrived, but no scan listed it | Adds it once more after 35 s, then skips it with `Skip::NotInLibrary` |
| Error 10: the provider failed | Skips it with `Skip::ProviderFailed` |
| No answer within 150 s, or anything else | Skips it with `Skip::Failed` |

The 35 s wait is for octo-fiesta's next scan. The 150 s cover the download and octo-fiesta's wait for the scan, which it starts at most once every
30 s. In a run with YouTube Music songs, an add took 2 to 4 s, or about 32 s inside the 30 s of another.

## Ending and leftovers

`end` stops the timer and every add still running, then deletes the share and the playlist. A stream
that a guest already runs finishes. Every new request for the share gets 404.

The profile keeps the jam's playlist and share ids in its database from the moment it makes them, and
forgets them once they are deleted. If the app dies, `clean_jam_leftovers` deletes them when the profile
next opens. It deletes only what this device made. Another device of the same account can be hosting a
jam with its own `nori-jam-` playlist at that moment.

## Hidden playlists

No list of playlists shows a jam's playlist. The Subsonic answer to `getPlaylists` leaves out every
playlist whose name starts with `nori-jam-` (`jam_share::is_jam_playlist`). Every client lists playlists
through `Read::PlaylistList`. So the playlists page, Home's shelves, the add-to-playlist picker and the
car leave it out, on every client.

## How a guest fetches

Through octo-fiesta, a request for part of a shared song gets that part as if it were the whole
file: status 200, and no `Content-Range`. Straight from Navidrome, it gets a correct 206. So the
host asks its own server: from the first song it shares, it asks for one byte (`Range: bytes=0-0`).

| Answer | `GuestFetch` |
|---|---|
| 206 | `FromAnyByte` |
| 200 with the whole file: a server that ignores ranges | `FromAnyByte` |
| 200 with one byte: a part labeled as the whole file | `WholeFromStart` |

nori has no other sign of octo-fiesta. Only octo-fiesta's fork announces itself, through its relay.

A guest that is told `WholeFromStart` answers true from `ByteSource::whole` for the host's addresses.
The engine then asks for such a song from its start only. It reads forward to where the song is wanted,
as it does with a server that ignores ranges. `Filling` does this for every client's loader, the web's too. The
fetch-ahead (`crates/engine/src/ahead.rs`) leaves such a song half way rather than ask for its rest, and
the player goes on with it. `Resolving` passes the question on to the byte source it wraps.

A browser hides `Content-Range` on an answer from another origin, which a share address is. The web
client reads each answer with `nori_engine::answer_span`. A 206 without a readable range starts at the
byte asked for, and the song's length is that byte plus `Content-Length`.

## Tests

`crates/core/src/jam_share/tests.rs` runs the share against an in-memory Navidrome, alone or behind
octo-fiesta, on a simulated clock (`nori_tasks::Native::simulated`). The engine's test
`a_song_opened_mid_way_plays_from_there_however_ranges_are_answered` plays a song opened mid-way from a
server that keeps ranges, ignores them, or labels a part as the whole file.
