//! Safe mode. While nori runs, a file mapped into memory marks each plugin being called: one atomic
//! store as a call starts and one as it ends. The kernel keeps the file as it was however nori stops (a
//! panic, an abort, a native crash, the system ending it), and the next start reads it: a plugin nori
//! stopped in is turned off, and a start that never finished, with no plugin marked, starts once with
//! every plugin off.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The file is slots of this many bytes: the first is the header (its first byte: starting), each
/// other one plugin's calls from one [`Caller`].
const SLOT: usize = 128;
const SLOTS: usize = 64;
/// A slot's bytes: being called, panicked in a call, the id's length, the id.
pub(crate) const CALLING: usize = 0;
const PANICKED: usize = 1;
const LEN: usize = 2;
const ID: usize = 3;

/// Why plugins are off at this start, for the client to say once.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SafeStart {
    /// nori stopped while calling these plugins (their ids), which are off now.
    Crashed { plugins: Vec<String> },
    /// The last start stopped before it finished with no plugin being called: every plugin is off
    /// for this start, their own switches kept.
    AllOff,
}

/// Where a plugin is called from; each has its own slot, so calls side by side don't share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caller {
    /// Its own instance's thread.
    Worker,
    /// Its sound stage, on the engine's thread.
    Stage,
    /// Its instance preparing songs, on a thread of its own.
    Preparation,
}

/// What the run before left.
#[derive(Debug, PartialEq)]
pub(crate) struct Last {
    /// The plugins (ids, cut to a slot's room) it stopped while calling, or that panicked in a call.
    pub stopped_in: Vec<String>,
    /// It stopped before its start finished.
    pub starting: bool,
}

/// `id` as a slot keeps it. Ids are ASCII (the manifest's rule).
pub(crate) fn cut(id: &str) -> &str {
    &id[..id.len().min(SLOT - ID)]
}

struct Mapped {
    /// Locked while this run has it.
    _file: File,
    _map: memmap2::MmapMut,
    at: *mut u8,
}

// SAFETY: `at` is the mapping's, which lives as long as this, and it is only ever reached through atomics.
unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

/// The file, mapped once this run has it: at start when plugins are installed, else at the first mark.
pub(crate) struct Marks {
    path: PathBuf,
    mapped: OnceLock<Option<Mapped>>,
    slots: Mutex<Vec<(String, Caller)>>,
}

/// The file at `path`, emptied and mapped, and what it held; None while another run over the same
/// folder holds it.
fn map(path: &Path) -> Option<(Mapped, Vec<u8>)> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut file = File::options().read(true).write(true).create(true).truncate(false).open(path).ok()?;
    if let Err(std::fs::TryLockError::WouldBlock) = file.try_lock() {
        return None;
    }
    let mut old = Vec::new();
    let _ = file.read_to_end(&mut old);
    file.set_len(0).ok()?;
    file.set_len((SLOT * SLOTS) as u64).ok()?;
    // SAFETY: the file is locked by this run, so nothing else changes it under the mapping.
    let mut map = unsafe { memmap2::MmapMut::map_mut(&file) }.ok()?;
    let at = map.as_mut_ptr();
    Some((Mapped { _file: file, _map: map, at }, old))
}

fn last(old: &[u8]) -> Last {
    let stopped_in = old
        .as_chunks::<SLOT>()
        .0
        .iter()
        .skip(1)
        .filter(|s| s[CALLING] != 0 || s[PANICKED] != 0)
        .filter_map(|s| s.get(ID..ID + s[LEN] as usize).and_then(|b| std::str::from_utf8(b).ok()).map(String::from))
        .collect();
    Last { stopped_in, starting: old.first() == Some(&1) }
}

impl Marks {
    /// The marks kept at `path`, and, when `read_last`, what the run before left there (None while
    /// another run holds it).
    pub(crate) fn open(path: PathBuf, read_last: bool) -> (Arc<Marks>, Option<Last>) {
        let marks = Marks { path, mapped: OnceLock::new(), slots: Mutex::default() };
        if !read_last {
            return (Arc::new(marks), None);
        }
        let (mapped, old) = map(&marks.path).unzip();
        let _ = marks.mapped.set(mapped);
        (Arc::new(marks), old.map(|o| last(&o)))
    }

