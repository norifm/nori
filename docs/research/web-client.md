# The web client

Internal planning note, not published. Issue #25, now integrated into the `slim` development branch.
How to build, serve and check it: `docs/site/developers/building.md`, "The
web client".

Labels: **[measured]** = measured for this note, with how; **[verified]** = run in headless Chrome over
`web/serve.py` and the local Navidrome (`web/check/check.mjs`); **[inferred]** = reasoning or an
estimate.

## Where it stands

A browser plays the library through the same session and engine as the other clients:

- **Sign-in, library, search** [verified]: Listen Now (newest, recently played, most played, random
  albums), the library's sections (playlists, artists, albums with their sort orders, songs, genres,
  favorites), album, artist, playlist and genre pages, and search over the server. Profiles are kept in the
  settings database; a reload opens the last one without signing in again.
- **Streaming** [verified]: songs play as their bytes arrive (one `fetch` per burst, read chunk by chunk);
  a 10-minute FLAC played 40 s through a proxy held to 400 kbit/s without the AudioWorklet running dry.
  First sound 275 ms after the click on localhost (the player module is already up, below).
  A song starts once the music at hand and still to come lasts as long as the rest takes to arrive at
  the rate measured (`arrives_in_time` in `crates/engine/src/source.rs`: at least 1 s of music and 16 KB,
  the rest counted 1.25 times slower, the rate known after 250 ms), or at 256 KB as before: through a
  proxy held to 400 kbit/s, a 124 kbit/s FLAC sounds 1.0 s after the click instead of 5.2 s and plays 20 s
  without running dry. A song arriving slower than it plays still starts at 256 KB (natively nothing
  changed: a loader on its own thread waits for 256 KB as ever).
- **AutoMix measured ahead** [verified]: with no stream cache, songs are measured as their loaders fetch
  them (the playing song from its first byte, and the next), decoded on the worker's turns
  (`nori_engine::arriving::Steps`, driven by `nori_host::session::Driven`): slices of 8 ms at most half the
  time, a song opened once 4 MB (or all of it) waited and read on while 256 KB do, so a read never runs
  dry inside a packet; at most 64 MB wait. A stored measurement replans the transition. In the check the
  plan out of a 52 s MP3 into the next album's first song was a measured `MixRampFade` 256 ms after Play
  (before: `EqualPowerFade`, "not analysed").

- **WebCodecs decoding** [verified]: Chrome decodes MP3, AAC, FLAC, Vorbis and Opus itself; the engine
  feeds it packets ahead and takes each one's samples back with its own stamps. A codec the browser lacks
  and `player.wasm` does not decode (ALAC everywhere; Vorbis or Opus in some browsers) is decoded by
  `player-codecs.wasm`'s Rust decoders (`Codecs`), the module loaded beside the player the first time a
  song needs it and kept: the same answers as WebCodecs through `decoder.ts`'s `RustDecoding`, so the
  queue, the session and the position are untouched. A 20 s ALAC song plays in Chrome (peak 0.20, no dry
  quantum).
- **The shared session** [verified]: nori-host's session runs in the worker: queue saves, scrobbles, lyrics
  prefetch, autofill, the index sync, reachability.
- **Player** [verified]: mini bar and Now Playing sheet (seek, previous/next, shuffle, repeat), the queue
  (jump, remove), synced lyrics lit line by line from the core's lyric clock, speed and pitch (the Rust
  Signalsmith port: 1.5x and 0.75x keep pace, pitch 1.1 plays), AutoMix and crossfade transitions heard
  with no silence over 20 ms, Media Session metadata and keys.
