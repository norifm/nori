//! The undo of a song the user took out of the queue: the last single song, for [`UNDO_MS`].

use nori_db as db;

use crate::playlist::QueueChange;
use crate::Session;

/// How long a song taken out of the queue can be put back.
pub const UNDO_MS: i64 = 5_000;

/// The undo on offer: the song it would put back and how much longer it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueUndo {
    pub id: String,
    pub left_ms: u32,
}

impl Session {
    /// The undo on offer: the last song taken out on its own, less than [`UNDO_MS`] ago.
    pub fn undoable(&self) -> Option<QueueUndo> {
        self.undoable_at(db::now_ms())
    }

    pub(crate) fn undoable_at(&self, now: i64) -> Option<QueueUndo> {
        let q = self.queue.lock();
        let left = UNDO_MS - (now - q.taken_at?);
        let id = q.list.undoable().filter(|_| left > 0)?;
        Some(QueueUndo { id: id.to_string(), left_ms: left as u32 })
    }

    pub(crate) fn remove_at(&self, from: u32, to: u32, now: i64) -> QueueChange {
        let mut q = self.queue.lock();
        q.list.remove_undoably(from as usize, to as usize);
        q.taken_at = q.list.undoable().map(|_| now);
        let at = q.list.current();
        q.change(at)
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
        assert_eq!(s.undoable_at(1_000 + UNDO_MS - 1), Some(QueueUndo { id: "b".into(), left_ms: 1 }));
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
}
