Based on symphonia-bundle-mp3 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `layer3/codebooks.rs`, `layer3/requantize.rs`: Huffman codes are looked up eight bits at a time
  in tables whose entries also say how many sign bits follow, and read from a 64-bit cache filled
  eight bytes at a time. A read past the end of the main data fails once the granule is read rather
  than at that read, with the same error.
- `common.rs`: the frame header's bitrate is read by layer 2 only, which nori does not build.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test --release -p nori-engine --test engine decoders::digest
-- --ignored --nocapture` prints a digest of every MP3 shape it decodes, to compare a change with.
