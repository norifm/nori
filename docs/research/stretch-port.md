# Signalsmith Stretch in Rust (crates/stretch)

AutoMix's tempo changes with pitch kept (`crates/player/src/automix/stretch.rs`) ran on Signalsmith Stretch
through the `signalsmith-stretch` 0.1.3 crate, which builds the C++ with `cc` and `bindgen`. That was the only
C++ in nori. `crates/stretch` (`nori-stretch`) is a port to Rust, so the audio path builds without a C++
toolchain (Android, iPod, desktop, and a future wasm build). The C++ crate stays only as a dev-dependency of
`nori-stretch`, for the comparison tests.

## What was ported

From Signalsmith Stretch 1.3.1 (`signalsmith-stretch.h`) and Signalsmith Linear (`stft.h`, the modified real FFT
from `fft.h`), what nori calls:

- `presetCheaper` (100 ms blocks, 40 ms interval, split computation), `reset`, `inputLatency`/`outputLatency`,
  `process` and `flush`;
- `setTransposeSemitones` with no tonality limit (nori does not use it yet; the comparison runs it, and it
  exercises the peak finding and frequency map);
- the silence path, the time-factor phase randomization above 2x, and the step scheduler that spreads one
  block's work over the interval that reads its output.

Not ported: `presetDefault`, `configure` with other sizes, `seek`, `exact`, formants, custom frequency maps, the
Accelerate/IPP backends. The complex FFT is rustfft's (`N/2` points, 2560 at 44.1 and 48 kHz), with Signalsmith's
half-bin pre- and post-twiddles around it. Buffers are interleaved, as the old wrapper took them.

Differences from the C++:

- The random engine is `minstd_rand0` seeded with 1; the original seeds it from `std::random_device`, so its
  output at stretches past 2x changes from run to run. At AutoMix's slowest ratio (0.5) a few blocks get there.
- No `unsafe`. Nothing allocates after construction (`tests/no_alloc.rs`); the C++ only kept within capacity.
- `nori-player` boxes the stretcher: inline it would make every `Engine` as large as itself.

## How close it is

The two cannot match sample for sample over long runs, and neither can the C++ against itself. The phase
vocoder feeds each block's output phases into the next block and decides peaks, the loudest channel and the
"prediction too weak" fallback by comparisons, so rounding differences grow. Two sources differ:

- the FFT (rustfft against Signalsmith's own), about 1e-6 relative per transform;
- the C++ compiler fusing multiply-adds (clang's default `-ffp-contract=on` on arm64), which Rust never does.

The C++ against itself, with every input sample nudged by one ulp, departs by 1e-3 of the peak within about
10 000 frames on music and up to 1e-2 within the first interval on noise fading to silence.

**Bit-exactness check (out of tree, 2026-10-17).** Both sides were given the same exact FFT (a double-precision
DFT in a patched copy of `stft.h` compiled with `clang++ -O2 -ffp-contract=off`; an f64 rustfft in the port) and
run on the twelve excerpts where the port differed most from the crate. Ten were bit-identical, the other two
within 4e-8 of the peak. With the crate's C++ rebuilt with `CXXFLAGS=-ffp-contract=off` (and each side using its
own FFT), a case that had differed by 1.5e-4 differed by 1e-6. The remaining differences are rounding, not the
algorithm.

**Committed tests (`crates/stretch/tests/compare.rs`)** against the crate as it builds, at 1.25, 0.8, 1.03,
0.9985, 0.6, 1.9 and 2.0 times, and at +2 st, 1.25x with +2 st and 0.8x with -5 st, mono and stereo, on sines,
an exponential chirp (100 Hz to 8 kHz), noise bursts with silent gaps, and a synthetic song (bass, chords, kick,
hats):

| check | tolerance | result (arm64 Mac) |
|---|---|---|
| 8192-frame excerpts from a reset, through the flush: difference RMS over signal RMS, 640 runs | median < 1e-5, 90 % < 2e-4, each < 0.05 | median 1.3e-6, 90 % 2.1e-5, largest 2.5e-2 (noise fading out at 0.6x) |
| 4 s runs: per-window, per-band level difference | at most 2x the C++'s own spread under the one-ulp nudge + 0.02 dB | at most 0.53 dB (chirp at 0.6x, where the C++ is 0.46 dB from itself); 0.000 dB on most |
| 4 s runs: level | 0.05 dB | at most 0.0001 dB |
| 0.5x (random phases on both sides) | 1 dB spectral, 0.3 dB level | passes |
| latencies at 8, 22.05, 44.1, 48, 96 kHz, 1/2/6 channels | equal | equal |
| after `reset`, the same as a fresh stretcher | 1e-5 of the peak | passes |
| identical stereo channels: left is the mono output | exact | exact; right within 3e-4, as the C++ |

The median is float rounding; the tail is where the original itself moves as much under its one-ulp nudge. A
wrong conjugate in one of the four phase predictions fails three of these tests (checked by mutation).

## Performance

`cargo run --release -p nori-stretch --example speed`: 60 s of stereo 44.1 kHz in AutoMix's 256-frame blocks,
best of five, release profile (fat LTO), Apple Silicon Mac, same session:

| setting | Rust | C++ | Rust / C++ |
|---|---:|---:|---:|
| 1.25x | 238 ms | 226 ms | 1.05 |
| 1.25x, +2 semitones | 267 ms | 253 ms | 1.06 |
| 0.8x | 375 ms | 355 ms | 1.06 |

About 0.4 % to 0.6 % of one core for real-time stereo. Not measured on a phone yet.

## wasm

`cargo build -p nori-stretch --target wasm32-unknown-unknown` builds (debug and release). rustfft's
`wasm_simd` feature is not turned on, so the FFT is scalar there.