- **Jams, host and guest** [verified, `web/check/jam.mjs`]: `async-remote` is merged; the remote runs on
  `nori_tasks::Local` in the player worker. The sheet's Jam tab and Settings show the same panel: start a
  jam (the invite link, copy), requests accepted or declined, guests listening along or not, members, end;
  or join with a link and a name, the guest's profile opening in place (the library is then the host's),
  the player showing the host's song and queue, picks going to the host as requests, listening along, and
  leaving (also when the host ends it). Checked with two headless Chrome profiles over octo-fiesta's relay
  (tools/jam-server.sh, port 5274) through `web/serve.py --server http://localhost:5274`: link made, guest
  sees the host's song, a guest's pick arrives as a request and, accepted, lands in the host's queue, the
  guest listening along hears it (3.4 s heard in its first 6 s, not running dry), the guest is back on its
  own profile when the host ends the jam. A guest's profile signs in at the invite's server, which must be
  the page's origin (no CORS); invites to another server need that server to serve the client.
- **One tab, and crashes** [verified]: a Web Lock (`nori-databases`) gives the databases to one tab; a
  second tab says nori is open in another tab, and "Use Here" asks the first (BroadcastChannel) to save
  and close its session and stop its worker, then starts in its place with the databases in OPFS. A worker
  that crashes (a Rust panic aborts the module: `WebAssembly.RuntimeError` from a call, or the worker's
  error event) is started again over the saved session; the page seeks to the song and place it saw last
  and plays on if it played, and says so. Checked by throwing a `RuntimeError` in the player worker while
  playing: back on the same song at 2.8 s, playing on (4.4 s 1.5 s later).
- **Service worker** [verified]: `web/src/sw.ts` keeps one build's shell and modules (the Rust-codecs
  player once fetched) under the hash of them all (`web/build.sh` fills it in); never the API or audio. A
  reload took 115 ms, a start without the network the same, the library from the databases. A new build
  installs beside the running one; at the next load the page finds it waiting, has it take over and loads
  again, the old build's files deleted (checked by changing the hash in `web/dist/sw.js`). Workers are
  told their module by name, since a kept answer carries no query string.
- **Playlists and hearts** [verified]: a new playlist from Playlists, songs added from each song's menu
  (Play Next, Play Later, Add to Playlist, Favorite), a playlist renamed, reordered (Move Up, Move Down),
  a song removed and the playlist deleted from its page's menu; hearts on the playing song, albums and
  artists, and the Favorites page with artists, albums and songs. Each is a core `Write` (two new ones:
  `RenamePlaylist`, `updatePlaylist` with a name; `ReplacePlaylist`, `createPlaylist` with the playlist's id,
  which replaces its songs in their new order), kept for later when offline; hearts through
  `Client::star`, the pages drawing the core's marks over what they were read with.
- Looks: big titles and cover grids as the Android app and Apple Music have them; a tab bar on a phone, a
  sidebar from 900 px; light and dark as the system is. Words are the web client's (`web/src/words.ts`).

## What changed in the shared code (every platform, behaviour kept, all tests green)

1. **The loader's decisions apart from its I/O** (`crates/engine/src/source.rs`): `Filling` holds what the
   loader decides (which range to ask for, a body's bytes, its end, failures and rests, a rangeless answer
   skipped to the byte asked for); natively the loader's thread drives it over `ByteSource::open` exactly
   as before. A platform where nothing may block takes it in `ByteSource::push` and drives it from its own
   events (`Filling::poll_next` wakes it when the next step may change). A pushed loader's readers never
   block: a read with no bytes is `WouldBlock`.
2. **Opening a pushed song on the engine's turns** (`crates/engine/src/demux.rs`, `Pushed`): tried once,
   and again from where a try ran out of bytes; reads after the first that found nothing fail, so a try
   stops there. While a song opens on a pushed loader, the windows it read before a restart are kept
   ("pieces"), so a header read at the start and a seek's target far away (FLAC's bisection, Ogg's last
   page) are both at hand on the next try; they go once the reader reads the window again. Tested with
   FLAC, MP3 and Ogg Opus, from the start and half way into a five-minute song with a small window
   (`tests/pushed.rs`); without the pieces the half-way open never finishes.
