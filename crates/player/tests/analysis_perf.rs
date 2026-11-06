//! Time to analyze a minute of stereo music for AutoMix, as it is fed while it plays (256-frame blocks of
//! 16-bit). Not a check. `NORI_DUMP=file` writes the features, to check a change keeps them exactly.
//!
//!   cargo test --release -p nori-player --test analysis_perf -- --ignored --nocapture

use std::time::Instant;

use nori_player::automix::analysis::Analyzer;

const RATE: u32 = 44_100;

/// A minute of bass, a chord, a kick and noise, interleaved stereo 16-bit.
fn music() -> Vec<i16> {
    let mut seed = 5u64;
    (0..60 * RATE as usize)
        .flat_map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = ((seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 0.05;
            let t = i as f32 / RATE as f32;
            let root = [110.0, 87.31, 130.81, 98.0][(t * 2.0) as usize % 4];
            let beat = t % 0.5;
            let kick = 0.5 * (-beat * 30.0).exp() * (std::f32::consts::TAU * 60.0 * beat).sin();
            let tone = |m: f32, c: f32| (std::f32::consts::TAU * root * m * t + c).sin();
            let (l, r) = (0.2 * tone(1.0, 0.0) + 0.1 * tone(5.04, 0.0) + kick + noise, 0.2 * tone(1.0, 0.5) + 0.1 * tone(6.0, 0.5) + kick + noise);
            [(l * 20_000.0) as i16, (r * 20_000.0) as i16]
        })
        .collect()
}

#[test]
#[ignore]
fn analyze_a_minute() {
    let x = music();
    let mut a = Analyzer::new(RATE, 60_000);
    let start = Instant::now();
    for block in x.chunks(512) {
        a.feed_interleaved(block, 2, |s: i16| s as f32 / 32768.0);
    }
    let fed = start.elapsed().as_secs_f64() * 1000.0;
    let f = a.take_features();
    println!("perf: analysis fed {fed:6.1} ms per minute");
    if let Ok(path) = std::env::var("NORI_DUMP") {
        let curves = [&f.onset, &f.low_onset, &f.power, &f.low_power, &f.vocal, &f.centroid, &f.blocks_k, &f.blocks_raw, &f.voice];
        let mut out: Vec<String> = curves.iter().flat_map(|c| c.iter().map(|v| v.to_bits().to_string())).collect();
        out.extend(f.chroma.iter().flatten().map(|v| v.to_bits().to_string()));
        out.extend(f.pitch.iter().map(|v| v.to_bits().to_string()));
        out.push(format!("{} {}", f.tuning_cs.0.to_bits(), f.tuning_cs.1.to_bits()));
        std::fs::write(path, out.join("\n")).unwrap();
    }
}
