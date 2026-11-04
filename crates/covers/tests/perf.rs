//! Host perf of a cover: decoded for a grid tile and for the full player, then its page colours. Not a
//! check; prints one "perf:" line per step. Needs ffmpeg for the large covers.
//!
//!   cargo test --release -p nori-covers --test perf -- --ignored --nocapture

use std::time::Instant;

use nori_covers::decode::Decoder;
use nori_covers::scale::Alpha;

/// A `side` square cover from ffmpeg's test pattern in `format` (its extension); None without ffmpeg.
fn cover(side: u32, format: &str) -> Option<Vec<u8>> {
    let dir = nori_testdir::TempDir::new("covers-perf");
    let out = dir.join(format!("cover.{format}"));
    let ran = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", &format!("testsrc2=size={side}x{side}"), "-frames:v", "1", "-y"])
        .arg(&out)
        .status();
    ran.is_ok_and(|s| s.success()).then(|| std::fs::read(&out).unwrap())
}

fn step(name: &str, runs: u32, mut f: impl FnMut()) {
    let t = Instant::now();
    for _ in 0..runs {
        f();
    }
    println!("perf: {name:<26} {:8.3} ms each", t.elapsed().as_secs_f64() * 1000.0 / runs as f64);
}

#[test]
#[ignore]
fn cover_perf() {
    let mut d = Decoder::new();
    for (format, side) in [("jpg", 1200), ("png", 1200), ("webp", 1200), ("jpg", 600)] {
        let Some(bytes) = cover(side, format) else {
            eprintln!("ffmpeg is not installed: no {format} cover");
            continue;
        };
        step(&format!("{format} {side} -> tile 300"), 20, || drop(d.decode(&bytes, 300, 300, Alpha::Straight).unwrap()));
        step(&format!("{format} {side} -> player 1080"), 10, || drop(d.decode(&bytes, 1080, 1080, Alpha::Straight).unwrap()));
    }
    let Some(bytes) = cover(1200, "jpg") else { return };
    let px = d.decode(&bytes, 300, 300, Alpha::Straight).unwrap();
    let argb: Vec<u32> = px.as_chunks::<4>().0.iter().map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]])).collect();
    step("colours of a 300 cover", 2000, || drop(nori_look::cover::derive(&argb, 300, 300, false)));
}
