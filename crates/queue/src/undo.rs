//! The undo of the user's last removal from the queue, a single song or a clear, for [`UNDO_MS`].

use nori_db as db;
use nori_model::Song;

use crate::playlist::QueueChange;
use crate::Session;

/// How long songs taken out of the queue can be put back.
pub const UNDO_MS: i64 = 5_000;

/// The undo on offer: the songs it would put back, in the order taken out, and how much longer it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueUndo {
    pub ids: Vec<String>,
    pub left_ms: u32,
}

/// A song an undo put back, at its list index as it went back.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PutBack {
    pub at: u32,
    pub song: Song,
}

impl Session {
    /// The undo on offer: the last song taken out on its own, or what the last clear took, less than
    /// [`UNDO_MS`] ago.
    pub fn undoable(&self) -> Option<QueueUndo> {
        self.undoable_at(db::now_ms())
    }

    pub(crate) fn undoable_at(&self, now: i64) -> Option<QueueUndo> {
        let q = self.queue.lock();
        let left = UNDO_MS - (now - q.taken_at?);
        let ids: Vec<String> = q.list.undoable().map(str::to_string).collect();
        (left > 0 && !ids.is_empty()).then_some(QueueUndo { ids, left_ms: left as u32 })
    }

    pub(crate) fn remove_at(&self, from: u32, to: u32, now: i64) -> QueueChange {
        let mut q = self.queue.lock();
        q.list.remove_undoably(from as usize, to as usize);
        let taken = q.list.undoable().next().is_some();
        q.taken_at = taken.then_some(now);
        let at = q.list.current();
        q.change(at)
    }

    pub(crate) fn clear_upcoming_at(&self, now: i64) -> Vec<u32> {
        let mut q = self.queue.lock();
        let gone = q.list.clear_upcoming();
        let taken = q.list.undoable().next().is_some();
        q.taken_at = taken.then_some(now);
        gone.into_iter().map(|i| i as u32).collect()
    }

    pub(crate) fn restore_all_at(&self, now: i64) -> Vec<PutBack> {
        let back: Vec<(usize, String)> = {
            let mut q = self.queue.lock();
            if !q.taken_at.is_some_and(|t| now - t < UNDO_MS) {
                return Vec::new();
            }
            let ids: Vec<String> = q.list.undoable().rev().map(str::to_string).collect();
            q.list.restore_all_taken().into_iter().zip(ids).collect()
        };
        back.into_iter().map(|(at, id)| PutBack { at: at as u32, song: self.song(&id).unwrap_or_else(|| Song::only_id(id)) }).collect()
    }

    pub(crate) fn restore_at(&self, id: &str, now: i64) -> QueueChange {
        let mut q = self.queue.lock();
        let offered = q.taken_at.is_some_and(|t| now - t < UNDO_MS);
        let at = if offered { q.list.restore_taken(id) } else { None };
        q.change(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::tests::session;

    #[test]
    fn a_song_taken_out_goes_back_only_within_the_window() {
        let s = session(&["a", "b", "c"], 0);
        s.remove_at(1, 2, 1_000);
        assert_eq!(s.undoable_at(1_000 + UNDO_MS - 1), Some(QueueUndo { ids: vec!["b".into()], left_ms: 1 }));
        assert_eq!(s.undoable_at(1_000 + UNDO_MS), None);
        assert_eq!(s.restore_at("b", 1_000 + UNDO_MS).at, None, "too late");
        assert_eq!(s.playlist(|p| p.len()), 2);

        s.remove_at(0, 1, 9_000);
        assert_eq!(s.undoable_at(9_000).map(|u| u.left_ms), Some(UNDO_MS as u32));
        assert_eq!(s.restore_at("a", 9_000 + UNDO_MS - 1).at, Some(0));
        assert_eq!(s.undoable_at(9_001), None, "once");
    }

    #[test]
    fn a_range_offers_no_undo() {
        let s = session(&["a", "b", "c"], 0);
        s.remove_at(1, 3, 0);
        assert_eq!(s.undoable_at(0), None);
    }

    #[test]
    fn a_clear_goes_back_whole_only_within_the_window() {
        let s = session(&["a", "b", "c", "d"], 1);
        assert_eq!(s.clear_upcoming_at(1_000), [3, 2]);
        assert_eq!(s.playlist(|p| p.ids().to_vec()), ["a", "b"]);
        assert_eq!(s.undoable_at(1_000).map(|u| u.ids), Some(vec!["d".into(), "c".into()]));
        assert!(s.restore_all_at(1_000 + UNDO_MS).is_empty(), "too late");
        assert_eq!(s.playlist(|p| p.len()), 2);

        let s = session(&["a", "b", "c", "d"], 1);
        s.clear_upcoming_at(9_000);
        let back = s.restore_all_at(9_000 + UNDO_MS - 1);
        assert_eq!(back.iter().map(|b| (b.at, b.song.id.as_str())).collect::<Vec<_>>(), [(2, "c"), (3, "d")]);
        assert_eq!(s.playlist(|p| (p.ids().to_vec(), p.current())), (vec!["a".into(), "b".into(), "c".into(), "d".into()], Some(1)));
        assert!(s.restore_all_at(9_001).is_empty(), "once");
        assert_eq!(s.undoable_at(9_001), None);
    }

    #[test]
    fn a_single_removal_goes_back_as_a_whole_too() {
        let s = session(&["a", "b", "c"], 0);
        s.remove_at(2, 3, 0);
        assert_eq!(s.restore_all_at(1).iter().map(|b| b.at).collect::<Vec<_>>(), [2]);
        assert_eq!(s.playlist(|p| p.len()), 3);
    }
}
