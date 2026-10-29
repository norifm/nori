//! An output into a plugin's cast ([`nori_plugins::Sink`]): the engine's final audio, pumped in real
//! time, a little ahead, on a thread of its own while it plays. The device far away buffers what is
//! ahead; a socket that fails ends the cast's output as a dead card would.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use nori_plugins::Sink;
use nori_time::{Duration, Instant};

use crate::output::{AudioOutput, Feed, OutputFormat};

/// Frames per pull.
const PERIOD: usize = 1024;
/// How far ahead of real time the pump runs: what the device buffers against the network's jitter.
const LEAD: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Shared {
    playing: AtomicBool,
    closed: AtomicBool,
    gone: AtomicBool,
}

pub struct CastOutput {
    sink: Arc<Sink>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl CastOutput {
    pub fn new(sink: Arc<Sink>) -> CastOutput {
        CastOutput { sink, shared: Arc::default(), thread: None }
    }
}

fn pump(mut feed: Feed, sink: Arc<Sink>, shared: Arc<Shared>) {
    let f = feed.format();
    let mut block = vec![0i16; PERIOD * f.channels];
    let period = Duration::from_secs_f64(PERIOD as f64 / f.rate as f64);
    let mut due = Instant::now();
    while !shared.closed.load(Ordering::Acquire) {
        if !shared.playing.load(Ordering::Acquire) {
            std::thread::park();
            due = Instant::now();
            continue;
        }
        let now = Instant::now();
        if due > now + LEAD {
            std::thread::sleep(due - now - LEAD);
        }
        due += period;
        // An underrun is sent as the silence it is: the device's stream never stops.
        feed.pull_i16(&mut block);
        if !sink.write(&block) {
            shared.gone.store(true, Ordering::Release);
            feed.wake_engine();
            return;
        }
    }
}

impl AudioOutput for CastOutput {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        let (rate, channels) = self.sink.wants.unwrap_or((want.rate, want.channels.min(8) as u8));
        self.sink.open(rate, channels);
        Ok(OutputFormat { rate, channels: channels as usize, bits: 0 })
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        let (sink, shared) = (self.sink.clone(), self.shared.clone());
        shared.closed.store(false, Ordering::Release);
        let t = std::thread::Builder::new().name("nori-cast".into()).spawn(move || pump(feed, sink, shared)).map_err(|e| e.to_string())?;
        self.thread = Some(t);
        Ok(())
    }

    fn pause(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    fn resume(&mut self) {
        self.shared.playing.store(true, Ordering::Release);
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    /// The device's own delay, and what the pump sent ahead of it.
    fn latency_us(&self) -> u64 {
        self.sink.latency_us() + LEAD.as_micros() as u64
    }

    fn failed(&mut self) -> Option<String> {
        self.shared.gone.load(Ordering::Acquire).then(|| "the cast's device is gone".to_string())
    }

    fn close(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}
