# Providers: what is Subsonic's and what every source shares

Internal note (not published). The owner's decision: Subsonic is a provider like YouTube Music, and
every provider gets the same library behaviour from the core, so providers cannot drift apart. The
YouTube bugs that led here (hearts not reaching lists, Home reshuffling, missing lengths) all came
from YouTube re-implementing what Subsonic got from the core's Subsonic path.

## Map: Subsonic outside `providers/` before the split (master at abe160e2)

| where | what | kind |
|---|---|---|
| `core/src/cache_policy.rs` | `spec()`: each `Read` as a Subsonic endpoint and params, its freshness window and parser; `key()` endpoint keys; `parse()` of `subsonic-response` into `Page` (star overlay inside the parser); `NOT_FOUND` = 70; `subsonic_read/stored/fetch`; `nothing()` = parse of an empty Subsonic answer; `kept_stored/kept_fetch/keep` for in-memory providers, `marked_favourites` | reads, cache, overlays |
| `core/src/client.rs` | `ping`, `login_check`, `scoped` (`musicFolderId`), `get`/`fetch`/`send` (second address retry), `choose_address`, `flush_pending` (offline queue), `sync_library` (`search3` walk), `subsonic_write` (queue + `request()` stale prefixes) | protocol, writes |
| `core/src/lib.rs` | the `subsonic-response` wire structs and `parse()`; `ServerConfig::server()`; `Core::configure` (Subsonic only); `url`, `url_prefix`, `stream_url`, `use_address`, `parse_status`, `parse_search`, `ingest_search`, `heard_from_server` (`type: navidrome`), `pending_*` | protocol |
| `net/src/requests.rs` | `request()`: each `Write` as an endpoint, params and cache-key prefixes it makes stale (`STAR_STALE`, `PLAYLIST_STALE`); `FOLDERED` | writes, invalidation |
| `library/src/browse.rs` | `home_refresh_drops()` = `getAlbumList2`, `getPlaylists`, `getStarred2`, `homeFeed` (key prefixes, sent back by Kotlin to `drop_cached`) | invalidation |
| `core/src/covers.rs` | `download_cover_urls` builds `getCoverArt` URLs for every profile | covers |
| `core/src/remote.rs` | a remote mark evicts `request(Star)`'s stale prefixes; relay `noriRemote.*` on the server, `subsonic-response` refusals; `server.url` signing | invalidation; account service |
| `core/src/history.rs` | Navidrome's own API (`/auth/login`, `/api/scrobble`) with `native_login` | account service |
| `core/src/stream.rs` | the second address's bitrate cap | address (Subsonic only has two) |
| `core/src/car.rs`, `autofill.rs`, `actions.rs`, `library.rs`, `race.rs` | reads only through `Read`; Subsonic shows only in tests (fixtures, cache keys like `getAlbum&id=a`, asked URLs) | tests |

What YouTube had on its own (`providers/youtube.rs`): `read_stored`/`read_fetch` overrides for Home (kept
for the session, a refresh merged into what was shown, `State::home`), `marked_favourites` called
inside its `StarredItems` read, and writes that invalidated nothing.

## The harness

`crates/core/src/providers/harness.rs` (Subsonic, over a fake server answering by endpoint) and
`a_youtube_session` in `providers/youtube.rs` (over fixtures answered by InnerTube request) run one
scripted session each: every kept and always-asked read, stored, fresh, aged, changed; hearts online,
offline and refused; every write kind and what it leaves kept; Home's refresh; a page gone; provider
items; streams, downloads and covers; the library walk. Each step's pages (short form plus a hash of the
whole `Page`), freshness and requests go into a transcript compared with `crates/core/tests/golden/`
(`NORI_BLESS=1` records it). The transcripts were recorded from master first; each fix below changed
them in its own commit, with its own test; the refactor itself changed the Subsonic transcript not at
all and the YouTube one by one line (the last fix below).

## Old behaviour the harness found (each fixed, with its own test)

| old bug | test |
|---|---|
| `download_cover_urls` built Subsonic `getCoverArt` URLs for a YouTube profile's downloads | `downloads_use_the_provider` |
| a heart did not show in a server's favourites until the server listed it (YouTube's did) | `a_heart_shows_in_the_favourites_before_the_server_lists_it` |
| a write dropped the pages it changed, so a heart or playlist edit made offline left favourites and album pages unreadable offline; a YouTube heart changed nothing kept | `an_offline_heart_keeps_its_pages_readable`, `a_heart_asks_the_favourites_anew` |
| a kept YouTube page asked anew was shown again when unchanged (no digest) | `an_unchanged_page_is_shown_once`, `favourites_listed_as_before_a_heart_show_once` |
| Home stayed put for the session only on YouTube; another provider's reshuffled every two minutes | `home_feed_is_kept_for_the_session_and_merged_on_refresh` |
| a Subsonic profile asked for `homeFeed`, which no server has | `a_server_is_not_asked_for_a_feed` |
| a heart showed on no album, artist or playlist page until it was read anew (never on YouTube's); a song without a length stayed without one though its record had it | `a_heart_shows_on_every_page`, `a_song_without_a_length_takes_its_known_one` |

## The split

- `cache_policy.rs`: the page store every provider shares. Keys are nori's own (`page:album/<id>`,
  `page:albums/<sort>/<size>/<offset>`...), with the provider's `scope` appended (Subsonic: its music
  folders), and a freshness window per `Read`. An `Answer::Raw` is kept on disk and parsed again on
  every read (Subsonic); an `Answer::Page` is kept in memory for the session (YouTube). On every read
  the session's marks lay over every page (hearts on every item, favourites gaining hearted songs whose
  record is known and losing unhearted items) and songs without a length take their known record's.
  A write makes the pages of its kind stale (`stale`). Home is kept for the session; a refresh merges
  the new feed into what was shown. Old Subsonic-keyed rows are deleted when the database opens.
- `providers/subsonic/`: `reads.rs` each `Read` as an endpoint, `wire.rs` the answers and the pages
  they make, `server.rs` signing, addresses, folder scope, writes and their offline queue, login check,
  the library walk.
- `providers/youtube.rs`: InnerTube's answers as `Page`s, likes, streams, sign-in; no caching, marks or
  merging of its own.

Left outside `providers/`: the account services that need the server itself (remote relay
`noriRemote.*` and its `subsonic-response` refusals in remote.rs, Navidrome's history in history.rs),
behind `Provider::subsonic_api` and signing with `Core::server`; Subsonic answers as fixtures in the
tests of shared code (car, autofill, actions, library, race).
