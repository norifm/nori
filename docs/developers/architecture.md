# Architecture

Applies to nori 0.7, every client.

nori is one Rust core with thin clients around it. Every decision lives once, in Rust, and is tested
there. The clients draw, hold the words and wire the operating system.

```
┌──────────── clients: drawing, words, OS hooks ─────────────────┐
│ Android (app/, core/)  desktop  terminal  iPod (ios/)  web     │
├──────────── nori-host: the session the other clients share ────┤
│ nori-engine: the player on its own thread, in bursts           │
├──────────── nori-core: a Core per profile, the Client ─────────┤
│ player · library · queue · lyrics · settings · devices         │
│ remote · covers · look · automix · transfers · db · net        │
│ model · tasks · time                                           │
└────────────────────────────────────────────────────────────────┘
```

## The layers

| layer | what it holds |
|---|---|
| domain crates | one area each: decoding and the sound chain (`player`), the library and its pages (`library`), the queue and autofill (`queue`), lyrics, settings, outputs and AutoEQ (`devices`), remote control and jams (`remote`), covers, colors from covers (`look`), and more |
| `nori-tasks`, `nori-time` | where async work runs and the one clock the core reads, natively and in a browser |
| `nori-core` | a `Core` per profile, the `Client`, and the [providers](providers.md) a profile plays from (a Subsonic server, YouTube Music). Re-exports every domain crate, so a client can link it alone |
| `nori-engine` | the whole player on its own thread: it loads in bursts, decodes, runs the sound chain and transitions, and sleeps in between. Every client plays through it |
| `nori-host` | the session the desktop, terminal, iPod and web clients share: the engine, queue saving, remote control and jams, the offline index, media controls. Android does not use it yet |

| client | language | where |
|---|---|---|
| Android | Kotlin, Compose | `app/` (UI), `core/` (media3 service, downloads, no UI), `crates/android` |
| desktop | Rust, Slint | `crates/desktop`, `crates/desktop-ui` |
| terminal | Rust, ratatui | `crates/cli` |
| iPod touch | Swift, UIKit | `ios/`, `crates/ios` |
| web (not released yet) | Svelte, TypeScript | `web/`, `crates/web` |

Every crate, with what it holds, is in [AGENTS.md](../../AGENTS.md#layout).

## The rules that shape it

- The core returns data and enums, never text a person reads. Every word is the client's.
- Calls across the boundary are coarse: one page or one answer per call. Per-buffer work goes through
  raw native doors that do not allocate.
- Every optional feature sits behind a setting and costs nothing when it is off.
- Speed, CPU, memory, wakeups and battery decide where work lives. See
  [Writing a client](writing-a-client.md).

## Docs

- User docs: [norifm.com/docs](https://norifm.com/docs/), kept in
  `norifm/norifm.com` (private).
- Developer docs: these pages, in `docs/developers/`.
