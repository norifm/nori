# YouTube Music as a profile kind (issue #23, phase 1)

Internal planning note, not published. Written 2026-10-17 against `master` at `06e615c9`.

Owner decisions, updated 2026-10-18: YouTube Music is its own serverless profile, separate from a
Navidrome profile. The provider is pure Rust, with no JavaScript engine. Playback uses InnerTube's
**visionOS** client; search and browse use WEB_REMIX JSON. Android's visible Google sign-in WebView is
only for signing in. Search, browse, playback, lyrics, AutoMix and native downloads use the shared
provider and engine. Signed-in profiles expose the account's library and song likes. There is no
YouTube profile in the web client. YouTube can change these unofficial APIs at any time.

## LAN pairing and jams: approved design, implementation pending

The owner now wants LAN crossplay and jams for YouTube profiles. Devices pair explicitly with a QR
code or link carrying a random shared secret. Discovery advertises a keyed hash of the pairing realm,
not a Google account ID. Hashing a public account ID alone would not authenticate a controller. LAN
requests use the existing HMAC authentication with the pairing secret, and pairing exchanges no Google
cookies or account identifiers. Pairing must survive restarts and offer a way to forget a device.

A jam has a separate temporary invite secret. A guest gets a temporary, signed-out YouTube profile
and resolves each `yt-` song locally. Existing profiles and their sign-in sessions stay intact and the
previous profile returns when the guest leaves. No Navidrome profile or server is required. Reusing an
existing server profile for relay transport is a separate, undecided option.

## Feature audit (2026-10-18, `slim`)

This table distinguishes the shared implementation from platform verification. Android and CLI have
been exercised with a real signed-out YouTube profile. Desktop and iPod share the tested native host;
their current integration has compiled, but this audit does not claim a live iPod or desktop run.
The web client intentionally has no YouTube profile.

| Feature | Android | Desktop | CLI | iPod | Evidence or remaining limit |
|---|---|---|---|---|---|
| Search, albums, artists, playlists, Home | Implemented | Implemented | Implemented | Implemented | Provider fixtures; signed-out search/play checked live |
| Radio from a song | Song menu | Song menu | Player's Radio control | Song menu | `next` watch queue, provider and parser tests; live metadata request returned 50 tracks without fetching audio |
| Autofill from similar songs | Implemented | Shared host | Shared host | Shared host | Uses the same metadata-only related-track read; genre/era choices depend on metadata YouTube supplies |
| Instant mix and local smart mixes | Account-only UI | Existing account-only UI | Existing account-only UI | Existing account-only UI | No broken YouTube action is exposed; radio supplies YouTube's recommendations |
| Local listening history | Library history and stats | Recorded | Recorded | Recorded | Shared queue records listens independently of Subsonic scrobbling; native clients without history screens have the same limitation with Navidrome |
| Account listening history and scrobbling | Not implemented | Not implemented | Not implemented | Not implemented | YouTube's account history endpoints are not mapped; local history remains available |
| Save and restore this device's queue | Implemented | Implemented | Implemented | Implemented | Queue serialization is provider-neutral; server queue pushes remain Subsonic-only |
| AutoMix and automatic lyric alignment | Implemented | Shared engine | Shared engine | Shared engine | WebM pair/voice tests and Android live mix; analysis reads fetched playback bytes |
| Download songs, albums and playlists | Implemented | Implemented | Implemented | Implemented | Provider resolves the download URL; CLI real download/offline replay passes; Android verification tracked in the batch checks |
| Playback, cover and lyric widgets | Implemented | — | — | — | Provider-neutral media-session/lyric data; Android glue needs the batch's YouTube check |
| For You and album widgets | Provider playlists and feed albums | — | — | — | Metadata-only feed reads; tap fetches the selected collection |
| Android Auto browse/search/voice/offline | Provider Home/library, generic search/voice/downloads | — | — | — | Car regression covers signed-out Home and removes unavailable account folders; live car glue remains in batch checks |
| Hearts and library | Sign-in WebView | Paste cookies | Cookie file | Paste cookies | Signed-in fixture tests cover session rotation, saved albums/playlists, liked songs and likes; no real signed-in account was supplied for this audit |
| Edit playlists, save albums, library artists | Not implemented | Not implemented | Not implemented | Not implemented | Account mutation UI stays hidden; library and liked-song pagination beyond the initial page remain incomplete |
| LAN crossplay and jams | In progress | In progress | In progress | In progress | See pairing design below; relay reuse remains undecided |

