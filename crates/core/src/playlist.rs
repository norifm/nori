//! Core and client calls on the queue: saving it locally and pushing it to the server. The queue is nori-queue's.

use crate::client::{Client, NetResult};
use crate::Core;

pub use nori_queue::playlist::*;

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Saves the queue, its shuffle order, repeat mode, the songs' marks and origin page for the next start.
    pub fn playlist_save(&self, position_ms: u64) -> crate::Result<()> {
        let (ids, index, order, repeat, hands) = self.session.playlist(|p| {
            let order = p.shuffle_order().map(|o| o.iter().map(|&i| i as u32).collect());
            (p.ids().to_vec(), p.current().unwrap_or(0) as u32, order, p.repeat(), (0..p.len()).map(|i| p.hand(i)).collect())
        });
        let now = crate::queue::Kept { ids, order, repeat, hands, origin: self.session.origin() };
        self.queue_save(now, index, position_ms)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Saves the queue as the server's play queue ([`nori_queue::Session::to_push`]), at `current` and `position_ms`.
    pub async fn playlist_push(&self, current: Option<String>, position_ms: i64) -> NetResult<()> {
        let place = current.clone().map(|song| crate::library::QueuePlace { song, position_ms: position_ms.max(0) as u64 });
        match self.core.session.push_write(current, position_ms) {
            Some(w) => {
                self.write(w).await?;
                self.remember_queue_place(crate::library::QUEUE_PUSHED, place);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// The rest of the songs of the page the queue was started from while they were still coming
    /// ([`nori_queue::Session::follows`]), joined to the queue once the page is whole, and the change for
    /// the player to make the same. A read of the page on its way is waited for, not asked again; one
    /// that ended before the whole came is asked again. None when the queue follows no page, another
    /// queue started meanwhile, or the page could not be read.
    pub async fn queue_follow(&self) -> Option<crate::follow::Followed> {
        let follow = self.core.session.follows()?;
        match self.read_whole(crate::cache_policy::Read::PlaylistById { id: follow.origin.id.clone() }).await {
            Ok(page) => self.core.session.follow_landed(&follow, page.songs()),
            Err(_) => {
                self.core.session.follow_dropped(&follow);
                None
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;

    /// A core over its own queue, playing `ids` from `start`.
    pub(crate) fn core(ids: &[&str], start: u32) -> Arc<crate::Core> {
        let core = crate::Core::new(String::new(), "t".into(), Arc::default()).unwrap();
        core.session.set(ids.iter().map(|s| s.to_string()).collect(), Some(start), false, None);
        core
    }

    #[test]
    fn saved_queue() {
        {
            use crate::{OriginKind, PageOrigin, Song};
            let core = core(&["sv0"], 0);
            let s = &core.session;
            let song = |id: &str| Song { id: id.into(), ..Default::default() };
            s.register(vec![song("sv1"), song("sv2"), song("sv3")]);
            let from = PageOrigin::new(OriginKind::Playlist, "pl-7");
            s.set(vec!["sv1".into(), "sv2".into()], Some(1), false, Some(from.clone()));
            // Edits and the offline bridge keep the origin.
            s.take(9, vec!["sv3".into()], vec![Hand::Last]);
            s.edit_splice(|p| p.bridge(vec!["sv3".into()]), vec![]);
            s.unbridge();
            assert_eq!(s.origin(), Some(from.clone()));
            core.playlist_save(1500).unwrap();

            let q = core.load_queue().unwrap();
            assert_eq!((q.songs.len(), q.index, q.origin.as_ref()), (3, 1, Some(&from)));
            s.set(q.songs.iter().map(|s| s.id.clone()).collect(), Some(q.index), false, q.origin);
            assert!(s.from_page(&nori_library::pages::PageQueue::new(from)));

            // No origin, a pre-origin save, or an unknown kind: songs still restore.
            s.set(vec!["sv1".into()], Some(0), false, None);
            core.playlist_save(0).unwrap();
            assert_eq!(core.load_queue().unwrap().origin, None);
            for origin in ["", r#","origin":{"kind":"Nebula","id":"x"}"#] {
                let json = format!(r#"{{"songs":[{{"id":"sv1"}}],"index":0,"position":0{origin}}}"#);
                nori_db::kv_put(&core.db.lock(), "queue", &json).unwrap();
                let q = core.load_queue().unwrap();
                assert_eq!((q.songs.len(), q.origin), (1, None), "{origin}");
            }
        }

        // Save drops unsaveable songs.
        {
            let core = core(&["radio:1", "rk-unknown", "rk1", "rk2"], 2);
            core.session.register(vec![crate::Song { id: "rk1".into(), ..Default::default() }, crate::Song { id: "rk2".into(), ..Default::default() }]);
            core.playlist_save(0).unwrap();
            let q = core.load_queue().unwrap();
            assert_eq!((q.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), q.index), (vec!["rk1", "rk2"], 0), "rk1 stays current");
        }
    }

    #[test]
    fn shuffle_repeat_and_marks_are_saved_with_the_queue() {
        let core = core(&["radio:1", "o1", "o2", "o3", "o4"], 2);
        core.session.register(["o1", "o2", "o3", "o4"].map(|id| crate::Song { id: id.into(), ..Default::default() }).to_vec());
        core.session.shuffle(true);
        core.session.repeat(nori_player::playlist::REPEAT_ALL);
        core.playlist_save(0).unwrap();
        // The radio stream (list index 0) is not saved: the order skips it and counts from the songs kept.
        let kept: Vec<u32> = core.session.shuffle_order(|o| o.unwrap().iter().filter(|&&i| i != 0).map(|&i| i as u32 - 1).collect());
        let q = core.load_queue().unwrap();
        assert_eq!((q.order, q.repeat, q.index), (Some(kept), nori_player::playlist::REPEAT_ALL, 1));

        // The songs added by hand or by autofill keep their marks, counted among the songs kept.
        core.session.take(5, vec!["o1".into()], vec![Hand::Last]);
        core.session.take(6, vec!["o2".into()], vec![Hand::Auto]);
        core.playlist_save(0).unwrap();
        let q = core.load_queue().unwrap();
        assert_eq!((q.queued, q.auto), (vec![4], vec![5]));

        // Shuffle or repeat changed alone, the songs the same: saved too.
        core.session.shuffle(false);
        core.session.repeat(nori_player::playlist::REPEAT_OFF);
        core.playlist_save(0).unwrap();
        let q = core.load_queue().unwrap();
        assert_eq!((q.order, q.repeat), (None, nori_player::playlist::REPEAT_OFF));
    }

    #[test]
    fn a_queue_is_saved_only_with_songs() {
        let core = core(&["qs1"], 0);
        assert!(!core.queue_saved().unwrap(), "nothing saved yet");
        core.session.register(vec![crate::Song { id: "qs1".into(), ..Default::default() }]);
        core.playlist_save(0).unwrap();
        assert!(core.queue_saved().unwrap());
        core.session.set(vec![], None, false, None);
        core.playlist_save(0).unwrap();
        assert!(!core.queue_saved().unwrap(), "an emptied queue");
        nori_db::kv_put(&core.db.lock(), "queue", r#"{"songs":"qs1"}"#).unwrap();
        assert!(!core.queue_saved().unwrap(), "a save load_queue cannot read");
    }

    #[test]
    fn songs_rewritten_only_when_the_queue_changes() {
        let core = core(&["radio:1", "w1", "w2", "w3"], 1);
        core.session.register(["w1", "w2", "w3"].map(|id| crate::Song { id: id.into(), ..Default::default() }).to_vec());
        let songs = || nori_db::kv_get(&core.db.lock(), "queue").unwrap();
        core.playlist_save(0).unwrap();
        let first = songs();
        core.session.moved_to(3);
        core.playlist_save(4200).unwrap();
        assert_eq!(songs(), first, "a song change keeps the stored songs");
        let q = core.load_queue().unwrap();
        assert_eq!((q.index, q.position_ms, q.songs.len()), (2, 4200, 3));

        core.session.set(vec!["w2".into(), "w1".into()], Some(0), false, None);
        core.playlist_save(10).unwrap();
        let q = core.load_queue().unwrap();
        assert_eq!((q.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), q.index, q.position_ms), (vec!["w2", "w1"], 0, 10));
    }

    #[test]
    fn two_apps_over_one_database_never_pair_a_place_with_the_other_queue() {
        let dir = nori_testdir::TempDir::new("shared-queue");
        let path = dir.join("nori.db").to_string_lossy().to_string();
        let app = |ids: &[&str], start: u32| {
            let core = crate::Core::new(path.clone(), "t".into(), Arc::default()).unwrap();
            core.session.register(ids.iter().map(|id| crate::Song { id: id.to_string(), ..Default::default() }).collect());
            core.session.set(ids.iter().map(|s| s.to_string()).collect(), Some(start), false, None);
            core
        };
        let restored = || {
            let q = app(&[], 0).load_queue().unwrap();
            (q.songs.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), q.index, q.position_ms)
        };
        let ids = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (a, b) = (app(&["a1", "a2", "a3"], 0), app(&["b1", "b2"], 0));
        a.playlist_save(0).unwrap();
        b.playlist_save(0).unwrap();
        a.session.moved_to(2);
        a.playlist_save(500).unwrap();
        assert_eq!(restored(), (ids(&["a1", "a2", "a3"]), 2, 500));
        b.session.moved_to(1);
        b.playlist_save(700).unwrap();
        assert_eq!(restored(), (ids(&["b1", "b2"]), 1, 700));
    }

}
