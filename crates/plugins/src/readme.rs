//! A plugin's README, as blocks a client draws, and its icon: from its package, or from its repo before
//! it is installed.

use crate::{Offer, Plugins};

/// A repo's file: the most an icon may be.
const ICON_BYTES: usize = 256 * 1024;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ReadmeBlock {
    Heading { level: u8, text: String },
    Text { text: String },
    Item { text: String },
    Code { text: String },
}

impl From<nori_plugin_index::readme::Block> for ReadmeBlock {
    fn from(b: nori_plugin_index::readme::Block) -> ReadmeBlock {
        use nori_plugin_index::readme::Block;
        match b {
            Block::Heading { level, text } => ReadmeBlock::Heading { level, text },
            Block::Text { text } => ReadmeBlock::Text { text },
            Block::Item { text } => ReadmeBlock::Item { text },
            Block::Code { text } => ReadmeBlock::Code { text },
        }
    }
}

impl Plugins {
    /// Plugin `id`'s `file`, from its package when installed with one, else from `url`.
    async fn shown_file(&self, id: &str, file: &str, url: Option<String>) -> Option<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(self.folder(id).join(file)) {
            return Some(bytes);
        }
        let r = self.transport_now().ok()?.get(url?, 0).await.ok()?;
        (200..=299).contains(&r.status).then_some(r.body)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Plugin `id`'s README; empty when it has none or its repo can't be reached.
    pub async fn readme(&self, id: String, offer: Option<Offer>) -> Vec<ReadmeBlock> {
        self.shown_file(&id, "README.md", offer.and_then(|o| o.readme)).await.map_or_else(Vec::new, |b| nori_plugin_index::readme::blocks(&String::from_utf8_lossy(&b)).into_iter().map(ReadmeBlock::from).collect())
    }

    /// Plugin `id`'s icon, a PNG a client draws in the page's ink; None without one, or for what isn't one
    /// or is past [`ICON_BYTES`].
    pub async fn icon(&self, id: String, offer: Option<Offer>) -> Option<Vec<u8>> {
        self.shown_file(&id, "icon.png", offer.and_then(|o| o.icon)).await.filter(|b| b.len() <= ICON_BYTES && b.starts_with(PNG))
    }
}
