//! The test binary's allocator: the system's, counting allocations and bytes, and the heap's peak, for
//! the perf report ([`counts`], [`peak`]), and optionally where the bytes are allocated ([`sample_stacks`]).
//! Process-wide by nature (a global allocator).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

struct Counted;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
/// Bytes held now and at most since [`restart_peak`], by threads not excluded.
static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);

/// A stack is taken once per this many bytes a thread allocates, weighted by them.
const SAMPLE_BYTES: u64 = 64 * 1024;
static SAMPLING: AtomicBool = AtomicBool::new(false);
/// Folded stack ("outer;...;inner") -> bytes allocated there, while [`SAMPLING`].
static STACKS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

thread_local! {
    static EXCLUDED: Cell<bool> = const { Cell::new(false) };
    /// Taking a stack: its own allocations are not the engine's.
    static IN_SAMPLE: Cell<bool> = const { Cell::new(false) };
    static UNTIL_SAMPLE: Cell<u64> = const { Cell::new(SAMPLE_BYTES) };
}

/// Whether this thread's allocations count (it is not excluded, and not taking a stack).
fn counted() -> bool {
    !EXCLUDED.try_with(Cell::get).unwrap_or(true) && !IN_SAMPLE.try_with(Cell::get).unwrap_or(true)
}

/// Counts an allocation of `bytes`, unless this thread is excluded.
fn count(bytes: usize) {
    if counted() {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
        if SAMPLING.load(Ordering::Relaxed) {
            sample(bytes as u64);
        }
    }
}

/// Takes this thread's stack when its next [`SAMPLE_BYTES`] are used up.
fn sample(bytes: u64) {
    let Ok(left) = UNTIL_SAMPLE.try_with(Cell::get) else { return };
    if bytes < left {
        UNTIL_SAMPLE.with(|u| u.set(left - bytes));
        return;
    }
    let weight = SAMPLE_BYTES * (1 + (bytes - left) / SAMPLE_BYTES);
    UNTIL_SAMPLE.with(|u| u.set(SAMPLE_BYTES - (bytes - left) % SAMPLE_BYTES));
    IN_SAMPLE.with(|s| s.set(true));
    let stack = folded(&std::backtrace::Backtrace::force_capture().to_string());
    if let Some(stacks) = STACKS.lock().unwrap().as_mut() {
        *stacks.entry(stack).or_default() += weight;
    }
    IN_SAMPLE.with(|s| s.set(false));
}

/// Frames of Rust's own allocation machinery, between the allocator and the code asking.
const MACHINERY: &[&str] = &[
    "alloc", "alloc_zeroed", "realloc", "alloc_impl", "alloc_impl_runtime", "allocate", "allocate_zeroed", "allocate_in", "try_allocate_in",
    "grow", "grow_amortized", "grow_exact", "grow_one", "finish_grow", "reserve", "reserve_exact", "do_reserve_and_handle", "with_capacity",
    "with_capacity_in", "with_capacity_zeroed_in", "from_elem", "extend_trusted", "extend_desugared", "spec_extend", "extend_from_slice", "push", "resize", "extend_with", "exchange_malloc", "new_uninit_in", "try_new_uninit_in",
];

/// A backtrace's frames, outermost first, from the code that asked for the memory.
fn folded(trace: &str) -> String {
    let frames: Vec<&str> = trace.lines().filter_map(|l| l.trim_start().split_once(": ").filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).map(|(_, f)| f)).collect();
    let entry = frames.iter().position(|f| f.starts_with("__rust_") || f.starts_with("__rustc::")).map_or(0, |i| i + 1);
    let machinery = |f: &&str| MACHINERY.contains(&f.split('<').next().unwrap_or(f)) || f.contains("RawVec") || f.starts_with("realloc");
    let asked = frames[entry..].iter().position(|f| !machinery(f)).map_or(frames.len(), |i| entry + i);
    let name = |f: &&str| f.rsplit_once("::h").filter(|(_, h)| h.len() == 16).map_or(*f, |(f, _)| f).replace(';', ":");
    frames[asked..].iter().rev().map(name).collect::<Vec<_>>().join(";")
}

/// Counts `delta` bytes held, unless this thread is excluded.
fn hold(delta: i64) {
    if !counted() {
        return;
    }
    let live = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: every call is the system allocator's own; only counts are kept beside it.
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count(l.size());
        hold(l.size() as i64);
        unsafe { System.alloc(l) }
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count(l.size());
        hold(l.size() as i64);
        unsafe { System.alloc_zeroed(l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        count(size);
        hold(size as i64 - l.size() as i64);
        unsafe { System.realloc(p, l, size) }
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        hold(-(l.size() as i64));
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static COUNTED: Counted = Counted;

/// Allocations so far, and their bytes, by threads not excluded.
pub fn counts() -> (u64, u64) {
    (ALLOCATIONS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
}

/// This thread's allocations stay out of [`counts`] and [`peak`] (a test's own buffers).
pub fn exclude_this_thread() {
    EXCLUDED.with(|e| e.set(true));
}

/// The peak counts from what is held now.
pub fn restart_peak() {
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Most bytes held at once since [`restart_peak`], by threads not excluded.
pub fn peak() -> i64 {
    PEAK.load(Ordering::Relaxed)
}

/// Starts taking allocation stacks, empty.
pub fn sample_stacks() {
    *STACKS.lock().unwrap() = Some(HashMap::new());
    SAMPLING.store(true, Ordering::Relaxed);
}

/// Stops taking stacks and writes them to `path` as folded lines under `root` (tools/flame.py reads them).
pub fn write_stacks(path: &std::path::Path, root: &str) {
    SAMPLING.store(false, Ordering::Relaxed);
    let stacks = STACKS.lock().unwrap().take().unwrap_or_default();
    let lines: String = stacks.iter().map(|(s, b)| format!("{root};{s} {b}\n")).collect();
    std::fs::write(path, lines).unwrap();
}
