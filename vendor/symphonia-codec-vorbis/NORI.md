Based on symphonia-codec-vorbis 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `bits.rs`, `residue.rs`, `codebook.rs`: residues are read through a bit reader that fills a
  64-bit cache eight bytes at a time, and looks a code of up to 10 bits up in one step, in a table
  each codebook builds from its codewords. A longer code, or one near the packet's end, is read by
  symphonia_core's `BitReaderRtl` from the same position, so a read fails where it failed. The
  packet's reader then steps over the bits the residue read.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test --release -p nori-engine --test engine
decoders::digest_vorbis -- --ignored --nocapture` prints a digest of every Vorbis shape it decodes,
to compare a change with.
