//! The phase re-lock: back at ratio 1 after a stretch, the output becomes the input again.

use nori_stretch::Stretch;

const RATE: usize = 44_100;
const BLOCK: usize = 256;
/// The analysis hop of `preset_cheaper`, frames.
const HOP: usize = RATE * 4 / 100;

/// Bass under a few partials and a little noise, stereo.
fn music(frames: usize) -> Vec<f32> {
    let mut seed = 3u64;
    (0..frames)
        .flat_map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = ((seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 0.01;
            let t = i as f32 / RATE as f32;
            let tone = |f: f32, a: f32| a * (std::f32::consts::TAU * f * t).sin();
            let v = tone(55.0, 0.3) + tone(82.4, 0.2) + tone(440.0, 0.05) + tone(1318.5, 0.02) + noise;
            [v, 0.8 * v]
        })
        .collect()
}

/// `x` at `ratio` input frames per output frame for `stretched` input frames, then at 1 with the re-lock at
/// `relock`: the output's difference from the input it plays, per hop of output at ratio 1, dB.
fn errors_at_one(x: &[f32], ratio: f64, stretched: usize, relock: f32) -> Vec<f64> {
    let mut s = Stretch::preset_cheaper(2, RATE as f32);
    let latency = s.input_latency() + s.output_latency();
    let mut buf = vec![0f32; (BLOCK * 2 + 8) * 2];
    let (mut out, mut frac) = (Vec::new(), 0.0);
    for block in x[..stretched * 2].chunks(BLOCK * 2) {
        let want = (block.len() / 2) as f64 / ratio + frac;
        let m = want.floor() as usize;
        frac = want - m as f64;
        s.process(block, &mut buf[..m * 2]);
        out.extend_from_slice(&buf[..m * 2]);
    }
    s.set_relock(relock);
    // At ratio 1 output frame `j` plays input frame `j + shift`.
    let shift = stretched as isize - (out.len() / 2) as isize - latency as isize;
    let from = out.len() / 2;
    for block in x[stretched * 2..].chunks(BLOCK * 2) {
        s.process(block, &mut buf[..block.len()]);
        out.extend_from_slice(&buf[..block.len()]);
    }
    let energy = |v: &[f32]| v.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
    (from..out.len() / 2 - HOP)
        .step_by(HOP)
        .map(|j| {
            let (made, plain) = (&out[j * 2..(j + HOP) * 2], &x[(j as isize + shift) as usize * 2..][..HOP * 2]);
            let diff: Vec<f32> = made.iter().zip(plain).map(|(a, b)| a - b).collect();
            10.0 * (energy(&diff) / energy(plain)).log10()
        })
        .collect()
}

#[test]
fn relock_returns_the_output_to_the_input() {
    let x = music(RATE * 5);
    for (ratio, stretched) in [(1.05, RATE * 2), (0.96, RATE * 3 / 2), (1.03, RATE * 5 / 2 + 777)] {
        let plain = errors_at_one(&x, ratio, stretched, 0.0);
        let relocked = errors_at_one(&x, ratio, stretched, 0.1);
        // The latency, then at most a half turn at 0.1 rad per hop.
        let bound = 3 + 32;
        assert!(relocked[bound..].iter().all(|e| *e < -40.0), "{ratio}: still {relocked:.0?} dB off the input");
        assert!(plain[bound..].iter().all(|e| *e > -20.0), "{ratio}: {plain:.0?} dB off the input with no re-lock");
    }
}

#[test]
fn relock_of_zero_is_the_plain_phase_vocoder() {
    let x = music(RATE * 3);
    let run = |relock: Option<f32>| {
        let mut s = Stretch::preset_cheaper(2, RATE as f32);
        let mut buf = vec![0f32; (BLOCK * 2 + 8) * 2];
        let mut out = Vec::new();
        for (i, block) in x.chunks(BLOCK * 2).enumerate() {
            if let Some(r) = relock.filter(|_| i % 50 == 0) {
                s.set_relock(r);
            }
            let m = if i < 200 { (block.len() / 2) * 20 / 21 } else { block.len() / 2 };
            s.process(block, &mut buf[..m * 2]);
            out.extend_from_slice(&buf[..m * 2]);
        }
        out
    };
    assert!(run(None) == run(Some(0.0)));
}
