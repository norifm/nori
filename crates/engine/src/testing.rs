//! A virtual clock for tests: time moves only while the engine sleeps, and the test's device pulls on that
//! time, so results do not depend on machine load and minutes of music run in moments. [`Stepper`]
//! waits for the engine to sleep, then advances to the next engine timer or device pull. While the engine
//! waits for a song's bytes, time stands still (up to [`BYTES_WAIT`] of real time).

use std::sync::Arc;
use std::thread::Thread;
use nori_time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::Clock;

/// Real time the clock waits per sleep for bytes the engine awaits; longer than any in-memory server takes.
pub const BYTES_WAIT: Duration = Duration::from_millis(2_000);
/// The shorter wait while a server itself waits for the clock (a paced transcode).
pub const TIME_WAIT: Duration = Duration::from_millis(20);
/// Real time after which an engine that never sleeps is taken as stuck.
const STUCK: Duration = Duration::from_secs(120);

#[derive(Default)]
struct State {
    now_ns: i64,
    engine: Option<Thread>,
    /// The engine sleeps (until `deadline_ns`, if any), awaiting bytes if `bytes`.
    parked: bool,
    deadline_ns: Option<i64>,
    bytes: bool,
    /// Sleeps so far, and the one whose byte wait was given up.
    sleeps: u64,
    gave_up: u64,
    /// Servers waiting for the clock ([`Virtual::wait_until`]), and the times they wait for: the clock
    /// stops at each until its server ran.
    time_waiters: u32,
    due_ns: Vec<i64>,
    /// Deliveries whose bytes are being taken ([`Delivery`]): the clock waits for them.
    handing: u32,
    /// Woken by the test and no turn taken since.
    woken: bool,
}

#[derive(Default)]
struct Shared {
    s: Mutex<State>,
    cv: Condvar,
}

#[derive(Clone, Default)]
pub struct Virtual(Arc<Shared>);

impl Clock for Virtual {
    /// Retries take milliseconds, and a request never stalls: this clock cannot see real time, and a
    /// server paced on it takes far longer in real time than any stall on a phone.
    fn waits(&self) -> crate::Waits {
        crate::Waits { stall_ms: 600_000, retry_ms: 2 }
    }

    fn now_ms(&self) -> i64 {
        self.0.s.lock().now_ns / 1_000_000
    }

    fn now_us(&self) -> i64 {
        self.0.s.lock().now_ns / 1_000
    }

    fn sleep(&self, ms: Option<u64>, waiting: impl FnOnce() -> bool) {
        let bytes = waiting();
        let mut s = self.0.s.lock();
        s.engine.get_or_insert_with(std::thread::current);
        if s.woken {
            // Woken during the turn: turn again.
            return;
        }
        s.parked = true;
        s.deadline_ns = ms.map(|ms| s.now_ns + ms as i64 * 1_000_000);
        s.bytes = bytes;
        s.sleeps += 1;
        self.0.cv.notify_all();
        drop(s);
        std::thread::park();
        let mut s = self.0.s.lock();
        s.parked = false;
        s.deadline_ns = None;
    }

    fn woke(&self) {
        let mut s = self.0.s.lock();
        if std::mem::take(&mut s.woken) {
            // Consume the wake's unpark so it cannot cut the next sleep short.
            std::thread::park_timeout(Duration::ZERO);
        }
    }

    fn wake(&self, engine: &Thread) {
        let mut s = self.0.s.lock();
        s.woken = true;
        engine.unpark();
    }
}

impl Virtual {
    pub fn now_ns(&self) -> i64 {
        self.0.s.lock().now_ns
    }

    /// Blocks until the engine sleeps with nothing due now.
    pub fn settle(&self) {
        let started = Instant::now();
        let mut s = self.0.s.lock();
        let mut bytes_since: Option<(u64, Instant)> = None;
        loop {
            if s.parked && !s.woken {
                if !s.bytes || s.gave_up == s.sleeps {
                    return;
                }
                let hold = if s.time_waiters > 0 { TIME_WAIT } else { BYTES_WAIT };
                match bytes_since {
                    Some((n, t)) if n == s.sleeps => {
                        if t.elapsed() >= hold {
                            s.gave_up = s.sleeps;
                            return;
                        }
                    }
                    _ => bytes_since = Some((s.sleeps, Instant::now())),
                }
            }
            assert!(started.elapsed() < STUCK, "the engine did not go to sleep");
            self.0.cv.wait_for(&mut s, Duration::from_millis(1));
        }
    }

    /// Blocks a server thread until the clock reaches `ns`; meanwhile byte waits are [`TIME_WAIT`].
    pub fn wait_until(&self, ns: i64) {
        self.wait_then(ns, &mut false, false);
    }

    /// [`Virtual::wait_until`]; with `hand`, a delivery's bytes are then handed over (`handing`), the
    /// last ones taken.
    fn wait_then(&self, ns: i64, handing: &mut bool, hand: bool) {
        let started = Instant::now();
        let mut s = self.0.s.lock();
        if std::mem::replace(handing, hand) {
            s.handing -= 1;
        }
        s.time_waiters += 1;
        s.due_ns.push(ns);
        while s.now_ns < ns {
            assert!(started.elapsed() < STUCK, "the test's clock never reached {ns} ns (it stays at {} ns)", s.now_ns);
            self.0.cv.wait_for(&mut s, Duration::from_millis(1));
        }
        if let Some(k) = s.due_ns.iter().position(|&d| d == ns) {
            s.due_ns.remove(k);
        }
        s.time_waiters -= 1;
        s.handing += hand as u32;
        self.0.cv.notify_all();
    }

