# YouTube Music as a profile kind (issue #23, phase 1)

Internal planning note, not published. Written 2026-10-17 against `master` at `06e615c9`.

Owner decisions this follows (final): YouTube Music is its own server-less profile kind, added like a
server and never mixed into a Navidrome profile; pure Rust, no JavaScript engine and no WebView; playback
only through the InnerTube **visionOS** client; search and browse through the WEB_REMIX
(music.youtube.com) JSON endpoints; v1 is search, browse (artist, album, playlist), play, lyrics and
AutoMix; no login, no library sync, no remote control or jams, nothing on the web client. Best effort:
the words say so, since YouTube can break it overnight.

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
- An unknown video id answers `status` ERROR, `reason` "This video is unavailable".
- Without a visitor id, music tracks (`MUSIC_VIDEO_TYPE_ATV` and `OMV`, e.g. `wU26xVT_vBU`) answer
  `LOGIN_REQUIRED` "Sign in to confirm you're not a bot" for every visionOS build (1.01, 1.02, 1.03),
  while a plain video (`dQw4w9WgXcQ`) plays. Sent the `responseContext.visitorData` of any earlier
  answer (the refusal's own, or a WEB_REMIX search's from an hour before) as `X-Goog-Visitor-Id` and
  `context.client.visitorData`, they all answer OK. yt-dlp sends it too (`generate_api_headers`).
- WEB_REMIX `search` with the songs/albums/artists filter `params` (the ytmusicapi values) answers a
  `musicShelfRenderer` of `musicResponsiveListItemRenderer`s (20 per page). The unfiltered search now
  answers about 30 mixed `itemSectionRenderer`s, a shape that changed in 2026, so filtered searches are
  used.

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
| writes (stars, scrobbles, playlists, queue push) | Subsonic writes, queued offline | none: `Client::get` refuses every Subsonic call for this kind, so nothing is queued |
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

`ProfileRules` gains `remote` (remote control and jams offered) and `measures` (AutoMix measures songs
ahead), and `youtube_rules()` is the kind's: `asks` false, `account` false (no hearts, ratings,
playlists, downloads, history, scrobbles, queue pushes), `indexed` false (no sync; the songs list reads
nothing), `sections` empty (no library to list; search is the way in), settings without
`ServerOptions`, `Library`, `Downloads` and `Streaming` (no transcode qualities, and fetching ahead is
fixed), `remote` false, `measures` true. `crates/host/src/session.rs`
reads `measures` for the measurer and `remote` before making a `Remote`; a jam guest keeps `remote` true
(it is in a jam) and `measures` false, an account has both, so Navidrome profiles behave as before.

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
   once more with the one it carried. The id is kept in memory only and never logged.
3. From byte 0 the URL carries `&range=0-<clen-1>` (a plain GET is throttled, above); from later bytes
   the `Range` header the byte source sends does the same.
4. A refused URL (403: the address changed, so the `ip=` binding no longer holds, or YouTube expired it
   early) drops the resolution, so the loader's own next try (its usual retries of a refused status)
   asks YouTube again; when those run out the song fails as any stream error does.

Expiry is per resolution, so a song paused for hours resolves again on its next open; nothing refreshes
in the background (no timers while playing, AGENTS.md).

## Rate limits and what is never prefetched

- One player request per song played, when its bytes are first needed; none for songs shown in a list.
- Fetching ahead (`Client::precache_targets`) is capped at the next song (`youtube::AHEAD` in the core), whatever
  the precache settings say: each one costs a player request and a whole download.
- No autofill (`account` false; its reads are Subsonic's), no mixes, no similar songs, no sync, no cover
  prefetching beyond what a page shows.
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
- Terms of service: unofficial clients are against YouTube's. The words say "best effort", no account
  is used, and nothing is prefetched that the user did not queue.
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
`feed` (Home is the service's feed); `account` (playlists, downloads, scrobbles, history, mixes) stays
off for YouTube. `rules_of` reads the profile's `user` for whether it is signed in.

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
