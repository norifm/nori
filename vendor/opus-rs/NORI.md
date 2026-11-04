Based on opus-rs 0.1.34 from crates.io, its library only: the tests, benches and examples it ships are
left out, and with them their targets and dev-dependencies in Cargo.toml. Its license is in COPYING.

Changes marked NORI keep every decoded sample the same, bit for bit, and the public API as it was:

- `pvq.rs`: a pulse vector's index (`cwrsi`) is decoded reading the codebook table's rows directly
  when the table holds every entry the decoding can reach, as it does for each codebook CELT uses.
  The spreading rotation with a stride of one (`exp_rotation1_scalar`) carries the sample each step
  hands the next in a register rather than through memory.
- `celt.rs`: the de-emphasis runs once both channels of a frame are synthesized, the two filters in
  one loop, and without its clamp when the frame's samples cannot reach the clamp's limit. The
  post-filter (`comb_filter_inplace`) makes eight samples a step: each reads only samples at least
  13 before it, which the steps before have made.
- `lib.rs`: a stereo CELT frame is interleaved into the output a pair of samples at a time.

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test --release -p nori-engine --test engine decoders::opus_digest
-- --ignored --nocapture` prints a digest of every Opus shape it decodes, to compare a change with.
