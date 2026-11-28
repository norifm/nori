//! A queue started from a page whose songs are still coming (a provider's long playlist, a page of it per
//! request). The songs shown so far play at once; the rest join the queue once the page is whole
//! (`Client::queue_follow` reads it). A page's read marks here how far it has shown the page
//! ([`Session::page_listed`]).

use std::collections::HashMap;

use nori_model::{Keyed, PageOrigin, Song};

use crate::playlist::QueueEdit;
use crate::Session;

/// How far a read has shown a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listed {
    /// Its first songs; more are on their way.
    Partly,
    /// All of its songs, this many.
    Whole(usize),
}

/// The page a queue was started from while its songs were still coming: the queue was given the first
/// `given` of them and waits for the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    pub origin: PageOrigin,
    given: usize,
    /// The queue it belongs to (its `origin_gen`): a queue started since follows nothing of it.
    queue: u32,
}

/// The pages shown before all their songs came, and how far.
#[derive(Default)]
pub(crate) struct Pages(HashMap<PageOrigin, Listed>);

impl Pages {
    /// What a queue of `given` songs, numbered `queue`, follows of page `origin`: the page as shown lists
    /// only its first songs, or fewer than came since (Play pressed as the rest arrived).
    pub(crate) fn follow(&self, origin: Option<&PageOrigin>, given: usize, queue: u32) -> Option<Follow> {
        let origin = origin?;
        let more = match self.0.get(origin)? {
            Listed::Partly => true,
            Listed::Whole(n) => given < *n,
        };
        more.then(|| Follow { origin: origin.clone(), given, queue })
    }
}

/// The rest of a page's songs joining the queue ([`Session::follow_landed`]).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Followed {
    /// For the player to make the same change.
    pub edit: QueueEdit,
    /// The queue's last entry in play order before they joined: a player stopped at its end plays on.
    pub after: Option<u64>,
}

impl Session {
    /// A read showed page `origin` as far as `listed`.
    pub fn page_listed(&self, origin: PageOrigin, listed: Listed) {
        self.queue.lock().pages.0.insert(origin, listed);
    }

    /// The page the queue waits on for the rest of its songs.
    pub fn follows(&self) -> Option<Follow> {
        self.queue.lock().follows.clone()
    }

    /// `follow`'s page came whole, with `songs`: those after the ones the queue was given join its end (in
    /// order, or shuffled in among the songs not played yet), unless another queue started meanwhile. None
    /// when nothing joined.
    pub fn follow_landed(&self, follow: &Follow, songs: Vec<Song>) -> Option<Followed> {
        let rest: Vec<Song> = songs.into_iter().skip(follow.given).collect();
        let mut q = self.queue.lock();
        if q.follows.as_ref() != Some(follow) {
            return None;
        }
        q.follows = None;
        if rest.is_empty() {
            return None;
        }
        self.register(rest.clone());
        let after = q.list.play_order().last().map(|at| q.list.seqs()[at]);
        let at = q.list.extend(rest.iter().map(Keyed::key).collect(), crate::playlist::seed());
        let shuffled = q.list.shuffle_order().is_some();
        Some(Followed { edit: QueueEdit { remove: Vec::new(), at: at as u32, songs: rest, seek: None, shuffled }, after })
    }

    /// `follow`'s page could not be read whole: the queue keeps the songs it has.
    pub fn follow_dropped(&self, follow: &Follow) {
        let mut q = self.queue.lock();
        if q.follows.as_ref() == Some(follow) {
            q.follows = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_model::OriginKind;

    fn songs(ids: &[&str]) -> Vec<Song> {
        ids.iter().map(|id| Song::only_id(id.to_string())).collect()
    }

    fn ids(v: &[&str]) -> Vec<nori_model::ItemKey> {
        v.iter().map(|s| nori_model::ItemKey::own(*s)).collect()
    }

    fn list(s: &Session) -> Vec<String> {
        s.playlist(|p| p.ids().iter().map(|k| k.id.clone()).collect())
    }

    fn page() -> PageOrigin {
        PageOrigin::new(OriginKind::Playlist, "long")
    }

    #[test]
    fn a_queue_from_a_page_still_coming_takes_the_rest_when_it_is_whole() {
        let s = Session::default();
        s.page_listed(page(), Listed::Partly);
        let change = s.set(ids(&["a", "b"]), Some(1), false, Some(page()));
        assert!(change.follows);
        let follow = s.follows().unwrap();
        assert!(!s.autofill_start(), "the rest of the page comes before any refill");

        let landed = s.follow_landed(&follow, songs(&["a", "b", "c", "d"])).unwrap();
        assert_eq!(list(&s), ["a", "b", "c", "d"]);
        assert_eq!((landed.edit.at, landed.edit.songs.len(), landed.edit.shuffled), (2, 2, false));
        assert_eq!(landed.after, Some(s.playlist(|p| p.seqs()[1])), "the end it had");
        assert_eq!(s.playlist(|p| p.next()), Some(2), "they play next, in order");
        assert!(s.follows().is_none());
        assert!(s.follow_landed(&follow, songs(&["a", "b", "c", "d", "e"])).is_none(), "once");
    }

    #[test]
    fn only_a_page_shown_in_part_is_followed() {
        let s = Session::default();
        assert!(!s.set(ids(&["a"]), Some(0), false, Some(page())).follows, "never shown in part");
        s.page_listed(page(), Listed::Whole(3));
        assert!(!s.set(ids(&["a", "b", "c"]), Some(0), false, Some(page())).follows, "shown whole");
        assert!(s.set(ids(&["a"]), Some(0), false, Some(page())).follows, "played from the part shown as the rest came");
        assert!(!s.set(ids(&["a"]), Some(0), false, Some(PageOrigin::new(OriginKind::Playlist, "other"))).follows);
        assert!(!s.set(ids(&["a"]), Some(0), false, None).follows);
    }

    #[test]
    fn a_queue_started_since_takes_nothing() {
        let s = Session::default();
        s.page_listed(page(), Listed::Partly);
        s.set(ids(&["a"]), Some(0), false, Some(page()));
        let follow = s.follows().unwrap();
        s.set(ids(&["a"]), Some(0), false, Some(page()));
        assert!(s.follow_landed(&follow, songs(&["a", "b"])).is_none(), "the same page, started again");
        assert_eq!(list(&s), ["a"]);
        s.follow_dropped(&s.follows().unwrap());
        assert!(s.follows().is_none(), "a page that could not be read whole: the queue keeps what it has");
    }

    #[test]
    fn shuffled_the_rest_joins_the_songs_not_played_yet() {
        let s = Session::default();
        s.page_listed(page(), Listed::Partly);
        s.set(ids(&["a", "b", "c"]), None, true, Some(page()));
        let playing = s.playlist(|p| p.current()).unwrap();
        let follow = s.follows().unwrap();
        let landed = s.follow_landed(&follow, songs(&["a", "b", "c", "d", "e"])).unwrap();
        assert_eq!(list(&s), ["a", "b", "c", "d", "e"]);
        assert!(landed.edit.shuffled);
        let order: Vec<usize> = s.playlist(|p| p.play_order().collect());
        assert_eq!(order[0], playing);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, [0, 1, 2, 3, 4]);
    }
}