The signed-out `SongById` path first uses queue or finished-download metadata without a network
request. If the song has not been kept here, the watch queue supplies its metadata. This also keeps
song actions and restored requests from silently receiving an empty Subsonic-shaped answer.

## Sources

- yt-dlp master, `yt_dlp/extractor/youtube/_base.py`, `INNERTUBE_CLIENTS['visionos']` (fetched
  2026-10-17): `clientName` `VISIONOS`, `clientVersion` `1.02`, `deviceMake` `Apple`, `deviceModel`
  `RealityDevice17,1`, `osName` `visionOS`, `osVersion` `26.5.23O471`, `userAgent` `Mozilla/5.0
  (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15`,
  `INNERTUBE_CONTEXT_CLIENT_NAME` 101, `REQUIRE_JS_PLAYER` False, no `GVS_PO_TOKEN_POLICY`. Host
  `www.youtube.com` (the default). `_video.py`: `_DEFAULT_JSLESS_CLIENTS = ('visionos',)`; requests send
  `contentCheckOk`/`racyCheckOk` (`_get_checkok_params`). Headers (`generate_api_headers`):
  `X-YouTube-Client-Name`, `X-YouTube-Client-Version`, `Origin`, `User-Agent`; `?prettyPrint=false`.
- yt-dlp `INNERTUBE_CLIENTS['web_music']`: host `music.youtube.com`, `WEB_REMIX` `1.20260707.12.00`,
  client name 67. Its GVS policy needs a PO token for streams, which is why it is used for search and
  browse only.
- ZemerTeam/zemer-app (HEAD of 2026-10-09): `innertube/.../models/YouTubeClient.kt` has the same visionOS
  1.02 values and notes that 0.1 was retired on 2026-10-07 (HTTP 400 "Precondition check failed") and that
  1.01 and 1.03 still drained whole songs that day (1.0 and 1.1 are 404); `app/.../utils/YTPlayerUtils.kt`
  uses visionOS URLs as they come ("no sig, no n-transform, no pot"), notes the `spc` parameter added in
  2026-10, and takes the expiry from `streamingData.expiresInSeconds`, else the URL's `expire`.

## Measured here (2026-10-17, from this Mac, no account)

- `POST https://www.youtube.com/youtubei/v1/player?prettyPrint=false` with the visionOS 1.02 context:
  `playabilityStatus.status` OK, `expiresInSeconds` 21540 (6 h), audio itags 139, 140 (AAC 130 kbps),
  249, 250, 251 (Opus 137 kbps), each with a plain `url` (no `signatureCipher`), `c=VISIONOS`, `spc`,
  `ip=` (the URL is bound to the requesting address).
- The 251 URL without a `Range` header is throttled to about 32 KB/s (106 s for a 3.4 MB song; 2.3x
  real time). With `Range: bytes=0-`, any range, or the `&range=0-<len-1>` query parameter it comes at
  full speed (0.35 s). There is no 1 MiB wall for visionOS URLs.
- That holds for small files only. A range of about 50 MB or more, or one without an end
  (`Range: bytes=F-`, `&range=F-<len-1>`), comes at the throttled 32 KB/s too: starting a 2.4 h, 155 MB
  mix at 1 h waited 6.2 s for its first 256 KB. Ranges of 10 MB or less come at full speed anywhere in
  the file (10 MB in 0.95 s; `Range: bytes=F-(F+1MB)` answered in 0.14 s). With `&range=` in the URL,
  a `Range` header counts from the slice's start.
