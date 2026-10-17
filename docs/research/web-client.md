# The web client: phase one spike

Internal planning note, not published. Issue #25.

Labels: **[measured]** = measured for this note, with how; **[verified]** = run in a browser (headless
Chrome over `web/serve.py` and the local Navidrome); **[inferred]** = reasoning or an estimate.

## What the spike proves

- **The core runs in a browser** [verified]: nori-core, nori-db (rusqlite on sqlite-wasm-rs) and the
  engine compile for `wasm32-unknown-unknown` and run in a Dedicated Worker. The databases live in OPFS
  through the sync-access-handle pool VFS (`sqlite-wasm-vfs` 0.2; `stored: true` in the check), with
  memory as the fallback.
- **Login and albums** [verified]: the page signs in to the local Navidrome through the core's own
  `login_check` and `Client::read_each(Read::AlbumList)` over a `fetch` Transport, lists 19 albums with
  covers (signed `getCoverArt` URLs from the core), opens an album (`Read::AlbumById`).
- **One song through the real engine** [verified]: the song is fetched whole, then played by the engine
  (`Engine::driven`): demux and decode (symphonia/nori-player), the sound chain and transition engine,
  the ring, `Feed::pull` into chunks sent to an AudioWorklet. A 10-minute FLAC played 10 s in headless
  Chrome with no underrun in the engine and no dry worklet quantum; the engine's status says Playing at
  the right position, the worklet's peak is the song's level.
- Not verified: Firefox and Safari; real speakers (headless runs muted, the samples are checked instead);
  a song longer than the check; reload persistence of OPFS (headless starts a new profile each run).

Timings, headless Chrome on this Mac, server on localhost, two runs [measured]:

| step | ms |
|---|---|
| core.wasm fetched, compiled, OPFS installed, databases opened | 196-345 |
| login (ping, token auth) | 18-55 |
| 24 newest albums (getAlbumList2, parsed, stored) | 15-41 |
| a song fetched whole and started (10-min test FLAC) | 106-167 |

## How it is built

- `crates/web` (nori-web): a `cdylib` for `wasm32-unknown-unknown` with wasm-bindgen. Natively it is an
  empty library, so `cargo test --workspace` and clippy are unchanged. It holds the core's `Transport` over
  the worker's `fetch` (`fetch.rs`), the `Nori` object the worker calls (`lib.rs`: open, login, albums,
  album, play, pause, turn, pull, status; every answer JSON), and the engine as the web runs it (`play.rs`).
- `web/`: plain TypeScript, HTML and CSS. `app.ts` is the page (it only draws and asks), `worker.ts` the
  Dedicated Worker holding core.wasm, `worklet.ts` the AudioWorklet. The worklet is joined to the worker by
  a `MessageChannel` port, so audio never passes through the page's thread; chunks of interleaved float
  are transferred (`postMessage(pcm, [pcm.buffer])`), no SharedArrayBuffer, so no COOP/COEP.
- `web/build.sh`: cargo (`--profile web`: release, `opt-level = "s"`, `panic = "abort"`), wasm-bindgen
  `--target web`, `wasm-opt -Oz`, `tsc`, and a size table. `web/serve.py`: the static files and `/rest/`
  forwarded to a Subsonic server on one origin (development only; any same-origin server works).

To run it: `tools/dev-server.sh`, `web/build.sh`, `python3 web/serve.py` (port 8080, API from
localhost:4533), open http://localhost:8080 and sign in as admin/admin.

`sqlite-wasm-vfs` 0.2 is the release that matches rusqlite 0.40's sqlite-wasm-rs 0.5; it brings
`tokio` (sync only) and `indexed_db_futures` along. 0.3 needs sqlite-wasm-rs 0.6, i.e. a newer rusqlite.

Toolchain: `wasm32-unknown-unknown` was already installed. Installed for the spike: `wasm-bindgen-cli
0.2.128` (the version in Cargo.lock) and `twiggy 0.8` (cargo install). Present already: `wasm-opt` 133,
`brotli`, node 26 (`npx` fetches `typescript@5.6.3`). SQLite's C needs a clang with a wasm32 backend:
Apple's clang has none and Homebrew LLVM is not installed here, but the Android NDK's clang (30.0) has
it; `web/build.sh` finds either.