3. **Decoders that answer later** (`nori_player::decode::LaterDecoder`, `Platform`): fed packets in order,
   each packet's samples come back later with its stamps (`Later` in demux.rs), so encoder delay, padding
   and seeks are cut exactly as with a decoder on the spot. `Platform` bundles Android's HE-AAC decoder and
   this one. Tested against the Rust decoders for MP3, FLAC, Vorbis, Opus and AAC in MP4, from the start
   and after a seek (`tests/later.rs`, a decoder that answers three packets late).
   What Chrome does, measured with a JS harness: one output per packet; MP3's 529-frame decoder delay
   kept (as symphonia keeps it); Opus's pre-skip dropped when given the OpusHead, and again after a reset,
   so the reader drops `pre-roll - pre-skip` after a seek (`later_seek_skip`); output stamps not always the
   input's (FLAC's are recomputed), so packets are matched by order with the stamps as a check.
4. **Rust codecs as a feature**: `rust-codecs` (nori-engine's default) decodes Vorbis, ALAC and Opus here;
   MP3, FLAC and AAC always are, their decoders coming with their readers. The web's player module leaves
   it off; `player-codecs.wasm` has it, loaded where WebCodecs lacks a codec.
5. **Host on tasks** (`nori-tasks`, cherry-picked from `async-remote`: `Tasks::spawn`, `Tasks::sleep`): the
   session's network work (check, sync, play/enqueue later, search, lyrics, stars, scrobbles, autofill, the
   offline bridge, AutoEQ, queue pushes) are futures on `Open::tasks`. Natively `nori_host::Threads` runs
   each on a thread of its own with `block_on` as before (ureq blocks), timers on one parked thread; on the
   web `nori_tasks::Local`. The queue saver is a task alive while a save is due (tested). `Session::
   open_driven` gives the engine to the caller (the worker) instead of a thread; `Open::keeps_music`
   (false on the web: no stream cache, downloads or measuring from disk) and `Open::decoders`.
