//! Music files for tests: WAV and FLAC files of samples given.

/// A 16-bit stereo WAV file of interleaved `samples`.
pub fn wav(rate: u32, samples: &[i16]) -> Vec<u8> {
    let samples: Vec<i32> = samples.iter().map(|&v| v as i32).collect();
    wav_bits(rate, 16, &samples)
}

/// A stereo PCM WAV file with `bits` (16 or 24) per sample.
pub fn wav_bits(rate: u32, bits: u16, samples: &[i32]) -> Vec<u8> {
    let width = bits as u32 / 8;
    let data = samples.len() as u32 * width;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2 * width).to_le_bytes());
    w.extend_from_slice(&(2 * width as u16).to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for v in samples {
        w.extend_from_slice(&v.to_le_bytes()[..width as usize]);
    }
    w
}

/// Interleaved stereo 16-bit samples of a sine at `hz`, `secs` long, peaking at `peak`.
pub fn sine(rate: u32, hz: f64, secs: f64, peak: f64) -> Vec<i16> {
    let frames = (rate as f64 * secs) as usize;
    (0..frames).flat_map(|i| {
        let v = ((std::f64::consts::TAU * hz * i as f64 / rate as f64).sin() * peak) as i16;
        [v, v]
    }).collect()
}

/// A stereo 16-bit FLAC file of interleaved `samples` in fixed blocks of 4096: a block whose channel
/// holds one value is stored as that value, others verbatim, so silence makes small frames. With
/// `points`, a seek table with a point every that many seconds.
pub fn flac(rate: u32, samples: &[i16], points: Option<u64>) -> Vec<u8> {
    const BLOCK: usize = 4096;
    let frames = samples.len() / 2;
    let mut body = Vec::new();
    let mut starts = Vec::new();
    for (k, block) in samples.chunks(BLOCK * 2).enumerate() {
        starts.push(body.len() as u64);
        let n = block.len() / 2;
        let mut f = vec![0xff, 0xf8, 0x70, 0x18];
        f.extend(utf8_number(k as u64));
        f.extend(((n - 1) as u16).to_be_bytes());
        f.push(crc8(&f));
        for ch in 0..2 {
            let side: Vec<i16> = block.iter().skip(ch).step_by(2).copied().collect();
            if side.iter().all(|&v| v == side[0]) {
                f.push(0);
                f.extend(side[0].to_be_bytes());
            } else {
                f.push(2);
                f.extend(side.iter().flat_map(|v| v.to_be_bytes()));
            }
        }
        f.extend(crc16(&f).to_be_bytes());
        body.extend(f);
    }
    let mut out = b"fLaC".to_vec();
    let mut info = Vec::new();
    info.extend((BLOCK as u16).to_be_bytes());
    info.extend((BLOCK as u16).to_be_bytes());
    info.extend([0u8; 6]);
    let packed = (rate as u64) << 44 | 1 << 41 | 15 << 36 | frames as u64;
    info.extend(packed.to_be_bytes());
    info.extend([0u8; 16]);
    out.push(if points.is_some() { 0 } else { 0x80 });
    out.extend(&(info.len() as u32).to_be_bytes()[1..]);
    out.extend(info);
    if let Some(every) = points {
        let table: Vec<u8> = (0..frames as u64).step_by((every * rate as u64) as usize).flat_map(|s| {
            let k = s as usize / BLOCK;
            let mut p = (k as u64 * BLOCK as u64).to_be_bytes().to_vec();
            p.extend(starts[k].to_be_bytes());
            p.extend((BLOCK as u16).to_be_bytes());
            p
        }).collect();
        out.push(0x83);
        out.extend(&(table.len() as u32).to_be_bytes()[1..]);
        out.extend(table);
    }
    out.extend(body);
    out
}

fn utf8_number(n: u64) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let mut tail = Vec::new();
    let mut n = n;
    let mut room = 0x3f;
    while n > room {
        tail.push(0x80 | (n & 0x3f) as u8);
        n >>= 6;
        room >>= 1;
    }
    let lead = !(0xffu8 >> (tail.len() + 1)) | n as u8;
    tail.push(lead);
    tail.reverse();
    tail
}

fn crc8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |mut c, &x| {
        c ^= x;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { c << 1 ^ 0x07 } else { c << 1 };
        }
        c
    })
}

fn crc16(b: &[u8]) -> u16 {
    b.iter().fold(0u16, |mut c, &x| {
        c ^= (x as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { c << 1 ^ 0x8005 } else { c << 1 };
        }
        c
    })
}
