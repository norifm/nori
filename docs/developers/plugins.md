# Plugins

Applies to nori 0.7, the Android, desktop and terminal clients. The iPod and the web have no plugins.

A plugin is a WebAssembly component built against the `nori:plugin` API in `wit/`. It runs sandboxed in
the plugin host (`crates/plugins`): it reaches only what the host links for it, only the hosts its
manifest names, and only the nori values its manifest says it overrides. The plan, with what is not
built yet, is `docs/research/plugins.md`.

| piece | where |
|---|---|
| the API (WIT) | `wit/`: `types.wit`, `host.wit` and `net.wit` (what a plugin calls), `plugin.wit` (what it exports, and the `plugin` world) |
| the SDK | `crates/plugin-sdk` |
| the host | `crates/plugins` |
| plugins' sound stages in the chain | `crates/player/src/stage.rs` |
| overrides laid over the settings | `crates/settings/src/overrides.rs` |
| plugins' providers | `crates/core/src/providers/plugin.rs`, through `Kinds` |
| nori's own plugins | [nori-plugins](https://github.com/norifm/nori-plugins): orbit (a sound stage), playback (speed and pitch as overrides), youtube-music (a provider) |

## Writing one

```rust
use nori_plugin_sdk::{state, Manifest, Row, Slot, SoundStage};

#[derive(Default)]
struct Orbit { /* ... */ }

impl SoundStage for Orbit {
    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> { /* ... */ samples }
    // configure, reset, save, restore ...
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    rows: &[Row::level("speed", 0.02, 1.0, 0.01, 0.12)],
    ..Manifest::new("fm.nori.orbit")
};

nori_plugin_sdk::plugin!(Orbit, MANIFEST, sound_stage);
```

- Each exported interface of the `plugin` world is a trait of the SDK, written from `wit/` by its
  build.rs. Every method answers as nori would on its own: `None` (nori's answer stands), "unsupported",
  empty, or, for a call handed what it answers (a stage's samples), what it was handed. An export
  answering a record nori can't make up is an `option<...>` in the WIT; build.rs refuses one that isn't.
- `plugin!(Type, MANIFEST, points...)` exports `Type` for the points named; the others answer as nori.
- The manifest is Rust. `plugin!` encodes it at compile time into the wasm's `nori:plugin` custom
  section, with the crate's version and the points named, and nori reads it before running anything.
  What the plugin calls (HTTP, settings) isn't declared: nori reads it from the component's imports.
  The manifest says only what code can't: its id, the hosts it may reach, the keys it overrides, its
  rows and a stage's slot.
- Words are files: `strings/<lang>.toml`, key = text, English required: `name`, `about`, each row's id
  and each choice option's key.
- Build with `cargo build --release --target wasm32-wasip2`; a `.noriplugin` is a zip of `plugin.wasm`,
  `strings/` and an optional `icon.svg` (`tools/pack.py` in nori-plugins). A plugin's `cdylib` doesn't
  link for the host target (its export names are wasm's); its tests do.

## The host

- **Nothing without a plugin.** With none enabled, opening the host reads `plugins/index.json` and nothing
  else: no engine, no compiling, no thread. The settings' layer is empty, so the engine's settings are
  nori's own.
- **One engine, compiled once.** wasmtime with Cranelift, made at the first plugin needed. Each plugin is
  compiled at install and kept as `compiled.cwasm` beside it, keyed by its wasm's hash and the engine's;
  never downloaded as native code.
- **Install checks first.** A package is unpacked beside the old version and refused when its manifest
  can't be read, its API version is not this host's or the one before, its id isn't reverse DNS, it
  overrides a key nori doesn't have, or it imports calls of an interface this host doesn't link
  (`runtime::LINKED`; today `log`, `store`, `http`, `settings`, `rows`, `notices`). Its store survives an
  update.
- **Its own thread.** A plugin's own instance (lifecycle, hooks, provider) runs on a thread made at its
  first call ([`Worker`](../../crates/plugins/src/worker.rs)): its calls may block (its HTTP waits on the
  core's transport there), and the core's executors never wait on one. Each call has a fuel budget; a
  trap or a spent budget fails that call and the next starts a fresh instance.
- **Its own store**, capped (1 MB): its keys, and under `nori/` what the host keeps for it (its rows'
  values, its overrides), which it can't write.
- **HTTP** goes through the open profile's `Transport` (the one connection pool), only to its `net`
  hosts. The transport sends GET, or POST with a JSON or form body; a response carries its status, body
  and `set-cookie` values.

## Overrides

A plugin changes nori's values by key, never by writing the settings: the user's settings stay as they
are, and turning the plugin off brings nori's own values back.

- A key is a setting's name (`crossfadeSec`), or a tunable's: a value with no setting
  (`playback.speed`, `playback.pitch`, `silence.skip`, `queue.previousAlwaysSkips`; `Tunables`).
- The host keeps a plugin's overrides in its store and lays the enabled plugins' over the settings in
  their order (`Layer`, held by the app's `Settings`); a later plugin's value of a key wins. A setting's
  override goes through that setting's own codec and range.
- The layer changes when a plugin overrides or clears a key, a row changes, or a plugin is turned on or
  off, moved or removed; the client then derives the engine's settings again
  (`nori_engine::core::settings(stored, layer)`), as for a settings change. Nothing is read per buffer.

## Sound stages

A stage runs in the chain after the equalizer and before nori's ceiling (a soft limit above -1 dBFS
that keeps whatever a plugin boosts from clipping), ahead of silence skipping and speed. A stage
counts as touching samples: songs stay off offload, and bit-perfect output drops it.

- **Its own instance** on the engine's thread, with every import linked to a trap: it can't wait on
  anything. A block (8192 frames or fewer) goes in as one `list<f32>`, channel after channel, and comes
  back the same length. The SDK exports `process` and `save` by hand without a post-return, so the host
  reads the answer in place: no host allocation per block (tests/host.rs counts them).
- **State is kept** like the chain's own: at every mark the host saves the stage (`save`: its inputs and
  the plugin's state bytes; the SDK's `state` module does it for a plain `Copy` struct) and a splice
  restores it, so the music made again is the same, bit for bit
  (`a_stage_made_again_mid_song_sounds_the_same` in the player's pipeline tests).
- **Inputs** are its numeric rows, in manifest order; a row changed is a sound change, spliced in.
- **A trap or a block over its fuel** bypasses the stage until the next reset (a seek, another song),
  which makes a fresh instance. Audio never waits for a plugin.

## Providers

A plugin with a `provider` export adds a kind of profile: `ServerKind::Plugin { id }`, stored as its id.
A client asks a `Kinds` (`Kinds::with_plugins`, or `Kinds::builtin` without plugins) for the kinds it
offers, their forms and sign-in, and hands it to each `Core` it opens (`Core::use_kinds`) and its
transport (`Kinds::set_transport`). See [Providers](providers.md).

## Tests

- `cargo test -p nori-plugins`: the manifest as the SDK writes it, the extension points' order against
  `wit/`, the store, and `tests/host.rs` over a real plugin (`fixture/`, built to wasm by the `testing` feature:
  needs `rustup target add wasm32-wasip2`): its stage and kept state, no allocation per block, its
  overrides, its HTTP refused outside its hosts, and a plugin needing an unlinked interface refused.
- `cargo test -p nori-player --test pipeline stage`: stages in the chain on the virtual clock.
- `cargo test -p nori-settings overrides`: tunables and settings overridden without being written.