    /// A server's bytes handed over at times of this clock.
    pub fn delivery(&self) -> Delivery {
        Delivery { clock: self.clone(), handing: false }
    }

    fn handed(&self, handing: &mut bool) {
        if std::mem::take(handing) {
            self.0.s.lock().handing -= 1;
            self.0.cv.notify_all();
        }
    }

    /// The earliest time a server waits for.
    fn server_due_ns(&self) -> Option<i64> {
        self.0.s.lock().due_ns.iter().min().copied()
    }

    /// Blocks until every server waiting for a time now passed has run on, and every delivery's bytes
    /// were taken.
    fn let_servers_run(&self) {
        let started = Instant::now();
        let mut s = self.0.s.lock();
        while s.handing > 0 || s.due_ns.iter().any(|&d| d <= s.now_ns) {
            assert!(started.elapsed() < STUCK, "a server never ran on");
            self.0.cv.wait_for(&mut s, Duration::from_millis(1));
        }
    }

    /// Blocks a server thread while `on` holds (a hung request), at most `limit`; returns whether it
    /// still held. Meanwhile byte waits are [`TIME_WAIT`].
    pub fn hang_while(&self, mut on: impl FnMut() -> bool, limit: Duration) -> bool {
        let started = Instant::now();
        let mut s = self.0.s.lock();
        s.time_waiters += 1;
        let held = loop {
            if !on() {
                break false;
            }
            if started.elapsed() >= limit {
                break true;
            }
            self.0.cv.wait_for(&mut s, Duration::from_millis(1));
        };
        s.time_waiters -= 1;
        held
    }

    /// Sleeps so far.
    pub fn sleeps(&self) -> u64 {
        self.0.s.lock().sleeps
    }

    /// When the engine asked to be woken.
    pub fn deadline_ns(&self) -> Option<i64> {
        self.0.s.lock().deadline_ns
    }

    /// Advances to `ns`, waking the engine if its timer ran out.
    pub fn move_to(&self, ns: i64) {
        let mut s = self.0.s.lock();
        s.now_ns = s.now_ns.max(ns);
        self.0.cv.notify_all();
        if s.deadline_ns.is_some_and(|d| d <= s.now_ns) {
            s.deadline_ns = None;
            Self::poke(&mut s);
        }
    }

    /// The test woke the engine (e.g. a pull reached the ring's low mark).
    pub fn woke_engine(&self) {
        Self::poke(&mut self.0.s.lock());
    }

    fn poke(s: &mut State) {
        if let Some(t) = &s.engine {
            s.woken = true;
            t.unpark();
        }
    }
}

/// Bytes a server hands over at times of the clock: the clock stops at each, and goes on once they were
/// taken (the reader asks for the next, or the delivery is dropped).
pub struct Delivery {
    clock: Virtual,
    handing: bool,
}

impl Delivery {
    /// Blocks until the clock reaches `ns`, the bytes before it taken.
    pub fn at(&mut self, ns: i64) {
        self.clock.wait_then(ns, &mut self.handing, true);
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.clock.handed(&mut self.handing);
    }
}

/// A device on the clock.
pub trait Device: Send {
    /// When it next pulls, ns.
    fn due_ns(&self) -> i64;
    /// Pulls at `now_ns`; true when that woke the engine.
    fn tick(&mut self, now_ns: i64) -> bool;
}

/// Drives a [`Virtual`] clock through engine timers and device pulls.
pub struct Stepper<D: Device> {
    pub clock: Virtual,
    pub device: Arc<Mutex<D>>,
}

impl<D: Device> Stepper<D> {
    pub fn new(clock: Virtual, device: Arc<Mutex<D>>) -> Self {
        Stepper { clock, device }
    }

    /// Runs the next due event: the engine's timer or the device's pull.
    pub fn step(&self) {
        self.step_by(i64::MAX);
    }

    /// [`Stepper::step`], the clock going no further than `limit_ns`.
    fn step_by(&self, limit_ns: i64) {
        self.clock.settle();
        let pull = self.device.lock().due_ns();
        let t = self.clock.deadline_ns().map_or(pull, |d| d.min(pull)).min(limit_ns);
        let t = self.clock.server_due_ns().map_or(t, |d| d.min(t));
        self.clock.move_to(t);
        self.clock.let_servers_run();
        self.clock.settle();
        if t >= pull {
            let woke = self.device.lock().tick(t);
            if woke {
                self.clock.woke_engine();
                self.clock.settle();
            }
        }
    }

    /// Advances by `d`.
    pub fn run(&self, d: Duration) {
        let until = self.clock.now_ns() + d.as_nanos() as i64;
        while self.clock.now_ns() < until {
            self.step_by(until);
        }
        self.clock.settle();
    }

    /// Advances until `done`, at most `limit`; returns whether it came.
    pub fn until(&self, limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let until = self.clock.now_ns() + limit.as_nanos() as i64;
        self.clock.settle();
        loop {
            if done() {
                return true;
            }
            if self.clock.now_ns() >= until {
                return false;
            }
            self.step();
        }
    }
}

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