6. `nori_core::server_config` and `net_profile` (a saved profile's sign-in and addresses) moved from host to
   the core, so the core module needs no host.
7. `Source::Memory` (the spike's whole-song source) is gone: the web streams.

## How it is built

- `crates/web` (nori-web): `lib.rs` the `Nori` the worker calls (open, login/resume/logout, `read` a page,
  covers, settings); `pages.rs` the page reads as JSON; `play.rs` (player build) the host session, the
  AudioWorklet's card, the session's reports to the page, the lyric clock; `stream.rs` each loader's
  fetching as a task over `fetch` (an `AbortController` per request); `later.rs` WebCodecs through
  `web/src/decoder.ts`.
- `web/src/`: `ui/App.svelte` sign-in, `state/route.svelte.ts` routing, `pages/` the pages, `ui/` the
  player bars, full player, lyrics and queue, `state/` their state, `rpc.ts` the worker (calls by promise,
  reports), `worker.ts` the Dedicated Worker (runs a module,
  feeds the worklet, turns the engine when it asks), `worklet.ts` the AudioWorklet (a 4096-frame start
  threshold, counts running dry and silences), `decoder.ts` WebCodecs, `words.ts` every word.
- Loading in pieces: the shell (36.9 KB br); `core.wasm` in a worker for sign-in and the library, so the
  first page is quick; then, once that page has drawn and the browser is idle (`requestIdleCallback`,
  `setTimeout` where Safari has none), the page downloads and compiles the player build
  (`WebAssembly.compileStreaming`, the build chosen by the same WebCodecs probe the worker makes) while
  the core worker keeps answering; then calls wait, the core worker answers what it was asked and goes
  (an OPFS file has one owner), and a worker over the compiled module opens the profile. Play never loads
  anything; a Play pressed during the load waits for that same load. With `navigator.connection.saveData`
  the player is loaded on the first Play instead. The AudioContext is still made on the first Play (it
  needs the click) and handed to the player worker then.
  Earlier-shell measurements [verified]: player worker up 370 ms after navigation on localhost, before any Play; the switch itself
  40 ms; first Play to sound 275 ms. Through `web/serve.py --kbps 4000 --files-kbps 4000` (4 Mbit/s, files sent
  uncompressed: player.wasm is 4.2 MB raw), the background load took 8.6 s after the first page; a Play
  pressed 0.1 s after that page waited 8.5 s for it and played the album once (queue 4 of 4 songs).
  With brotli on a real server that is 1.38 MB, about 2.8 s at 4 Mbit/s [inferred].

## Svelte shell (October 18, 2026)

The shell uses Svelte 5 runes and Vite. `web/src/state/` holds the library, route, playback and remote
state; `web/src/pages/` and `web/src/ui/` draw it and forward actions to the Rust modules. Settings
pages are laid out by the core. The shared host session drives playback, including Play On, lyric
seeking and saved 250 ms timing adjustments; the guest's role determines whether a lyric tap seeks.

Wide windows use a sidebar, bottom player bar and optional side panel. Phones use tabs, a mini player
and a full-screen player. Other pages and the full player load after the first paint.
`web/build.sh --shell` rebuilds just the shell.

October 18, 2026, build `f2a2d692017863f9` [measured with `web/build.sh`]:

| file | raw bytes | brotli bytes |
|---|---:|---:|
| first paint (HTML, CSS, initial scripts) | 113 507 | 36 863 |
| core.wasm | 1 658 122 | 576 089 |
| core.js | 38 932 | 6 818 |
| player.wasm (WebCodecs) | 4 521 430 | 1 457 125 |
| player.js | 60 378 | 9 849 |
| player-codecs.wasm (Rust decoders) | 4 756 831 | 1 537 850 |

The first-paint count excludes the wasm modules, their glue and deferred pages. The earlier shell's
sizes below remain for comparison.

## Earlier measured sizes

`web/build.sh` (`--profile web`: release, `opt-level = "s"`, `panic = "abort"`; wasm-bindgen 0.2.128;
`wasm-opt -Oz`; `brotli -q 11`), October 17, 2026 [measured]:

| file | raw | brotli |
|---|---|---|
| core.wasm (core and db) | 1 575 174 | 552 698 |
| player.wasm (core, db, host, engine; WebCodecs) | 4 439 304 | 1 429 559 |
| player-codecs.wasm (the same with Vorbis, ALAC, Opus in Rust, and `Codecs`) | 4 674 527 | 1 511 002 |
| core.js / player.js (glue) | 33 605 / 46 361 | 6 238 / 7 872 |
| the shell: HTML, CSS, 9 scripts | 68 533 | 19 301 (17 347 as one stream) |

The web builds nori-core without its `youtube` feature: no YouTube Music profile, and with it core.wasm
was 583 KB br. core.wasm was 578 624 br before SQLite was built as the core uses it (no R-tree, sessions, introspection
tables, UTF-16, column metadata, math functions; `-Oz`): 39 KB less. Rust at `opt-level = "z"` made it
5 KB larger compressed. player.wasm grew from the spike's 1 102 048 br by the host session (remote, covers'
URLs, the index sync, settings effects); WebCodecs took 79 KB br off it (vorbis, alac, opus-rs).

Where core.wasm's 2.2 MB of code and data go before wasm-opt (twiggy over a build with names): SQLite and
libc 1 062 000 (48 %), data 176 000, nori-model 134 000 (serde for every record), nori-core 110 000,
nori-settings 107 000, Rust's core/alloc 140 000, serde_json 67 000, nori-lyrics 51 000.

## Verified in headless Chrome

October 18, 2026, `node web/check/check.mjs http://localhost:8081/`, every default phase passing
[verified]: 6 library sections, 20 albums, 12 artists, 4 genres, 79 songs; first sound in 294 ms and
10 s of FLAC with no dry quantum or engine underrun. Lyrics lit lines -1, 0, 1; tapping a line lit it
in 13 ms and sounded from its place in 23 ms. Pause took 15 ms, seek 46 ms, next 23 ms. A measured
`MixRampFade` mixed for 5 s and crossfade for 6 s, both without a silence over 20 ms or running dry.
ALAC played 8 s without running dry. Speed 1.5 and 0.75 ran at 1.51 and 0.75 song-seconds per second;
pitch 1.1 ran at 1.00. Paused and hidden seek bars held no timer. Worker crash recovery, tab takeover,
offline shell reload (114 ms), build replacement, playlist edits and a fresh phone startup over a
saved queue passed too.

The phone startup also passed at 4 Mbit/s (`SLOW_URL`, `--kbps 4000 --files-kbps 4000`): Play was
pressed while the player module loaded, waited 8.8 s and started exactly one four-song album queue.
`web/check/jam.mjs` passed Play On in both directions, accepted a guest request, and played 3.75 s
for the listening guest with no dry quantum.

`web/check/shots.mjs` captures 46 desktop/phone screenshots in light/dark modes, including the
full player, lyrics, queue, jam, settings and the last album row above the phone's mini-player.

Earlier throttled-network check [verified]: with `THROTTLED_URL`
(`web/serve.py --port 8082 --kbps 400`), first sound came 1.0 s after the click and 20 s played without
running dry or an engine underrun.

Not verifiable headless: iOS Safari (background audio and a locked screen; its WebCodecs set), Firefox,
what a speaker plays (headless is muted: the worklet's samples are checked), and mobile networks.

## What remains for v1

| piece | notes | days [inferred] |
|---|---|---|
| iOS Safari | an `<audio>` element fed from the worklet (`MediaStreamAudioDestinationNode`) for lock-screen playback, tried on a device; Safari's WebCodecs set | 2 |
| More pages | artist top songs, the queue and playlists reordered by dragging, more settings | 2 |

Smaller notes:

- Measuring ahead: the next song is measured only if it fits what the playing one leaves of the loaders'
  32 MB (`memory_mb` 128): a FLAC after a FLAC mostly does not, and is measured as it plays instead (the
  native stream cache has no such limit). A song started mid-way, or still held from a recent play, is
  not measured as it comes. The playing song is measured twice the first time (as it comes and as it
  plays); MP4s are not measured as they come (their index may be at the end), nor Vorbis or Opus in
  `player.wasm` (decoded by the browser only).

- The worklet counts running dry while the session plays; a jump to another song shows as one, since the
  new song's first bytes come while the state is still Playing.
- The worker pulls 4096 frames at a time, keeping the worklet 0.5 s ahead (`AHEAD_MS` in `play.rs`). The
  engine's output tells the worklet at once what the engine does: a pause stops it where it is with a
  15 ms fade, keeping what it holds; a flush (seek, skip, another queue) drops what it holds, the frames
  given back to the ring (`Feed::rewind`) before anything new is pulled; the engine's fades run on the
  worklet's level. Its latency is what the worklet holds plus the AudioContext's, so the place heard is
  exact. Measured in the check: silent 15 ms after a pause (523 ms before), a seek heard after 23 ms
  (511 ms before).
- Opus after a seek in Chrome is placed by `later_seek_skip`; Firefox's decoder may differ (not measured).
- Covers are the server's `getCoverArt` addresses loaded by the page; nothing is cached beyond the
  browser's HTTP cache.
- Stalls: a pushed loader has no stall watch (`fetch` reports its own failures); a server that never
  answers keeps the song loading.

## Open risks

- **iOS Safari background audio**: an AudioContext is suspended when the page is hidden or the screen
  locks, unless audio comes from a media element. Lock-screen playback on iPhone likely needs the
  worklet's output routed into an `<audio>` element, and whether that keeps playing locked has to be tried
  on a device. Desktop browsers and Android Chrome keep a playing AudioContext alive.
- **Codecs**: Chrome has MP3, AAC, FLAC, Vorbis and Opus [measured]; Safari's set is smaller [inferred];
  ALAC nowhere [measured in Chrome].
- **OPFS sync access handles** exist only in Dedicated Workers (fine here) and allow one connection per file.
- **CORS**: the page must be served from the server's origin (octo-fiesta or a reverse proxy), as decided.

## The async remote and jam client

`crates/core/src/remote.rs` runs as tasks on `nori-tasks` (one native thread for all of them;
`spawn_local` and `setTimeout` on the web). The browser uses the shared host session for remote
playback and jams. `web/check/jam.mjs` exercises Play On in both directions and a host/guest jam
using separate browser profiles against the local relay.
