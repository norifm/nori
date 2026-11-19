//! Saving the queue in the core's database. The song store is nori-queue's.

use crate::Core;

pub use nori_queue::queue::*;
pub use nori_queue::Session;
use nori_queue::playlist::Hand;

impl Core {
    /// Saves the known songs of `now` (radio streams dropped), the current `index`, the shuffle order and
    /// the songs' marks remapped to the kept songs. The songs are written only when `now` changed since the
    /// last save, or another app over the database saved its own; otherwise only the place.
    pub(crate) fn queue_save(&self, now: Kept, index: u32, position_ms: u64) -> crate::Result<()> {
        let mut saved = self.saved_queue.lock();
        if let Some(s) = saved.as_ref().filter(|s| s.now == now) {
            if self.save_place(s.place(index), position_ms, &s.stamp)? {
                return Ok(());
            }
        }
        let (songs, kept) = self.session.store(|s| {
            let mut songs = Vec::with_capacity(now.ids.len());
            let kept = now.ids.iter().map(|id| s.songs.get(id).filter(|_| !is_radio(id)).map(|(song, _)| songs.push(song.clone())).is_some()).collect();
            (songs, kept)
        });
        let mut s = SavedQueue { now, kept, stamp: String::new() };
        let at = s.kept_at();
        let marked = |by: fn(Hand) -> bool| s.now.hands.iter().enumerate().filter(|(_, &h)| by(h)).filter_map(|(i, _)| at[i]).collect();
        let queue = crate::PlayQueue {
            index: s.place(index),
            songs,
            position_ms,
            origin: s.now.origin.clone(),
            order: s.now.order.as_ref().map(|o| o.iter().filter_map(|&i| at[i as usize]).collect()),
            repeat: s.now.repeat,
            queued: marked(Hand::by_user),
            auto: marked(|h| h == Hand::Auto),
        };
        s.stamp = self.write_queue(queue)?;
        *saved = Some(s);
        Ok(())
    }
}

/// The queue as it is to be saved: its ids, play order under shuffle, repeat mode, each song's mark and
/// the page it came from.
#[derive(PartialEq)]
pub(crate) struct Kept {
    pub(crate) ids: Vec<String>,
    pub(crate) order: Option<Vec<u32>>,
    pub(crate) repeat: u8,
    pub(crate) hands: Vec<Hand>,
    pub(crate) origin: Option<crate::PageOrigin>,
}

/// The queue whose songs are stored, and which of its ids were kept.
pub(crate) struct SavedQueue {
    now: Kept,
    kept: Vec<bool>,
    /// The stamp written with them.
    stamp: String,
}

impl SavedQueue {
    /// `index` among the kept songs.
    fn place(&self, index: u32) -> u32 {
        let before = self.kept.iter().take(index as usize).filter(|k| **k).count();
        before.min(self.kept.iter().filter(|k| **k).count().saturating_sub(1)) as u32
    }

    /// Each list index's place among the kept songs; None for one dropped.
    fn kept_at(&self) -> Vec<Option<u32>> {
        let mut n = 0;
        self.kept
            .iter()
            .map(|&k| {
                let at = n;
                n += k as u32;
                k.then_some(at)
            })
            .collect()
    }
}