- An unknown video id answers `status` ERROR, `reason` "This video is unavailable".
- Without a visitor id, music tracks (`MUSIC_VIDEO_TYPE_ATV` and `OMV`, e.g. `wU26xVT_vBU`) answer
  `LOGIN_REQUIRED` "Sign in to confirm you're not a bot" for every visionOS build (1.01, 1.02, 1.03),
  while a plain video (`dQw4w9WgXcQ`) plays. Sent the `responseContext.visitorData` of any earlier
  answer (the refusal's own, or a WEB_REMIX search's from an hour before) as `X-Goog-Visitor-Id` and
  `context.client.visitorData`, they all answer OK. yt-dlp sends it too (`generate_api_headers`).
- WEB_REMIX `search` with the songs/albums/artists filter `params` (the ytmusicapi values) answers a
  `musicShelfRenderer` of `musicResponsiveListItemRenderer`s (20 per page). The unfiltered search (no
  `params`) answers a `musicCardShelfRenderer` top result (its title links the item; its `contents` are
  the top artist's songs, with no artist of their own) and then about 30 `itemSectionRenderer`s of one
  item each, every kind ranked together: songs, videos (UGC), albums, artists, profiles (user channels),
  playlists, podcasts and episodes. Each item's second column opens with its kind ("Song • ...",
  "Profile • @handle"), which for an album is its release type. nori asks the unfiltered search and the
  three filtered ones side by side: the unfiltered answer leads each kind, the filtered ones add depth.
  Podcasts and episodes are dropped. A channel handle (`@name`) matches its videos, not a profile.

## What a profile kind provides (the `Provider` seam)

A profile kind answers the same reads the clients already make (`cache_policy::Read` into
`cache_policy::Page`), so pages, the queue, the engine, lyrics and AutoMix need no second path. Per kind:

| need | Subsonic (today) | YouTube Music |
|---|---|---|
| reads | `spec(read)` endpoint + `Parser` (`crates/core/src/cache_policy.rs`) | `Client::youtube_read` (`crates/core/src/youtube.rs`) over `innertube_music` |
| search | `search3` | three filtered WEB_REMIX searches, side by side |
| album, artist, playlist | `getAlbum`, `getArtist`, `getPlaylist` | WEB_REMIX `browse` of `MPREb_...`, `UC...`, `VL...` |
| stream address | signed `stream?id=` (`Core::stream_url`) | `youtube:<videoId>`, resolved when opened (below) |
| covers | signed `getCoverArt&id=&size=` (`Core::cover_address`) | the thumbnail's own URL at the asked size (`innertube_music::thumbnail_url`) |
| lyrics | `getLyricsBySongId` plus the lyrics services | the lyrics services only (the server read is empty) |
| writes | Subsonic writes, queued offline | signed-in song likes; Subsonic scrobbles, playlist edits and queue pushes are refused |
| sync | `search3` pages | none (`ProfileRules::indexed` false) |

In code the seam is the `Provider` trait (`crates/core/src/providers/`, docs/site/developers/providers.md):
`ServerKind` on `SavedServer` picks the provider `Core::configure_profile` installs, and the Subsonic
server and YouTube Music are its two implementations. YouTube Music's is one module
(`providers/youtube.rs`, `providers/youtube/account.rs`) over one platform-free crate for its wire
format (`crates/youtube`); the names below of the first version (`configure_youtube_music`,
`youtube_read`, `Resolving` over `Client::playable`, `youtube_sign_in`) are now that module's
`Provider` methods, the engine's generic `Resolving` over a `ResolveStream`, and `Client::sign_in`.

### Ids

Each profile has its own rows (`sid()`), but the stream store (`<data>/music`) and the cover cache are
shared by all profiles, so YouTube ids are namespaced: songs `yt-<videoId>`, albums `yt-MPREb_...`,
artists `yt-UC...`, playlists `yt-VL...` (`ID_PREFIX` in `crates/core/src/youtube.rs`). They are not `ext-` ids: they never
change once played, so `Song::is_provider` stays false and pages are indexed as read (local search finds
what was seen). Records are nori's own (`Song`, `Album`, `Artist`, `Playlist`, `ArtistInfo`, via
`AlbumDetail::new` and friends), so every page and row draws them as it does Navidrome's.

