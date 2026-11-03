Based on symphonia-bundle-flac 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `bits.rs`: subframes are read through a bit reader that fills a 64-bit cache eight bytes at a
  time and reads a Rice partition in one loop. It fails where symphonia_core's `BitReaderLtr` fails.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test -p nori-engine --test engine decoders` checks every
FLAC shape there decodes to what was encoded.
