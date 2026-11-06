//! What a jam guest's pick of songs does, by its role: an admin's goes into the host's queue, a plain
//! guest's is asked of the host, who decides.

use std::sync::Arc;

use nori_model::Song;
use nori_remote::wire::Op;

use crate::remote::Remote;

/// What a jam guest's pick did.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Picked {
    /// Sent to the host's queue (an admin's).
    Sent,
    /// Asked of the host (a guest's), `title` the first song asked for's: the client says so.
    Asked { title: String },
    /// In no jam: nothing was done.
    Nowhere,
}

impl Remote {
    /// Whether this guest's picks go straight to the host's queue rather than being asked.
    fn picks_play(&self) -> bool {
        self.jam_controls().is_some_and(|c| !c.asks)
    }

    /// A guest's requests, one per song.
    fn ask_for(self: &Arc<Self>, songs: Vec<Song>) -> Picked {
        let Some(title) = songs.first().map(|s| s.title.clone()).filter(|_| self.jam_controls().is_some()) else { return Picked::Nowhere };
        for song in songs {
            self.clone().jam_act(Op::Request { song });
        }
        Picked::Asked { title }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Remote {
    /// This jam guest played `songs` from `start` (`shuffle`d): an admin's replace the jam's queue and
    /// play; a guest asks for the one song that would start, any of them shuffled.
    pub fn jam_play(self: Arc<Self>, songs: Vec<Song>, start: u32, shuffle: bool) -> Picked {
        if songs.is_empty() {
            return Picked::Nowhere;
        }
        if self.picks_play() {
            let repeat = self.jam_playing().map_or(0, |m| m.repeat);
            self.jam_press(Op::Replace { songs, index: start, position_ms: 0, play: true, order: None, shuffle, repeat, transfer: None, listen: None, mix: None });
            return Picked::Sent;
        }
        let at = if shuffle { (nori_time::device_us().unsigned_abs() as usize) % songs.len() } else { (start as usize).min(songs.len() - 1) };
        self.ask_for(vec![songs[at].clone()])
    }

    /// This jam guest queued `songs` next or last: an admin's are added to the jam's queue; a guest asks
    /// for each.
    pub fn jam_add(self: Arc<Self>, songs: Vec<Song>, next: bool) -> Picked {
        if songs.is_empty() {
            return Picked::Nowhere;
        }
        if self.picks_play() {
            self.jam_press(Op::Add { songs, next });
            return Picked::Sent;
        }
        self.ask_for(songs)
    }
}
