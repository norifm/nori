//! A plugin's README, as blocks a client draws: from its package, or from its repo before it is
//! installed. Inline marks are dropped and a link keeps its words.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::{Offer, Plugins};

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

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// Plugin `id`'s README: its package's when installed with one, else what `offer` points at.
    /// Empty when it has none or its repo can't be reached.
    pub async fn readme(&self, id: String, offer: Option<Offer>) -> Vec<ReadmeBlock> {
        if let Ok(text) = std::fs::read_to_string(self.folder(&id).join("README.md")) {
            return blocks(&text);
        }
        let (Some(url), Ok(transport)) = (offer.and_then(|o| o.readme), self.transport_now()) else { return Vec::new() };
        match transport.get(url, 0).await {
            Ok(r) if (200..=299).contains(&r.status) => blocks(&String::from_utf8_lossy(&r.body)),
            _ => Vec::new(),
        }
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
