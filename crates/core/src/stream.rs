//! Stream and download URLs and cache keys, and the prefetch list. Keys and network state are nori-net's.

use crate::client::Client;
use crate::ItemKey;

pub use nori_net::stream::*;

impl Client {
    /// Where `id` opens now: the download if finished, else a stream at the network's quality now; None
    /// for a song of a source the profile does not have.
    pub fn resolve_now(&self, id: &ItemKey) -> Option<StreamTarget> {
        let kept = self.core.transfers().held().state(id) == crate::transfers::HeldState::Done;
        self.resolve(id.clone(), kept, !kept && self.metered())
    }

    /// [`Client::precache_targets`] on the network now.
    pub fn precache_now(&self) -> Vec<Fetch> {
        self.precache_targets(self.metered())
    }

    /// The user's (wifi, mobile, download) qualities.
    fn saved_qualities(&self) -> (StreamQuality, StreamQuality, StreamQuality) {
        let q = |s: &crate::settings::SavedQuality| StreamQuality { bit_rate: s.bit_rate.max(0) as u32, format: s.format.clone() };
        self.settings().in_effect(|p| (q(&p.wifi), q(&p.mobile), q(&p.download)))
    }
}

impl Client {
    /// [`Self::precache_targets`] over `ids`; `held` gives each id's download state.
    fn fetches(&self, ids: Vec<ItemKey>, metered: bool, wifi: &StreamQuality, mobile: &StreamQuality, held: impl Fn(&ItemKey) -> crate::transfers::HeldState) -> Vec<Fetch> {
        self.core.session.precache_list(ids, |id| held(id) != crate::transfers::HeldState::Absent)
            .into_iter()
            .filter_map(|id| {
                let t = self.stream_target(&id, metered, wifi.clone(), mobile.clone())?;
                Some(Fetch { id, url: t.url, key: t.key })
            })
            .collect()
    }

    /// `f` asked of the client of `song`'s source; None for a source the profile does not have.
    pub(crate) fn in_song_source<R>(&self, song: &ItemKey, f: impl FnOnce(&Client) -> R) -> Option<R> {
        let other = self.other_source(song).ok()?;
        Some(f(other.as_deref().unwrap_or(self)))
    }

    /// The client of `song`'s source when that is another than this client's (None: this one's own).
    pub(crate) fn other_source(&self, song: &ItemKey) -> crate::client::NetResult<Option<std::sync::Arc<Client>>> {
        song.source.as_ref().map(|s| self.core.source(s).ok_or_else(|| crate::providers::nowhere("such source"))).transpose()
    }

    /// The network's quality, capped on the second address (as opus unless a format is set).
    fn quality(&self, metered: bool, wifi: StreamQuality, mobile: StreamQuality) -> StreamQuality {
        let q = if metered { mobile } else { wifi };
        let cap = self.profile.read().alt_max_bit_rate;
        if self.on_second_address() && cap > 0 && (q.bit_rate == 0 || q.bit_rate > cap) {
            return StreamQuality { bit_rate: cap, format: if q.format.is_empty() { "opus".into() } else { q.format } };
        }
        q
    }

    /// The quality [`Self::resolve`] streams at on this network.
    pub fn streaming_quality(&self, metered: bool) -> StreamQuality {
        let (wifi, mobile, _) = self.saved_qualities();
        self.quality(metered, wifi, mobile)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// The cache key [`Self::stream_target`] would give; None for a song of a source the profile does
    /// not have.
    pub fn stream_key(&self, id: ItemKey, metered: bool, wifi: StreamQuality, mobile: StreamQuality) -> Option<String> {
        self.stream_target(&id, metered, wifi, mobile).map(|t| t.key)
    }

    /// Where `id` opens: the download (`downloaded`) at download quality, else a stream for the network;
    /// None for a song of a source the profile does not have.
    pub fn resolve(&self, id: ItemKey, downloaded: bool, metered: bool) -> Option<StreamTarget> {
        let (wifi, mobile, download) = self.saved_qualities();
        if downloaded {
            self.download_target(id, download)
        } else {
            self.stream_target(&id, metered, wifi, mobile)
        }
    }

    /// The URL and cache key of `id`'s download, through its source; None for a source the profile does
    /// not have.
    pub fn download_target(&self, id: ItemKey, quality: StreamQuality) -> Option<StreamTarget> {
        let url = self.in_song_source(&id, |c| c.core.provider().stream(c, id.id.clone(), quality).url)?;
        Some(StreamTarget { url, key: download_key(id) })
    }
}

impl Client {
    /// The URL and cache key to stream `id`, through the source it comes from, at that source's quality
    /// on this network.
    pub(crate) fn stream_target(&self, id: &ItemKey, metered: bool, wifi: StreamQuality, mobile: StreamQuality) -> Option<StreamTarget> {
        let s = self.in_song_source(id, |c| c.core.provider().stream(c, id.id.clone(), c.quality(metered, wifi, mobile)))?;
        Some(StreamTarget { url: s.url, key: stream_cache_key(id, &s.quality) })
    }

