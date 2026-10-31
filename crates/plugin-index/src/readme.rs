//! A plugin's README as blocks a client draws. Inline marks are dropped and a link keeps its words.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
pub enum Block {
    Heading { level: u8, text: String },
    Text { text: String },
    Item { text: String },
    Code { text: String },
}

pub fn blocks(markdown: &str) -> Vec<Block> {
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
            Event::Start(Tag::List(_)) if !text.is_empty() => out.push(Block::Item { text: std::mem::take(&mut text) }),
            Event::End(TagEnd::Item) => {
                items -= 1;
                if !text.is_empty() {
                    out.push(Block::Item { text: std::mem::take(&mut text) });
                }
            }
            Event::End(TagEnd::Heading(level)) => out.push(Block::Heading { level: level as u8, text: std::mem::take(&mut text) }),
            Event::End(TagEnd::Paragraph) if items == 0 => out.push(Block::Text { text: std::mem::take(&mut text) }),
            Event::End(TagEnd::CodeBlock) => out.push(Block::Code { text: std::mem::take(&mut text).trim_end().to_string() }),
            _ => {}
        }
    }
    // The page already names it.
    if matches!(out.first(), Some(Block::Heading { level: 1, .. })) {
        out.remove(0);
    }
    out
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
                Block::Text { text: text("A moving cover line.") },
                Block::Item { text: text("one") },
                Block::Item { text: text("two") },
                Block::Item { text: text("under") },
                Block::Code { text: text("let a;") },
                Block::Text { text: text("html") },
                Block::Heading { level: 2, text: text("End") },
            ]
        );
    }
}
