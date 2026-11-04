//! Time to stretch a minute of stereo music the way AutoMix drives it (256-frame blocks), the port against
//! the C++ original. Not a check.
//!
//!   cargo test --release -p nori-stretch --test perf -- --ignored --nocapture

use std::time::Instant;

const RATE: u32 = 44_100;
const BLOCK: usize = 256;

/// A minute of a chord over bass and noise, interleaved stereo.
fn music() -> Vec<f32> {
    let mut seed = 11u64;
    let n = 60 * RATE as usize;
    (0..n)
        .flat_map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = ((seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 0.05;
            let t = i as f32 / RATE as f32;
            let root = [110.0, 87.31, 130.81, 98.0][(t * 2.0) as usize % 4];
            let tone = |m: f32, c: f32| (std::f32::consts::TAU * root * m * t + c).sin();
            [0.3 * tone(1.0, 0.0) + 0.1 * tone(5.04, 0.0) + noise, 0.3 * tone(1.0, 0.5) + 0.1 * tone(6.0, 0.5) + noise]
        })
        .collect()
}

/// Milliseconds to run `x` through `process` at `ratio` input frames per output frame.
fn time(x: &[f32], ratio: f64, mut process: impl FnMut(&[f32], &mut [f32])) -> f64 {
    let mut out = vec![0f32; (BLOCK * 4 + 8) * 2];
    let mut frac = 0.0;
    let start = Instant::now();
    for block in x.chunks(BLOCK * 2) {
        let want = (block.len() / 2) as f64 / ratio + frac;
        let m = want.floor() as usize;
        frac = want - m as f64;
        process(block, &mut out[..m * 2]);
    }
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore]
fn stretch_a_minute() {
    let x = music();
    for ratio in [1.071, 0.94] {
        let mut port = nori_stretch::Stretch::preset_cheaper(2, RATE as f32);
        let mut cpp = signalsmith_stretch::Stretch::preset_cheaper(2, RATE);
        let ours = time(&x, ratio, |i, o| port.process(i, o));
        let theirs = time(&x, ratio, |i, o| cpp.process(i, o));
        println!("perf: stretch at {ratio}: port {ours:7.1} ms, C++ {theirs:7.1} ms per minute");
    }
}