### Rules (`crates/library/src/browse.rs`)

`ProfileRules::downloads` is independent of `account`: local downloads need no Subsonic account.
YouTube profiles offer Downloads and Storage settings, their own Home feed and AutoMix analysis.
Signed-in profiles also offer albums, favorites and playlists. Subsonic-only account work stays off.
Remote control remains off until the pairing and local jam transport above are implemented.

The cache key is always the provider's, including when a platform looks up a whole cached song for
analysis. Download addresses also come from the provider and resolve on the download worker. The same
WebM bytes feed playback analysis; no analysis-only network request is made.

## Playing: from id to bytes

1. `Client::stream_target` (`crates/core/src/stream.rs`) gives a YouTube song the address
   `youtube:<videoId>` and the key `yt-<videoId>:0` (the quality settings do not apply: there is no
   transcode). The key is stable, so a cached song plays from `<data>/music` without asking YouTube.
2. The engine opens addresses through its `ByteSource`. For a YouTube profile the host wraps it in
   `nori_engine::core::Resolving` (only then: a Navidrome profile pays nothing). On the loader's thread
   (never the engine's) it asks `Client::playable(address, from)`, which takes a resolved stream from
   the core's `Streams` or makes the visionOS player request (`InnerTube::player`), picks itag 251,
   else 140, else the best Opus, else the best AAC (`player::pick`), and keeps it until 5 minutes
   before `expiresInSeconds`. `innertube_music::InnerTube` (one per client) also keeps the visitor id the
   first answer gave and sends it with every later request; a player refusal for want of one is asked
   once more with the one it carried. The id is never logged; nori keeps it in the profile's
   `youtube_visitor` row and gives it back before the next start's first request, since a new
   visitor gets a different Home each time.
3. The engine asks for the plain URL in parts of at most 10 MB (`Range: bytes=F-E`), opening the next
   at each part's end (`ResolveStream::parts`, nori-engine's `Resolving`); the same 1 h start plays in
   about a second. media3's cache learns a song's length only from a request without an end, so on
   Android the length a part's `Content-Range` says is kept as the entry's (`MediaSources.rememberLength`);
   without it a song never counted as whole and was never analysed. Android's download worker asks the
   whole file in one request, so its URL carries
   `&range=0-<clen-1>` from byte 0 (`ResolveStream::resolve_rest`).
4. A refused URL (403: the address changed, so the `ip=` binding no longer holds, or YouTube expired it
   early) drops the resolution, so the loader's own next try (its usual retries of a refused status)
   asks YouTube again; when those run out the song fails as any stream error does.

Expiry is per resolution, so a song paused for hours resolves again on its next open; nothing refreshes
in the background (no timers while playing, AGENTS.md).

## Rate limits and what is never prefetched

- One player request per song played, when its bytes are first needed; none for songs shown in a list.
- Fetching ahead (`Client::precache_targets`) is capped at the next song (`youtube::AHEAD` in the core), whatever
  the precache settings say: each one costs a player request and a whole download.
- Radio requests only metadata from WEB_REMIX `next` (`RDAMVM<videoId>`); no audio is resolved until
  playback or an explicit download asks for it. Subsonic-only library sync and account queue pushes
  stay off. Covers are fetched for what is shown.
- Browse reads are not cached in the response cache in v1 (`read_stored` has nothing for this kind); the
  pages are indexed. Caching them is the first follow-up if YouTube starts answering 429.
