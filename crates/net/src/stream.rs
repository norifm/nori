//! Stream targets and cache keys. Caches are keyed by song (its source and id) and quality, never by URL.

use nori_model::ItemKey;

/// A quality setting: `bit_rate` 0 and an empty `format` mean the original file.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StreamQuality {
    pub bit_rate: u32,
    pub format: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StreamTarget {
    pub url: String,
    /// The cache key: `<song>:<quality>` in the rolling stream cache ([`stream_cache_key`]), `dl:<song>`
    /// for downloads ([`download_key`]).
    pub key: String,
}

/// Where a provider streams a song from, and what its quality is called in a cache key (`192opus`, `0`).
pub struct Stream {
    pub url: String,
    pub quality: String,
}

/// A song to fetch whole into the stream cache ahead of its turn.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Fetch {
    pub id: ItemKey,
    pub url: String,
    pub key: String,
}

/// Song `song`'s key in the rolling stream cache at `quality`; [`ItemKey::text`] keeps a song of the
/// profile's own source under `<id>:<quality>`, as before.
pub fn stream_cache_key(song: &ItemKey, quality: &str) -> String {
    format!("{}:{quality}", song.text())
}

/// A finished download's cache key.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn download_key(id: ItemKey) -> String {
    format!("dl:{}", id.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_of_one_id_in_two_sources_differ_and_own_ones_stay() {
        let (own, theirs) = (ItemKey::own("s1"), ItemKey { source: Some("nd2".into()), id: "s1".into() });
        assert_eq!((stream_cache_key(&own, "192opus"), download_key(own)), ("s1:192opus".into(), "dl:s1".into()));
        assert_eq!((stream_cache_key(&theirs, "0"), download_key(theirs)), ("s1\u{1f}nd2:0".into(), "dl:s1\u{1f}nd2".into()));
    }
}
