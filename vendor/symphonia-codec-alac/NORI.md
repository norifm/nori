Based on symphonia-codec-alac 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `bits.rs`: a packet is read through a bit reader that fills a 64-bit cache eight bytes at a time,
  and reads a channel's residuals in one loop, each adaptive Golomb code from the cache in one step.
  It fails where symphonia_core's `BitReaderLtr` fails.
- `lib.rs`: prediction of orders 1 to 8, those encoders choose, has a predictor per order
  (`predict_order`), which keeps the coefficients and the past samples in registers and adapts the
  coefficients without branching on the residual.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test -p nori-engine --test engine decoders` checks every ALAC
shape there decodes to what was encoded; `decoders::digest_alac` prints a digest of each.