- A 429 or a "Sign in to confirm you're not a bot" playability answer comes back as an error for that
  song (`NetError::Api` with YouTube's reason); there is no retry loop.

## Risks

- visionOS can be retired like 0.1 was. 1.01 and 1.03 are the known siblings; the fallback list belongs
  in `innertube_music` with a test, not a retry loop. When all are gone, playback stops working until a new
  JS-less client is found; search and browse keep working.
- The WEB_REMIX shapes change (the unfiltered search did in 2026). Parsing is defensive (a missing field
  drops the item, not the page) and the fixtures in `crates/youtube/tests/fixtures` say what was parsed.
- Terms of service: unofficial clients are against YouTube's. The words say "best effort"; playback
  uses the anonymous client even while the library is signed in.
- `ip=`-bound URLs break when the network changes mid-song; the 403 path above covers the next open.
- The visitor id requirement may tighten into PO tokens for visionOS too (it did for WEB and MWEB).
  That would end JS-free playback.
- Age-gated and "made for kids" videos do not play with visionOS (yt-dlp falls back to web_embedded,
  which needs JS); they fail as unplayable.

## Wired in this phase

- `crates/youtube` (now innertube-music, see "Decoupled" below): requests over `nori_net::transport::Transport` (no new HTTP stack),
  WEB_REMIX search/browse parsing and the visionOS player, tested against trimmed real answers
  (`tests/fixtures`: tracking, menus, session data and the requester's address removed; googlevideo
  URLs replaced by fixture ones; `not_a_bot.json`'s visitor id is made up).
- settings: `ServerKind` on `SavedServer` (stored as `kind`, absent meaning Subsonic), `server_ready`
  for the kind.
- core: `Core::configure_youtube_music`, `Client::youtube_read`, stream addresses, covers, the guard in
  `Client::get`, `youtube_rules`. Cover cache keys of URLs without `/rest/` now include their path
  (`CoverNet::key_parts`): before, every image of one host shared a key.
- host: the session opens a YouTube profile (`config` only for Subsonic), wraps the byte source, honours
  `remote` and `measures`.
- cli: `--youtube-music` (the full-screen client adds and opens a YouTube Music profile) and
  `nori-cli --script --youtube-music --play <query>` (search and play, or `--wav` render).
- engine: symphonia's WebM reader (`mkv` feature), so itag 251 plays. Pre-skip is the Opus decoder's;
  the last packet's padding (`DiscardPadding`, which symphonia drops) is read by `engine/src/webm.rs`
  when the whole file is there, as MP4's gapless boxes are. AAC 140 plays through the MP4 path as any
  `.m4a` does.

### Live check (2026-10-17, this Mac, debug build)

`nori-cli --script --youtube-music --play "daft punk one more time" --songs 1 --wav out.wav --pace 4`:
WEB_REMIX search found "Daft Punk - One More Time (Radio Edit) (5:21)"; the visionOS player gave itag
251; the whole 5.3 MB song came in one ranged request into the stream cache; the engine rendered
5:20 of audio (mean -14.8 dB) with 0 underruns. The first attempt, before the visitor id, was refused
"Sign in to confirm you're not a bot"; the second, before the `mkv` feature, failed to probe the WebM.
The full-screen client's `--youtube-music` was built but not driven by hand.

## Android, desktop and iPod (second phase, done)

Wired as planned below: the sign-in forms offer the kind, `Resolving` wraps Android's `JavaBytes` and
fetch-ahead bytes, `rules_of(kind, guest)` gives the clients their rules without opening a core, and
the song menu takes `account`. Kotlin's OkHttp keeps a User-Agent the core's exchange names.
`tools/feature-e2e.sh --only youtube-music` plays a searched song on a device.

## What Android and the desktop would take (the plan)

- Android: `crates/android/src/player.rs` has its own library (not `CoreLibrary`): it needs the same
  `Resolving` wrap and the visitor/stream state through the `Client`. The login screen offers "YouTube Music (best effort)" as a kind (Kotlin
  `newServer` already passes `kind`); `Nori.kt` calls `configureYoutubeMusic()` instead of
  `configure(config)` for that kind; wrapping `JavaBytes` in `Resolving` gives media3/OkHttp the
  googlevideo URL; the remote-control and jam entry points must check `rules.remote` (the host does); the cover loader already takes
  whole URLs from `Core::cover_address`; screens follow `ProfileRules` (hearts, downloads and library
  sections hide by themselves; the Remote and Jam entries need `rules.remote`). Strings: the kind's name,
  the best-effort line, "Not available on YouTube Music" where a Subsonic-only action is reached. No new
  device check is needed beyond `tools/smoke.sh` with a YouTube profile, since the decisions are Rust's.
- Desktop (Slint): the same login choice and the same `rules.remote` check; it opens through
  `nori_host::session`, so playback, covers and rules come for free.
- iPod: through `nori_host` too; the login form needs the kind.
- Not planned: the web client (owner's decision).

## Decoupled: innertube-music (owner's decision, 2026-10-17)

`crates/youtube` is the package `innertube-music` (lib `innertube_music`), meant to move to its own
repository and be usable without nori. It depends on `serde_json` only: no nori crate, no HTTP client,
no runtime. It builds `Request`s and parses `Response`s into its own plain records with YouTube's ids
(`Song.video_id`, `Album.id` `MPREb_...`, `Artist.id` `UC...`, `Playlist.id` `VL...`) and sends them
through its own small `Http` trait (`fn send(Request) -> impl Future<Output = Result<Response, E>> +
Send`). It keeps its README, MIT license field, tests and fixtures. Checked standalone: the directory
copied into a scratch workspace of its own builds and passes its tests; `cargo package --list` lists
only its own files.

nori's side, `crates/core/src/youtube.rs`, owns everything nori: the `yt-` ids and `youtube:` stream
addresses, the mapping to `Song`/`Album`/`Artist`/`Playlist` (album subtitles, counts, `dressed()`),
the transport (`Wire`, the platform's `Transport` as `Http`), error mapping into `NetError`, the stream
resolution cache (`Streams`, 5 minutes' margin), `AHEAD` and the rules. Behavior is unchanged; the
one difference is that an album page's tracks without artists of their own now also carry the album's
artist links (`Song.artists`), not only its `artist_id`.

## Signing in (owner's decision: login that stays signed in, 2026-10-17)

Researched 2026-10-17. What signs a non-browser client in to music.youtube.com's InnerTube, and how long
each stays signed in:

- **OAuth device flow (TV client) is closed.** yt-dlp removed its YouTube OAuth in 2024.11.18 (PR
  #11558, "Login with OAuth is no longer supported for YouTube") after YouTube restricted the TV client's
  path. ytmusicapi 1.9 (Dec 2024) then asked users for their own Google Cloud client ("TVs and Limited
  Input devices", YouTube Data API on); since 2025-08-29 every OAuth request to WEB_REMIX answers 400
  "Request contains an invalid argument" (ytmusicapi issue #813, still open in Aug 2026 on 1.12.2; the
  maintainer: "There is no solution unfortunately … You must use the browser based auth variant"). The
  token still works with the TVHTML5 client (a different, tvRenderer-shaped API nobody parses) and with
  the YouTube Data API v3 (playlists only, with quota). Durability would be poor too: a Google Cloud app
  in "Testing" gets refresh tokens that expire after 7 days for the `youtube` scope; "In production"
  needs Google's verification of that scope. Not implemented.
- **Browser cookies with SAPISIDHASH are what works** (ytmusicapi "browser" auth, yt-dlp, Metrolist,
  SimpMusic, InnerTune and its forks). Signed-in WEB_REMIX requests carry the session's `Cookie`,
  `Authorization: SAPISIDHASH <t>_<sha1("<t> <SAPISID> <origin>")>` (yt-dlp adds `SAPISID1PHASH` and
  `SAPISID3PHASH` from `__Secure-1PAPISID`/`__Secure-3PAPISID`), `X-Goog-AuthUser: 0` and
  `X-Origin: https://music.youtube.com` (ytmusicapi `helpers.py`, yt-dlp `_base.py`
  `_get_sid_authorization_header`).
- **Why exported cookies die.** Google rotates `__Secure-1PSIDTS`/`__Secure-3PSIDTS` and the `SIDCC`
  family; a browser tab left open on YouTube keeps rotating the session, and two holders rotating one
  session undo each other. yt-dlp's wiki ("Exporting YouTube cookies"): "YouTube rotates account cookies
  frequently on open YouTube browser tabs as a security measure", so sign in in a private window, export,
  then "close the private browsing/incognito window so that the session is never opened in the browser
  again". ytmusicapi's docs promise about two years; users report 24–48 hours when the browser stays in
  use (ytmusicapi #921). A session that nobody rotates also goes stale (ytmusic-rs says within the hour;
  not measured here).
- **Rotation keeps it alive.** `POST https://accounts.youtube.com/RotateCookies` with the session's
  cookies, `Origin: https://accounts.youtube.com` and the body `[000,"-0000000000000000000"]` answers
  new `__Secure-1PSIDTS`/`__Secure-3PSIDTS`/`SIDCC` in `Set-Cookie` (accounts.google.com answers a
  youtube.com session 401). 429 means too soon (a session just signed in counts); 401 means the session
  is over. Done this way by HanaokaYuzu/Gemini-API (`rotate_1psidts.py`, for google.com),
  TonybynMp4/yMusic (`docs/sign-in.md`: at launch and every 10 minutes), cwuom/NeriPlayer(-Desktop)
  (`refresh.rs`) and sonorahq/ytmusic-rs. Answers from music.youtube.com carry rotated cookies in
  `Set-Cookie` too.
- **A stale session fails quietly**: YouTube answers 200 as to a visitor, `responseContext`'s
  `serviceTrackingParams` saying `logged_in` `0`.
- ytmusicapi #962: the cookies worked elsewhere only from the browser's own public IP. A phone signing
  itself in from its own network is that case.

**Chosen: browser-session cookies, rotated by nori.** No JavaScript engine, no Google Cloud client, no
central service. How it stays signed in (`crates/youtube/src/auth.rs`, `crates/core/src/youtube/account.rs`):

1. The user signs in at music.youtube.com in a private window, copies the `Cookie` header of any
   request (or a cookies.txt), pastes it into the profile, and closes the window, so nothing else
   rotates the session. `Client::youtube_sign_in` checks it by asking `account/account_menu` for the
   account's name (kept as the profile's `user`, which the rules read) and keeps the cookies.
2. The cookies are kept in the profile's `kv` row `youtube_session` (`{cookies, rotatedAt, ended}`),
   not in `StoredPrefs`: the platform holds the settings and writes them back whole, and would put back
   cookies from before a rotation. Removing the profile deletes the row (`forget_server`).
3. Before any YouTube call, a session last rotated `ROTATE_EVERY` (10 minutes) ago or more is rotated
   first; nothing runs on a timer (AGENTS.md), so a profile not used for days rotates on its next use.
   Every `Set-Cookie` of a signed-in answer or a rotation is merged into the jar and written to the row
   at once (`InnerTube::take_changed`, `Client::youtube_after`).
4. A transport failure, a 5xx or a 429 changes nothing (the rotation is tried again on the next call).
   A signed-in answer refused (401) or given as to a visitor is retried once after a rotation. Only a
   rotation answered 401, or a refusal that a rotation does not set right, ends the session: the row is
   marked `ended` (the cookies stay), `youtube_account` says `Ended`, the clients say "YouTube signed you
   out, sign in again", and the profile carries on as a visitor's (search, Home, playback).
5. Streams stay on the anonymous visionOS client: cookies are sent only to music.youtube.com and
   accounts.youtube.com. Signed-in users of other clients get "Sign in to confirm you're not a bot" when
   their player requests carry the account (Metrolist #4220/#4192), and a web session's cookies on a
   non-web client are what YouTube flags (yMusic).

What it unlocks (v1): Home is YouTube's own feed signed in or out (`FEmusic_home` and its first
continuation, `Read::HomeFeed`, `Page::Feed` of `FeedShelf`s); signed in, Library offers Albums
(`FEmusic_liked_albums`, one page), Favourites (the "Liked music" playlist, `VLLM`, its first page,
hearted) and Playlists (`FEmusic_liked_playlists`); a song's heart is a like (`like/like`,
`like/removelike`, never queued offline). `ProfileRules` gained `stars` (hearts shown and sent) and
`feed` (Home is the service's feed); `account` (Subsonic playlist mutations, scrobbles, queue pushes and library mixes) stays
off for YouTube. Local downloads, radio and history have independent capabilities. `rules_of` reads the profile's `user` for whether it is signed in.

Left out: editing playlists and adding to them, saving albums to the library, more than the first page
of liked songs and library albums, artists in the library, the account's history, liked state of songs
outside the favourites (hearts show this session's marks), brand accounts (`X-Goog-PageId`) and the
`_u` SAPISIDHASH form with the data-sync id, signed-in search ranking, scrobbling to YouTube history.

**For the owner to decide**

- *A sign-in window instead of pasting* (owner's yes, built on Android). Pasting a Cookie header is
  fine on the desktop and painful on a phone. Android opens Google's sign-in page
  (`auth::SIGN_IN_PAGE`, `ServiceLogin?service=youtube&continue=music.youtube.com`, as Metrolist and
  InnerTune) in a WebView of its own, with the WebView's default user agent as they keep it, no
  JavaScript bridge and https only. On every page it reads `CookieManager.getCookie(url)` and asks the
  core `youtube_sign_in_done(url, cookies)` (`auth::signed_in_at`: back on music.youtube.com with an
  `SAPISID`); then the whole header goes to `youtube_sign_in`, and the WebView's cookies and storage
  are cleared (`removeAllCookies`, `WebStorage.deleteAllData`) at once, on Cancel, and before it opens,
  so it never rotates the session. Metrolist instead keeps the WebView's cookies. Playback never uses
  a WebView.
- *OAuth with the user's own Google Cloud client.* Only worth building if YouTube reopens WEB_REMIX to
  OAuth tokens (#813). It would need every user to create a "TVs and Limited Input devices" client, and
  publish it to "In production" to escape the 7-day expiry.
- *Account risk.* Unofficial clients are against YouTube's terms; the yt-dlp wiki warns of temporary or
  permanent bans for account use, though none were found reported for browse-only use. The sign-in form
  says so, and playback never uses the account.

Sources: https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies,
https://github.com/yt-dlp/yt-dlp/pull/11558, https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html,
https://ytmusicapi.readthedocs.io/en/latest/setup/oauth.html, https://github.com/sigma67/ytmusicapi/issues/813,
https://github.com/sigma67/ytmusicapi/issues/921, https://github.com/sigma67/ytmusicapi/issues/962,
https://github.com/sigma67/ytmusicapi/discussions/682,
https://developers.google.com/identity/protocols/oauth2#expiration,
https://developers.google.com/identity/protocols/oauth2/limited-input-device,
https://github.com/MetrolistGroup/Metrolist/issues/4220, https://github.com/TonybynMp4/yMusic/blob/HEAD/docs/sign-in.md,
https://github.com/HanaokaYuzu/Gemini-API, https://github.com/sonorahq/ytmusic-rs, https://github.com/cwuom/NeriPlayer-Desktop.

Fixtures: `home.json`/`home_more.json` are a signed-out answer from this Mac (trimmed; tracking, menus
and the visitor id replaced). The signed-in ones (`library_playlists.json`, `library_albums.json`,
`account_menu.json`, `signed_out.json`) were written by hand after ytmusicapi's parsers (`parsers/library.py`,
`get_library_contents`) with public ids and a made-up name: no account was used. A real sign-in is still
to be tried by the owner (`nori-cli --script --youtube-music --sign-in cookies.txt --home`).

The radio request fields and watch-queue shape follow the primary implementation in
[ytmusicapi watch.py](https://github.com/sigma67/ytmusicapi/blob/main/ytmusicapi/mixins/watch.py).
`radio.json` is a hand-written fixture with public video ids and invented artist/album metadata; it
contains no account data or tracking values.
