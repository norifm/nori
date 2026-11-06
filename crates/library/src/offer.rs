//! The queue another device or app left on the server, as Home offers it (`Client::queue_offer`): a
//! look's answer is shown while it stands against what the player shows, let go once it does not, and
//! closed when it is taken up or turned down (then marked done, `Client::queue_offer_done`, and, taken,
//! played from its song and place).
//!
//! [`Offered`] is the state; [`HomeOffer`] is the same behind a lock, for a client that keeps it across
//! the FFI (Android).

use crate::library::{offer_stands, QueueOffer};

/// The offer Home shows, if any.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Offered {
    offer: Option<QueueOffer>,
}

impl Offered {
    pub const fn new() -> Self {
        Offered { offer: None }
    }

    /// A look's answer, with what the player shows now (`here`, `playing`): it replaces the offer shown,
    /// and is shown only while it stands. The offer shown after it.
    pub fn found(&mut self, offer: Option<QueueOffer>, here: Option<String>, playing: bool) -> Option<&QueueOffer> {
        self.offer = offer;
        self.still(here, playing);
        self.offer.as_ref()
    }

    /// The player shows `here` and is `playing` or not: an offer that no longer stands (music started,
    /// or the player came to show its song) is let go. Whether one is still shown.
    pub fn still(&mut self, here: Option<String>, playing: bool) -> bool {
        let stands = self.offer.as_ref().is_some_and(|o| o.songs.get(o.index as usize).is_some_and(|s| offer_stands(&s.id, here, playing)));
        if !stands {
            self.offer = None;
        }
        stands
    }

    /// Taken up or turned down: the offer goes, to be marked done and, taken, played where it was left.
    pub fn close(&mut self) -> Option<QueueOffer> {
        self.offer.take()
    }

    pub fn shown(&self) -> Option<&QueueOffer> {
        self.offer.as_ref()
    }
}

/// [`Offered`] behind a lock.
#[derive(Debug, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct HomeOffer {
    offered: std::sync::Mutex<Offered>,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl HomeOffer {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::default()
    }

    /// [`Offered::found`]: whether `offer` is shown.
    pub fn found(&self, offer: Option<QueueOffer>, here: Option<String>, playing: bool) -> bool {
        self.offered().found(offer, here, playing).is_some()
    }

    /// [`Offered::still`].
    pub fn still(&self, here: Option<String>, playing: bool) -> bool {
        self.offered().still(here, playing)
    }

    /// [`Offered::close`] (`close` itself is the binding's own, freeing it).
    pub fn end(&self) -> Option<QueueOffer> {
        self.offered().close()
    }
}

impl HomeOffer {
    fn offered(&self) -> std::sync::MutexGuard<'_, Offered> {
        self.offered.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::QueueBy;
    use nori_model::Song;

    fn offer(at: &str) -> QueueOffer {
        let songs = ["a", "b", "c"].iter().map(|id| Song { id: id.to_string(), ..Default::default() }).collect();
        let index = ["a", "b", "c"].iter().position(|id| *id == at).unwrap() as u32;
        QueueOffer { by: QueueBy::Nori, songs, index, position_ms: 30_000 }
    }

    #[test]
    fn an_offer_is_shown_while_it_stands() {
        let mut o = Offered::default();
        assert_eq!(o.found(Some(offer("b")), None, false), Some(&offer("b")));
        assert!(o.still(Some("a".into()), false), "the player shows another song, paused");
        assert!(!o.still(Some("a".into()), true), "music started");
        assert_eq!(o.shown(), None);
        assert!(!o.still(None, false), "a let go offer does not come back");
        assert_eq!(o.found(Some(offer("b")), Some("b".into()), false), None, "the player shows its song as the answer comes");
    }

    #[test]
    fn a_look_replaces_the_offer_shown() {
        let mut o = Offered::default();
        o.found(Some(offer("b")), None, false);
        assert_eq!(o.found(Some(offer("c")), None, false), Some(&offer("c")));
        assert_eq!(o.found(None, None, false), None, "nothing left on the server");
    }

    #[test]
    fn a_closed_offer_is_handed_over_once() {
        let o = HomeOffer::new();
        assert!(o.found(Some(offer("b")), None, false));
        assert_eq!(o.end(), Some(offer("b")));
        assert_eq!(o.end(), None);
        assert!(!o.still(None, false));
    }
}
