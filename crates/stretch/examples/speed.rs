//! CPU time of the port against the C++ original: 60 s of stereo 44.1 kHz through each, in AutoMix's 256-frame
//! blocks, best of five. `cargo run --release -p nori-stretch --example speed`

use std::time::{Duration, Instant};

const RATE: usize = 44_100;
const CH: usize = 2;

fn input() -> Vec<f32> {
    let mut seed = 1u32;
    (0..RATE * 60)
        .flat_map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let t = i as f32 / RATE as f32;
            let tau = std::f32::consts::TAU;
            let noise = ((seed >> 9) as f32 / (1 << 23) as f32 - 0.5) * (-(t % 0.5) * 30.0).exp();
            let tone = 0.2 * (tau * 110.0 * t).sin() + 0.1 * (tau * 440.0 * t).sin() + 0.05 * (tau * 1320.0 * t).sin();
            [tone + 0.2 * noise, tone - 0.2 * noise]
        })
        .collect()
}

fn time(x: &[f32], ratio: f64, mut process: impl FnMut(&[f32], &mut [f32])) -> Duration {
    let mut out = vec![0f32; 1024 * CH];
    let mut frac = 0.0;
    let start = Instant::now();
    for block in x.chunks(256 * CH) {
        let want = (block.len() / CH) as f64 / ratio + frac;
        let m = want.floor() as usize;
        frac = want - m as f64;
        process(block, &mut out[..m * CH]);
    }
    start.elapsed()
}

fn main() {
    let x = input();
    for ratio in [1.25, 0.8] {
        let (mut rust, mut cpp) = (Duration::MAX, Duration::MAX);
        for _ in 0..5 {
            let mut r = nori_stretch::Stretch::preset_cheaper(CH, RATE as f32);
            let mut c = signalsmith_stretch::Stretch::preset_cheaper(CH as u32, RATE as u32);
            rust = rust.min(time(&x, ratio, |i, o| r.process(i, o)));
            cpp = cpp.min(time(&x, ratio, |i, o| c.process(i, o)));
        }
        println!(
            "60 s stereo at {ratio}x: Rust {:.0} ms, C++ {:.0} ms, Rust/C++ {:.2}",
            rust.as_secs_f64() * 1000.0,
            cpp.as_secs_f64() * 1000.0,
            rust.as_secs_f64() / cpp.as_secs_f64()
        );
    }
}
