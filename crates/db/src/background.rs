//! The thread database writes run on, so callers (often the main or audio thread) never wait on the
//! database.

use std::sync::mpsc::channel;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{mpsc::Sender, Arc, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
type Job = Box<dyn FnOnce() + Send>;

/// Runs jobs one after another on its own thread, started at the first job; the thread ends with the
/// last clone. The web has no threads (and builds without unwinding): there a job runs at once.
#[derive(Clone, Default)]
pub struct Writer {
    #[cfg(not(target_arch = "wasm32"))]
    jobs: Arc<OnceLock<Sender<Job>>>,
}

impl Writer {
    /// Runs `job` after earlier jobs. A job that panics is logged; the jobs after it still run.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let jobs = self.jobs.get_or_init(|| {
            let (tx, rx) = channel::<Job>();
            std::thread::Builder::new()
                .name("nori-core".into())
                .spawn(move || {
                    for job in rx {
                        if catch_unwind(AssertUnwindSafe(job)).is_err() {
                            nori_model::alog::info("background: a write panicked");
                        }
                    }
                })
                .expect("spawn the background thread");
            tx
        });
        let _ = jobs.send(Box::new(job));
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        job();
    }

    /// Waits until the jobs sent so far have run: a client quitting, so its last writes are kept.
    pub fn flush(&self) {
        let (tx, rx) = channel();
        self.run(move || {
            let _ = tx.send(());
        });
        let _ = rx.recv();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panicking_write_stops_no_other() {
        let w = Writer::default();
        w.run(|| panic!("a broken write"));
        let (tx, rx) = channel();
        w.run(move || tx.send(()).unwrap());
        w.flush();
        assert!(rx.try_recv().is_ok());
    }
}
