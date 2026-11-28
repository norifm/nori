//! Queued songs by key (their source and id). The platform player holds only keys; everything needing
//! song data (planner window, ReplayGain, the queue view, the saved queue) reads it here.

use std::collections::HashMap;

use nori_db as db;
use nori_model::{ItemKey, Keyed, Song};
use nori_player::gain::{song_gain, stereo_loudness_of_mid, GainMode as PlayerGainMode, GainPrefs, GainTags as PlayerGainTags, SongLoudness};
use nori_player::transitions::{in_album_order, WindowSong};

use crate::Session;

/// How long a song outside the queue is kept after it was last registered.
const KEEP_MS: i64 = 60_000;

/// Songs by key, each with when it was registered.
#[derive(Default)]
pub struct Store {
    pub songs: HashMap<ItemKey, (Song, i64)>,
}

pub use nori_model::{analysable, is_radio};

/// The planner's and seek bar's view of `id` (as `song` says, when known).
pub fn window_song_of(song: Option<&Song>, id: &ItemKey) -> WindowSong {
    match song {
        Some(song) => WindowSong {
            id: id.clone(),
            title: song.title.clone(),
            duration_ms: song.duration as i64 * 1000,
            album_id: song.album_id.clone().map(|album| ItemKey { source: song.source.clone(), id: album }),
            disc: song.disc_number as i32,
            track: song.track as i32,
            tag_bpm: song.bpm as f32,
            radio: false,
        },
        None => WindowSong { id: id.clone(), title: id.to_string(), radio: is_radio(&id.id), ..Default::default() },
    }
}

fn window_song(s: &Store, id: &ItemKey) -> WindowSong {
    window_song_of(s.songs.get(id).map(|(song, _)| song), id)
}

/// AutoMix's measured mid-signal loudness of `id`, if analysed: in its source's rows of `db`.
fn measured_lufs(db: &nori_db::Profile, id: &ItemKey) -> Option<f32> {
    let db = db.rows(&id.source)?;
    let a = nori_automix::store::get(&db.lock(), &id.id).ok().flatten()?;
    Some(a.lufs)
}

