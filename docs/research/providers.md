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

## Old behaviour the harness found (each now a bug fixed, with its own test)

- A heart, sent or queued offline, did not show in a Subsonic profile's favourites until the server
  listed it; YouTube's did (`marked_favourites`). Now every provider lays the session's marks over its
  favourites.
- A YouTube heart left the kept favourites, album, artist and playlist pages as they were; Subsonic's
  dropped them. Now each write kind drops the same pages for every provider.
- A kept YouTube page read anew was shown again even when nothing changed (no digest); now an
  in-memory page has a digest too.
- `download_cover_urls` built Subsonic `getCoverArt` URLs for a YouTube profile's downloads; now it
  asks the provider.
- A Subsonic profile asked the server for `homeFeed`, an endpoint no server has; now a source without
  a feed answers the empty page without a request.
- Home stayed for the session only on YouTube; a provider without that override reshuffled it every
  two minutes. Home is now session-stable for every provider.

## The split

- `cache_policy.rs`: the page store every provider shares. Keys are nori's own (`album/<id>`,
  `albums/<sort>/...`), freshness windows per `Read`, a provider's `scope` appended (Subsonic: its music
  folders). Answers a provider gives raw are kept on disk and parsed again on every read; pages are kept
  in memory for the session. The overlays apply to every page on every read: the session's star marks
  (favourites gain hearted songs whose record is known and lose unhearted items), lengths filled in from
  known song records. Writes drop what they make stale by kind. Home is kept for the session and a
  refresh merges the new feed into what was shown.
- `providers/subsonic/`: the protocol: endpoints and params, the wire format and its parser, music
  folder scope, signing and the second address, the offline write queue, the library walk.
- `providers/youtube.rs`: InnerTube's answers as `Page`s, likes, streams, sign-in; no caching of its own.

Left on the core: the account services that need the server itself (remote relay, Navidrome history,
scrobbles' queue) behind `Provider::subsonic_api`, and `Core::server` they sign with.
