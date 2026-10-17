//! Nothing allocates after construction: `process`, `flush`, `reset` and transposing run on the audio thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nori_stretch::Stretch;

struct Counting;

thread_local! {
    // Global by necessity: the allocator has no other place to count.
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn processing_never_allocates() {
    let (rate, ch) = (44_100usize, 2);
    // Tones, noise and silence, so the silence path and peak picking both run.
    let mut seed = 1u32;
    let x: Vec<f32> = (0..rate * 6)
        .flat_map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 9) as f32 / (1 << 23) as f32 - 0.5;
            let t = i as f32 / rate as f32;
            let v = if (i / rate) % 3 == 2 { 0.0 } else { 0.3 * (std::f32::consts::TAU * 220.0 * t).sin() + 0.1 * noise };
            [v, -v]
        })
        .collect();
    let mut s = Stretch::preset_cheaper(ch, rate as f32);
    let mut out = vec![0f32; 2048 * ch];
    let mut flushed = vec![0f32; s.output_latency() * ch];
    let before = ALLOCS.with(Cell::get);
    for semitones in [0.0, 2.0] {
        s.set_transpose_semitones(semitones);
        for (i, block) in x.chunks(256 * ch).enumerate() {
            // AutoMix's range of ratios, 0.5 to 2.
            let ratio = 0.5 + 1.5 * (i % 64) as f64 / 63.0;
            let m = ((block.len() / ch) as f64 / ratio) as usize;
            s.process(block, &mut out[..m * ch]);
        }
        s.flush(&mut flushed);
        s.reset();
    }
    assert_eq!(ALLOCS.with(Cell::get) - before, 0);
}
