Based on symphonia-codec-alac 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `bits.rs`: a packet is read through a bit reader that fills a 64-bit cache eight bytes at a time,
  and reads a channel's residuals in one loop, each adaptive Golomb code from the cache in one step.
  It fails where symphonia_core's `BitReaderLtr` fails.
- `lib.rs`: in mode 0 with orders 1 to 8, those encoders choose, each sample is predicted as its
  residual is read, in the same loop (`decode_order`), so that reading and predicting overlap. The
  predictor per order keeps the coefficients and the last sample in registers and adapts the
  coefficients without branching on the residual. Other modes and orders read all residuals, then
  predict as symphonia does.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test -p nori-engine --test engine decoders` checks every ALAC
shape there decodes to what was encoded; `decoders::digest_alac` prints a digest of each.