impl Session {
    /// Lends the store to `f`.
    pub fn store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.store.lock())
    }

    /// The cover art id of queued song `id`, if known.
    pub fn cover_art(&self, id: &ItemKey) -> Option<String> {
        self.store(|s| s.songs.get(id).and_then(|(song, _)| song.cover_art.clone()))
    }

    /// A registered song.
    pub fn song(&self, id: &ItemKey) -> Option<Song> {
        self.store(|s| s.songs.get(id).map(|(song, _)| song.clone()))
    }

    fn songs_at(&self, ids: Vec<ItemKey>, now: i64) -> Vec<Song> {
        // The queue's ids are looked at in place (the playlist's lock, then the store's, as elsewhere).
        let undo = self.undoable_at(now).map(|u| u.ids).unwrap_or_default();
        self.playlist(|p| {
            let kept: std::collections::HashSet<&ItemKey> = ids.iter().chain(p.ids()).chain(&undo).collect();
            self.store(|s| {
                s.songs.retain(|id, (_, at)| kept.contains(id) || now - *at < KEEP_MS);
                ids.iter().map(|id| s.songs.get(id).map_or_else(|| Song::only_key(id.clone()), |(song, _)| song.clone())).collect()
            })
        })
    }

    /// Each id with its length in ms (0 if unknown).
    pub(crate) fn durations(&self, ids: &[ItemKey]) -> Vec<(ItemKey, i64)> {
        self.store(|s| ids.iter().map(|id| (id.clone(), s.songs.get(id).map_or(0, |(song, _)| song.duration as i64 * 1000))).collect())
    }

    /// Hands the planner its window: the previous, current and next songs in play order.
    pub(crate) fn hand_window(&self, songs: &[ItemKey], shuffling: bool) {
        let window = self.store(|s| songs.iter().map(|id| window_song(s, id)).collect());
        self.planner.transition_window(window, shuffling);
    }

    /// The ReplayGain volume for `current` given its neighbours (album gain applies only inside an album
    /// in order). 1.0 for nothing, radio or bit-perfect output. Untagged songs fall back to AutoMix's
    /// measured loudness.
    pub fn queue_gain(&self, before: Option<ItemKey>, current: Option<ItemKey>, after: Option<ItemKey>, prefs: &GainPrefs, bit_perfect: bool, shuffling: bool) -> f32 {
        let Some(current) = current.filter(|id| !is_radio(&id.id)) else { return 1.0 };
        if bit_perfect {
            return 1.0;
        }
        let (run, mut song, channels) = self.store(|s| {
            let w = |p: &Option<ItemKey>| p.as_ref().map(|i| window_song(s, i));
            let (b, c, a) = (w(&before), window_song(s, &current), w(&after));
            let run = in_album_order(b.as_ref(), &c, a.as_ref(), shuffling);
            let known = s.songs.get(&current).map(|(song, _)| song);
            let rg = known.and_then(|song| song.replay_gain.as_ref());
            let tags = rg.map(|g| PlayerGainTags {
                track_gain: g.track_gain,
                album_gain: g.album_gain,
                track_peak: g.track_peak,
                album_peak: g.album_peak,
            });
            let song = SongLoudness { tags, fallback_db: rg.and_then(|g| g.fallback_gain), measured_lufs: None };
            (run, song, known.map_or(0, |s| s.channel_count))
        });
        let untagged = song.tags.is_none_or(|g| g.track_gain.is_none() && g.album_gain.is_none()) && song.fallback_db.is_none();
        if untagged && prefs.mode != PlayerGainMode::Off {
            // Outside the store's lock: the database's is never taken inside it.
            song.measured_lufs = measured_lufs(&self.db, &current).map(|mid| stereo_loudness_of_mid(mid, channels));
        }
        let nori = song_gain(prefs, &song, run);
        // Leveling off stays off.
        if prefs.mode == PlayerGainMode::Off {
            return nori;
        }
        self.decide_leveling(&current, nori)
    }

    /// Song `id`'s measured integrated loudness (BS.1770, LUFS), once AutoMix measured it.
    pub fn loudness(&self, id: &ItemKey) -> Option<f32> {
        let channels = self.store(|s| s.songs.get(id).map_or(2, |(song, _)| song.channel_count));
        measured_lufs(&self.db, id).filter(|l| *l > -70.0).map(|mid| stereo_loudness_of_mid(mid, channels))
    }

    /// Whether registered song `id` is marked explicit; false if unknown.
    pub fn explicit(&self, id: &ItemKey) -> bool {
        self.store(|s| s.songs.get(id).is_some_and(|(song, _)| song.explicit_status == nori_model::ExplicitStatus::Explicit))
    }

    /// The distinct albums of `ids`, as their sources tell them apart.
    pub fn albums(&self, ids: Vec<ItemKey>) -> Vec<ItemKey> {
        self.store(|s| {
            let mut seen = std::collections::HashSet::new();
            ids.iter().filter_map(|id| Some(ItemKey { source: id.source.clone(), id: s.songs.get(id)?.0.album_id.clone()? })).filter(|a| seen.insert(a.clone())).collect()
        })
    }

    /// The ids that may be prefetched: no radio, no provider songs (fetching one makes the server
    /// download it).
    pub fn fetchable(&self, ids: Vec<ItemKey>) -> Vec<ItemKey> {
        self.store(|s| ids.into_iter().filter(|id| !is_radio(&id.id) && !nori_model::is_provider_id(&id.id) && !s.songs.get(id).is_some_and(|(song, _)| song.is_provider())).collect())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Session {
    /// Registers songs about to be queued.
    #[cfg_attr(feature = "ffi", uniffi::method(name = "queue_register"))]
    pub fn register(&self, songs: Vec<Song>) {
        let now = db::now_ms();
        self.store(|s| {
            for song in songs {
                s.songs.insert(song.key(), (song, now));
            }
        });
    }

    /// The songs for `ids` in order (unknown ones as id only). Prunes the store to `ids`, the queue, the
    /// song an undo would put back, and songs registered within [`KEEP_MS`].
    #[cfg_attr(feature = "ffi", uniffi::method(name = "queue_songs"))]
    pub fn songs(&self, ids: Vec<ItemKey>) -> Vec<Song> {
        self.songs_at(ids, db::now_ms())
    }

    /// Whether registered song `id` is starred, as the server said when it was queued; false if unknown.
    pub fn queue_starred(&self, id: ItemKey) -> bool {
        self.store(|s| s.songs.get(&id).is_some_and(|(song, _)| song.starred))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_songs_kept() {
        assert!(analysable("a1b2"));
        assert!(analysable("ext-deezer-1"), "measured as it plays or once downloaded");
        assert!(!analysable("radio:3"));

        // Songs lookup keeps queued songs.
        let s = crate::playlist::tests::session(&["keep1", "keep2", "keep3"], 0);
        let song = |id: &str| Song { duration: 200, ..Song::only_id(id.to_string()) };
        s.register(vec![song("keep1"), song("keep2"), song("keep3"), song("taken"), song("gone")]);
        s.take(3, vec![ItemKey::own("taken")], vec![crate::playlist::Hand::No]);
        // Long after they were registered, the song just taken out is kept while its undo is offered.
        let later = db::now_ms() + 2 * KEEP_MS;
        s.remove_at(3, 4, later - 1_000);
        // Asking for one song keeps the rest of the queue.
        assert_eq!(s.songs_at(vec![ItemKey::own("keep1")], later)[0].duration, 200);
        for id in ["keep2", "keep3", "taken"] {
            assert_eq!(s.song(&ItemKey::own(id)).map(|s| s.duration), Some(200), "{id}");
        }
        assert!(s.song(&ItemKey::own("gone")).is_none());
        s.songs_at(vec![ItemKey::own("keep1")], later + crate::undo::UNDO_MS);
        assert!(s.song(&ItemKey::own("taken")).is_none(), "once its undo is over");
    }

}