    fn byte(&self, at: usize) -> Option<&AtomicU8> {
        let m = self.mapped.get_or_init(|| map(&self.path).map(|(m, _)| m)).as_ref()?;
        // SAFETY: `at` is within the mapping (slots are SLOT apart, offsets under SLOT), which outlives `self`'s borrow.
        Some(unsafe { AtomicU8::from_ptr(m.at.add(at)) })
    }

    /// Whether this run is starting: a start that doesn't finish is found at the next.
    pub(crate) fn starting(&self, on: bool) {
        if let Some(b) = self.byte(0) {
            b.store(on as u8, Ordering::Relaxed);
        }
    }

    /// The mark of plugin `id`'s calls from `caller`; one marking nothing once the slots are taken.
    pub(crate) fn mark(self: &Arc<Self>, id: &str, caller: Caller) -> Mark {
        let mut slots = self.slots.lock().expect("not poisoned");
        let n = match slots.iter().position(|(i, c)| i == id && *c == caller) {
            Some(n) => n,
            None if slots.len() + 1 < SLOTS => {
                slots.push((id.to_string(), caller));
                let at = slots.len() * SLOT;
                let name = cut(id).as_bytes();
                for (k, b) in name.iter().enumerate() {
                    let Some(cell) = self.byte(at + ID + k) else { return Mark { marks: None, at: 0 } };
                    cell.store(*b, Ordering::Relaxed);
                }
                if let Some(cell) = self.byte(at + LEN) {
                    cell.store(name.len() as u8, Ordering::Relaxed);
                }
                slots.len() - 1
            }
            None => return Mark { marks: None, at: 0 },
        };
        Mark { marks: Some(self.clone()), at: (n + 1) * SLOT }
    }
}

/// One plugin's slot for one caller.
#[derive(Clone)]
pub(crate) struct Mark {
    marks: Option<Arc<Marks>>,
    at: usize,
}

impl Mark {
    pub(crate) fn set(&self, offset: usize, v: u8) {
        if let Some(b) = self.marks.as_ref().and_then(|m| m.byte(self.at + offset)) {
            b.store(v, Ordering::Relaxed);
        }
    }

    /// `f`, marked as a call into the plugin; a panic in it stays marked for the next start.
    pub(crate) fn calling<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Ends<'a>(&'a Mark);
        impl Drop for Ends<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.set(PANICKED, 1);
                }
                self.0.set(CALLING, 0);
            }
        }
        self.set(CALLING, 1);
        let _ends = Ends(self);
        f()
    }

    /// `f`, a wait within a call (for an answer over the network), unmarked: nori ended by the system
    /// while a plugin waits is no plugin's crash.
    pub(crate) fn waiting<R>(&self, f: impl FnOnce() -> R) -> R {
        self.set(CALLING, 0);
        let r = f();
        self.set(CALLING, 1);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_run_reads_what_stopped_in_a_call_or_panicked() {
        let dir = nori_testdir::TempDir::new("plugin-marks");
        let path = dir.path().join("safe");
        let (marks, left) = Marks::open(path.clone(), true);
        assert_eq!(left, Some(Last { stopped_in: Vec::new(), starting: false }), "a new file");
        let (a, b, c) = (marks.mark("a.calls", Caller::Worker), marks.mark("b.panics", Caller::Stage), marks.mark("c.waits", Caller::Worker));
        a.calling(|| ());
        assert!(std::panic::catch_unwind(|| b.calling(|| panic!("in host code"))).is_err());
        b.calling(|| ());
        let stopped_in = || last(&std::fs::read(&path).unwrap()).stopped_in;
        c.calling(|| {
            assert!(stopped_in().contains(&"c.waits".to_string()));
            c.waiting(|| assert!(!stopped_in().contains(&"c.waits".to_string()), "a wait for an answer is no call"));
        });
        marks.starting(true);
        let (_, held) = Marks::open(path.clone(), true);
        assert_eq!(held, None, "held by the run that has it");
        // Stops in a's call.
        a.set(CALLING, 1);
        drop((marks, a, b, c));
        let (_, left) = Marks::open(path.clone(), true);
        assert_eq!(left, Some(Last { stopped_in: vec!["a.calls".into(), "b.panics".into()], starting: true }));
        let (_, again) = Marks::open(path, true);
        assert_eq!(again, Some(Last { stopped_in: Vec::new(), starting: false }), "read once");
    }
}