## Shared core changes (every platform, behaviour kept, all tests green)

1. **One clock**: `crates/time` (nori-time) re-exports `Instant`/`SystemTime`/`Duration` from std natively
   and from `web-time` on the web (`performance.now()`, `Date.now()`), plus `since_epoch()`/`unix_ms()`.
   Every `std::time::Instant`/`SystemTime::now` in the core crates (player, model, db, settings, queue,
   lyrics, library, transfers, core, engine, remote, host) goes through it. parking_lot's `wait_until`
   takes std's `Instant`, so those four waits became `wait_for(until - now)`, the same wait. Not done: a
   virtual-clock hook in it. The engine already has its own `Clock` trait (tests move it); the other
   clocks (remote timing, freshness windows) would need the hook to be tested on a virtual clock, which
   is a global and so wants its own decision.
2. **Host takes the platform's HTTP as a trait**: `nori_host::session::Net` (`Transport + ByteSource`,
   implemented for anything that is both). nori-http (ureq, rustls, ring) is now only the clients' and
   host's tests' dependency; cli, desktop and the iPod pass `nori_http::Http::new()` as before, so their
   builds are unchanged without a feature. `device_name` (gethostname) and its libc are unix only. Host
   compiles for wasm32 (`cargo check --target wasm32-unknown-unknown -p nori-host
   --no-default-features`), but does not run there: it is built on blocking threads (below).
3. **Signalsmith Stretch** (C++) is a native-only dependency; on wasm32 AutoMix's tempo changes take the
   varispeed path (`Engine::Vari` in `automix/stretch.rs`, pitch moves). Speed/pitch on the web waits for
   the pure Rust port being done in parallel. (User-facing speed/pitch is Sonic, pure Rust, and works.)
4. **The engine on its caller's thread**: `Engine::driven(...) -> (Engine, Driven)`. The engine thread's
   body was already one `Worker::turn()` returning how long to sleep; the thread now runs `Worker::step()`
   in a loop, and `Driven::turn()` runs one step for a caller with no threads. Same commands, same
   `Engine` handle. Tested on the virtual clock (`tests/driven.rs`: a song heard sample-exact, whole).
5. **`Source::Memory`**: a song whose bytes are all in memory, demuxed and decoded from a `Cursor` by the
   same `Demuxed::open` a file uses. The web fetches the song whole for now (below).
6. Single-threaded fallbacks on wasm32 only: the database's background writer runs a job at once, and the
   planner finishes an analysis at once instead of on its thread.

## Measured sizes

`web/build.sh` (`--profile web`, wasm-bindgen 0.2.128, `wasm-opt -Oz`, `brotli -q 11`) [measured]:

| file | raw | brotli |
|---|---|---|
| core.wasm, as built (core, db, engine, decoders) | 3 225 221 | 1 102 048 |
| core.wasm without the engine (`NORI_WEB_CORE_ONLY=1`: core and db) | 1 572 757 | 551 677 |
| so the engine and the decoders | about 1 652 000 | about 550 000 |
| core.js (wasm-bindgen glue) | 32 799 | 6 252 |
| worker.js | 3 264 | 978 |
| worklet.js | 1 972 | 660 |
| app.js + index.html + style.css (the shell) | 6 588 | 2 155 |

Against the targets: the shell is far under 50 KB; core is 540 KB brotli against ~300 KB for core and
~350 KB for db, so core+db together are within the two targets' sum (650 KB); the audio module is about
540 KB brotli.

Biggest contributors (twiggy over the unstripped build before wasm-opt, 3.55 MB of code and data):

