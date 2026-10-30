//! A plugin's README, as blocks a client draws, and its icon: from its package, or from its repo before
//! it is installed. Inline marks are dropped and a link keeps its words.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

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

fn blocks(markdown: &str) -> Vec<ReadmeBlock> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut items = 0;
    for event in Parser::new(markdown) {
        match event {
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak => text.push(' '),
            Event::HardBreak => text.push('\n'),
            Event::Start(Tag::Item) => items += 1,
            // An item's own words end where a list inside it starts.
            Event::Start(Tag::List(_)) if !text.is_empty() => out.push(ReadmeBlock::Item { text: std::mem::take(&mut text) }),
            Event::End(TagEnd::Item) => {
                items -= 1;
                if !text.is_empty() {
                    out.push(ReadmeBlock::Item { text: std::mem::take(&mut text) });
                }
            }
            Event::End(TagEnd::Heading(level)) => out.push(ReadmeBlock::Heading { level: level as u8, text: std::mem::take(&mut text) }),
            Event::End(TagEnd::Paragraph) if items == 0 => out.push(ReadmeBlock::Text { text: std::mem::take(&mut text) }),
            Event::End(TagEnd::CodeBlock) => out.push(ReadmeBlock::Code { text: std::mem::take(&mut text).trim_end().to_string() }),
            _ => {}
        }
    }
    // The page already names it.
    if matches!(out.first(), Some(ReadmeBlock::Heading { level: 1, .. })) {
        out.remove(0);
    }
    out
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
        self.shown_file(&id, "README.md", offer.and_then(|o| o.readme)).await.map_or_else(Vec::new, |b| blocks(&String::from_utf8_lossy(&b)))
    }

    /// Plugin `id`'s icon, a PNG a client draws in the page's ink; None without one, or for what isn't one
    /// or is past [`ICON_BYTES`].
    pub async fn icon(&self, id: String, offer: Option<Offer>) -> Option<Vec<u8>> {
        self.shown_file(&id, "icon.png", offer.and_then(|o| o.icon)).await.filter(|b| b.len() <= ICON_BYTES && b.starts_with(PNG))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_blocks() {
        let text = |t: &str| t.to_string();
        let got = blocks("# Orbit\n\nA *moving* [cover](http://x)\nline.\n\n- one\n- two\n  - under\n\n```\nlet a;\n```\n\n<b>html</b>\n\n## End\n");
        assert_eq!(
            got,
            [
                ReadmeBlock::Text { text: text("A moving cover line.") },
                ReadmeBlock::Item { text: text("one") },
                ReadmeBlock::Item { text: text("two") },
                ReadmeBlock::Item { text: text("under") },
                ReadmeBlock::Code { text: text("let a;") },
                ReadmeBlock::Text { text: text("html") },
                ReadmeBlock::Heading { level: 2, text: text("End") },
            ]
        );
    }
}
