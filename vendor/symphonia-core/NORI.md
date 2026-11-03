Based on symphonia-core 0.6.1 from crates.io. Its license is in LICENSE.

Changes marked NORI keep every decoded sample the same, bit for bit:

- `checksum/crc16.rs`: the CRC-16 a FLAC frame is checked with reads sixteen bytes a step, from
  tables built at compile time.
- `audio/conv.rs`: a 32-bit integer sample becomes a float in single precision. Scaling by a power
  of two is exact, so it rounds as the double precision it replaces did (checked for every i32).

The crate stays out of the workspace (`exclude` in the root Cargo.toml) and replaces the crates.io
one through `[patch.crates-io]`. `cargo test -p nori-engine --test engine decoders` decodes FLAC and
MP3 as the engine does; `decoders::digest` prints a digest of the output to compare a change with.