| what | bytes | share |
|---|---|---|
| SQLite (C) | ~826 000 | 23 % |
| Rust's core/alloc/std (formatting, collections, generic code) | ~570 000 | 16 % |
| .rodata (strings, tables: SQLite's, the codecs', the settings') | 369 000 | 10 % |
| symphonia: formats (MP4 127 K, Ogg 40 K, RIFF 14 K, metadata 36 K, core 80 K) | ~300 000 | 8 % |
| symphonia codecs (MP3 44 K, AAC 42 K, FLAC 31 K, Vorbis 29 K, ALAC 11 K, common 29 K) | ~190 000 | 5 % |
| nori-player (sound chain, transitions, AutoMix analysis) | 224 000 | 6 % |
| serde_json (the Subsonic answers, the settings) | 201 000 | 6 % |
| rustfft (AutoMix's analysis) | 156 000 | 4 % |
| nori-engine | 139 000 | 4 % |
| opus-rs | 126 000 | 4 % |
| nori-settings | 73 000 | 2 % |
| rusqlite, regex-lite, ebur128 | 82 000 | 2 % |

What would shrink it [inferred]: WebCodecs takes the codecs and opus-rs out of the first audio load
(~320 K raw, ~100 K br); a hand-written decoder for the Subsonic answers instead of serde_json's derive
on every model struct; SQLite compiled with `SQLITE_OMIT_*` options for what the core never uses (it
uses FTS5, so not that); the settings' derive generates per-field code that a table would not.

## What blocks the full engine and the host, and the restructuring

The engine runs about ten threads natively. On the web, after this spike:

- **Done**: the engine thread itself (`Driven`, stepped by the worker). In the browser the worker turns
  it after every command, after every chunk pulled for the worklet, and when the timer it asked for
  fires (`setTimeout`). With `AudioOutput::bursts()` false the engine wakes on its own timer.
- **Song bytes (`Loader`)**: one blocking thread per song reading a `ByteSource` body. Proposed: split
  `Loader::run`'s loop into its decisions (what range to open next, bytes arrived, ended, failed, rest
  before retrying) and its I/O; natively a thread does the I/O as now, on the web the worker's `fetch`
  reader pushes each chunk (`ReadableStream`) and asks the loader what to open next. The window (low,
  high, cap) logic stays one implementation. Until then the web uses `Source::Memory` (whole song, then
  play: about a second for a 5 MB MP3 on a LAN, too slow on mobile data).
- **Opening while bytes arrive** (`Demuxed::load`, the `nori-open` thread): symphonia's readers block on
  `Read`. With the push loader the open is retried on the engine's turn when more bytes arrived
  (`Reading::ready()` already lets the pipeline wait), or the probe is made incremental for MP3/FLAC/Ogg,
  whose headers are at the start; MP4 already waits for the whole song.
- **The stall watch, the measurer, fetch ahead, the stream cache (`Store`, std::fs)**: the stall watch
  becomes a timer the worker owns; measuring ahead runs in turns between the engine's (or in a second
  worker); the stream cache is out of v1 (no offline), or later on OPFS.
- **Decoding by WebCodecs**: the engine demuxes already; `Demuxed::open_packets` gives packets and the
  codec config (the offload path). A `Reading` that hands packets to an `AudioDecoder` in the worker and
  takes back `AudioData` (copied out as f32, interleaved) keeps the chain, mixer and ring as they are;
  `ready()` is false until decoded audio is there. Saves the decoders' code (below) at the cost of an
  async hop per packet. ALAC (no WebCodecs codec string) and browsers without AudioDecoder take the Rust
  decoders, as a lazily loaded module. The spike decodes in Rust (symphonia, opus-rs), which works.
- **Host**: 32 `block_on`/`spawn` sites in `session.rs`; each is "run this network call off the UI
  thread and report through `Out`". On the web that is `wasm_bindgen_futures::spawn_local` of the same
  future. Proposed: host takes `nori_tasks::Tasks` (spawn a future, sleep; below), natively a thread per
  task with `block_on` as now or the remote's `Native`, on the web `Local` (`spawn_local`, `setTimeout`). The rest of host (queue saves, the
  index sync, jams' rules) is then shared with the web as the brief wants.

## The async remote and jam client (done, branch `async-remote`)

`crates/core/src/remote.rs` ran on threads: a sender (`nori-remote-out`), the relay poller (`RETRY_MS`
rests on a `Condvar`), one poller per LAN peer, the clock keeper (`timing` condvar) and the foreseen-state
timer, with `Remote::get` a `block_on` whose waker sat in `Inner.waiting`. Now one implementation for all:

