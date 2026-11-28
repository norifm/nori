//! What tells an item apart in a profile of several sources: the source it comes from and its id there.
//! The queue, the engine's streams and the lists all carry it whole; it becomes a string only where a
//! value must cross as one ([`ItemKey::text`]).

use alloc::borrow::Cow;
use core::fmt;

use crate::prelude::*;

/// An item as a profile of several sources tells it apart: the source it comes from (None: the
/// profile's own) and its id there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey {
    pub source: Option<String>,
    pub id: String,
}

impl ItemKey {
    /// Between a key's id and its source in [`ItemKey::text`]: a control character no id or saved
    /// profile id holds.
    pub const SOURCE_MARK: char = '\u{1f}';

    /// Item `id` of the profile's own source.
    pub fn own(id: impl Into<String>) -> ItemKey {
        ItemKey { source: None, id: id.into() }
    }

    /// The id the profile's own source knows the item by: None for another source's item. What its
    /// server and other devices are told of speaks of the profile's own items only.
    pub fn own_id(&self) -> Option<&str> {
        self.source.is_none().then_some(self.id.as_str())
    }

    /// The key as one string, where only a string crosses (a platform player's item id, a stored row, a
    /// cache key): the id alone for the profile's own source, so nothing stored before changes meaning.
    pub fn text(&self) -> Cow<'_, str> {
        match &self.source {
            None => Cow::Borrowed(&self.id),
            Some(source) => Cow::Owned(format!("{}{}{source}", self.id, Self::SOURCE_MARK)),
        }
    }

    /// The key [`ItemKey::text`] wrote.
    pub fn from_text(text: &str) -> ItemKey {
        match text.split_once(Self::SOURCE_MARK) {
            Some((id, source)) => ItemKey { source: Some(source.to_string()), id: id.to_string() },
            None => ItemKey::own(text),
        }
    }
}

/// For logs: the id, after its source when another's.
impl fmt::Display for ItemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            None => f.write_str(&self.id),
            Some(source) => write!(f, "{source}/{}", self.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_keeps_own_ids_as_they_were() {
        for (key, text) in [
            (ItemKey::own("a1b2"), "a1b2"),
            (ItemKey::own("ext-deezer-1"), "ext-deezer-1"),
            (ItemKey::own("radio:3"), "radio:3"),
            (ItemKey { source: Some("nd2".into()), id: "a1b2".into() }, "a1b2\u{1f}nd2"),
            (ItemKey { source: Some("yt".into()), id: "s:1/x".into() }, "s:1/x\u{1f}yt"),
        ] {
            assert_eq!(key.text(), text);
            assert_eq!(ItemKey::from_text(text), key, "{text}");
        }
        assert_ne!(ItemKey::own("1"), ItemKey { source: Some("b".into()), id: "1".into() }, "one id, two sources");
    }
}