    /// The songs to prefetch now ([`nori_queue::Session::precache`], minus downloaded or queued downloads),
    /// with URL and key.
    pub fn precache_targets(&self, metered: bool) -> Vec<Fetch> {
        let (wifi, mobile, _) = self.saved_qualities();
        let ids = self.core.session.precache(metered);
        // Past the next song (which the player fetches itself), a song's provider may take fewer than the
        // settings.
        let ids = ids
            .into_iter()
            .filter(|id| match self.in_song_source(id, |c| c.core.provider().ahead()).flatten() {
                Some(ahead) => self.core.session.upcoming(1 + ahead as u32).iter().skip(1).any(|n| n == id),
                None => true,
            })
            .collect();
        self.fetches(ids, metered, &wifi, &mobile, |id| self.core.transfers().held().state(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ItemKey;
    use crate::client::tests::{block, client, two_addresses};
    use crate::client::NetProfile;
    use crate::transport::FailureKind;

    fn q(bit_rate: u32, format: &str) -> StreamQuality {
        StreamQuality { bit_rate, format: format.into() }
    }

    #[test]
    fn stream_quality() {
        let (c, fake) = client(NetProfile { alt_max_bit_rate: 128, ..two_addresses() });
        let t = c.stream_target(&ItemKey::own("s1"), false, q(0, ""), q(192, "opus")).unwrap();
        assert_eq!(t.key, "s1:0");
        assert!(t.url.contains("/rest/stream?") && t.url.ends_with("&id=s1"));
        assert_eq!(c.stream_key(ItemKey::own("s1"), true, q(0, ""), q(192, "opus")).unwrap(), "s1:192opus");

        fake.fail(FailureKind::Connect);
        assert!(block(c.choose_address()));
        let t = c.stream_target(&ItemKey::own("s1"), false, q(0, ""), q(192, "opus")).unwrap();
        assert_eq!(t.key, "s1:128opus");
        assert!(t.url.starts_with("https://wan.example/rest/stream?") && t.url.ends_with("&id=s1&maxBitRate=128&format=opus"));
        assert_eq!(c.stream_key(ItemKey::own("s1"), false, q(96, "mp3"), q(0, "")).unwrap(), "s1:96mp3", "under the cap");
        assert_eq!(c.stream_key(ItemKey::own("s1"), false, q(320, "mp3"), q(0, "")).unwrap(), "s1:128mp3");
    }

    #[test]
    fn resolve_now_follows_network_and_downloads() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let dir = nori_testdir::TempDir::new("resolve-now");
        c.session().settings.open(&dir.join("app.db").to_string_lossy()).unwrap();
        c.session().settings.edit_by_name("mobile", "192:opus");
        *fake.metered.lock() = true;
        assert_eq!(c.resolve_now(&ItemKey::own("s1")).unwrap().key, "s1:192opus");
        *fake.metered.lock() = false;
        assert_eq!(c.resolve_now(&ItemKey::own("s1")).unwrap().key, "s1:0");
        c.core.download_queue(vec![crate::Song { id: "s1".into(), ..Default::default() }]).unwrap();
        c.core.download_settle(vec![ItemKey::own("s1")], vec![true]).unwrap();
        *fake.metered.lock() = true;
        assert_eq!(c.resolve_now(&ItemKey::own("s1")).unwrap().key, "dl:s1", "a download plays whatever the network");
    }

    #[test]
    fn prefetch_skips_radio_and_more() {
        let (c, _) = client(NetProfile { url: "h".into(), ..Default::default() });
        let ids = crate::tests::keys(&["a", "radio:1", "ext-2", "queued", "done", "b"]);
        let held = |id: &ItemKey| match id.id.as_str() {
            "queued" => crate::transfers::HeldState::Pending,
            "done" => crate::transfers::HeldState::Done,
            _ => crate::transfers::HeldState::Absent,
        };
        let f = c.fetches(ids, true, &q(0, ""), &q(192, "opus"), held);
        assert_eq!(f.iter().map(|f| (f.id.id.as_str(), f.key.as_str())).collect::<Vec<_>>(), [("a", "a:192opus"), ("b", "b:192opus")]);
        assert!(f[0].url.ends_with("&id=a&maxBitRate=192&format=opus"), "{}", f[0].url);
    }

    #[test]
    fn downloads() {
        let (c, _) = client(NetProfile { url: "h".into(), ..Default::default() });
        assert_eq!(c.resolve(ItemKey::own("s1"), true, true).unwrap().key, "dl:s1");
        assert_eq!(c.resolve(ItemKey::own("s1"), false, false).unwrap().key, "s1:0", "not downloaded: streamed");

        // Download keys and stream copies.
        let (c, _) = client(NetProfile { url: "h".into(), ..Default::default() });
        let t = c.download_target(ItemKey::own("s:1"), q(0, "")).unwrap();
        assert_eq!(t.key, "dl:s:1");
        assert!(t.url.ends_with("&id=s%3A1"));
    }

}