- `nori-tasks` (crates/tasks) holds the `Tasks` trait: `spawn(Task)` and `sleep(Duration) -> Sleep`, both
  boxed `Send` futures. `Native` is one thread per executor that parks until a task is woken or its next
  timer is due; `Local` (wasm32) is `spawn_local` and `setTimeout`. With it `Notify` (a notice reaches
  every `notified()` made before it, so make, check, await misses nothing) and `first(a, b)`.
- each thread body is an `async fn` with the same loop; `Condvar::wait_for(d)` is `first(sleep(d),
  notified())`; `Remote::get` races the transport's future with a `given_up` loop on the `cancel` notify;
  the sender is one task over a queue (order kept); `Remote::stop` waits (natively, `block_on`) for the
  tasks and the queued sends, up to 3 s.
- `Remote::new` makes its own `Native` (`Local` on wasm), so the uniffi surface and Kotlin are unchanged;
  `Remote::with_tasks` takes any `Tasks` (the tests hand one whose timers run on a virtual clock).
- the LAN door (`nori_remote::lan::Door`) is native only; signing a request to another door stays shared.
- a transport future must not block its thread (all three platform transports answer from their own
  threads or the event loop); the in-memory relay of tests/remote.rs waits off-thread to match.

Host's `Tasks` should be this trait, so the session and the remote share one executor per platform.

## Plan to v1, with estimates [inferred]

| piece | days |
|---|---|
| Push loader and incremental open (play while it downloads, seek by range) | 3 |
| Host on `Tasks`, the web on host's session (queue saves, sync, errors, scrobbles) | 3 |
| WebCodecs `Reading`, Rust decoders as a lazy module | 3 |
| Split loading: shell, core.wasm, audio module on first Play (two wasm modules or one with lazy init) | 2-3 |
| Library, search, album/artist/playlist pages, queue UI (TypeScript, Apple Music shapes) | 6-8 |
| Synced lyrics (core has them; worker clock to the page each frame while visible) | 2 |
| AutoMix on the web (analysis between turns or a second worker; plans already shared) | 2 |
| Async remote and jams, host and guest | done |
| Speed/pitch with the Rust Stretch port | 1 once the port lands |
| Media Session API, OPFS settings and profiles, service worker for the static files | 2 |
| Headless browser checks in CI (the spike's script) | 1 |

About five to six weeks of one person.

## Open risks

- **iOS Safari background audio**: an AudioContext is suspended when the page is hidden or the screen
  locks, unless audio comes from a media element. Lock screen playback on iPhone likely needs the
  worklet's output routed into an `<audio>` element (`MediaStreamAudioDestinationNode` to `srcObject`),
  and whether that keeps playing locked has to be tried on a device. Desktop browsers and Android Chrome
  keep a playing AudioContext alive.
- **Codecs**: WebCodecs AudioDecoder: Chrome/Edge and Firefox have AAC, MP3, FLAC, Opus, Vorbis; Safari's
  set is smaller and newer (AAC, MP3, FLAC, Opus [inferred, to verify]); ALAC nowhere. The Rust decoders
  stay needed as the fallback, which is why they are a separate lazy module rather than gone.
- **Size**: SQLite is the largest single piece and is needed before the library shows anything stored.
  Splitting it into its own module needs a database interface across modules (it is called synchronously
  everywhere today), so the realistic first split is core+db and audio, not core, db and audio.
- **OPFS sync access handles** exist only in Dedicated Workers (fine here) and allow one connection per
  file: a second tab of the client cannot open the same databases. Needs a lock (Web Locks) and a "the
  app is open in another tab" state.
- **Panics abort the worker** (`panic = "abort"`; wasm has no unwinding on stable): the engine's
  catch-and-recover does not run on the web. The worker must be restartable from the page with the queue
  restored from the database.
- **Memory**: whole songs in memory until the push loader lands; a lossless album would be hundreds of MB.
- **CORS**: the page must be served from the server's origin (octo-fiesta or a reverse proxy), as
  decided; covers and streams use the same signed URLs, so caches hit.
