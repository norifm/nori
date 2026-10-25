//! Word-by-word layout of the active lyric line, as Android's SungText draws it: a feathered fill, each
//! word lifting while sung, held notes swelling. Slint cannot vary colour or offset within one text, so
//! the line is measured with Skia here and each piece becomes its own text.

use nori_core::{LyricLine, LyricWord};
use nori_look::lyrics::{GLOW_FADE_MS, HELD_MS, RISE_MIN_MS, SETTLE_MS};
use skia_safe::font_arguments::variation_position::Coordinate;
use skia_safe::font_arguments::VariationPosition;
use skia_safe::{Font, FontArguments, FontMgr, FourByteTag, Typeface};

use crate::LyricPiece;

/// A laid-out piece: a timed word (`word` is its index) or the text between words.
struct Placed {
    text: slint::SharedString,
    /// Span in the line's text, UTF-16 units.
    start: u32,
    end: u32,
    x: f32,
    row: u32,
    w: f32,
    word: Option<usize>,
}

/// What of a line is laid out: what is sung, or the backing vocals sung over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Lead,
    Backing,
}

impl Part {
    /// The part's text and its word times.
    pub fn of(self, line: &LyricLine) -> (&str, &[LyricWord]) {
        match self {
            Part::Lead => (&line.text, &line.words),
            Part::Backing => (&line.backing, &line.backing_words),
        }
    }
}

/// A line laid out at one size and width.
pub struct Laid {
    key: (usize, Part, u32, u32),
    pieces: Vec<Placed>,
    row_h: f32,
}

/// Inter, which app.slint bundles and draws every text in: the pieces are measured in the face they are drawn in.
const INTER: &[u8] = include_bytes!("../../../app/src/main/res/font/inter.ttf");

/// Inter, read from the same file the window draws with, at the two weights lines are drawn in. Made once:
/// each variation is a new typeface, and Skia keeps a font face and glyph cache for every typeface.
#[derive(Clone)]
pub struct Faces {
    bold: Typeface,
    regular: Typeface,
}

impl Faces {
    pub fn inter() -> Option<Faces> {
        let face = FontMgr::new().new_from_data(skia_safe::Data::new_copy(INTER), None)?;
        Some(Faces { bold: weighted(&face, 700.0), regular: weighted(&face, 400.0) })
    }

    /// Backing vocals, alone on their line or sung over it, are drawn lighter.
    fn font(&self, line: &LyricLine, part: Part, size: f32) -> Font {
        Font::from_typeface(if line.background || part == Part::Backing { &self.regular } else { &self.bold }.clone(), size)
    }
}

/// `face` at `weight`, at the optical size Slint draws it in: the variable font's default (Inter's text
/// cut, wider than its display cut).
fn weighted(face: &Typeface, weight: f32) -> Typeface {
    let coords = [Coordinate { axis: FourByteTag::from_chars('w', 'g', 'h', 't'), value: weight }];
    let args = FontArguments::new().set_variation_design_position(VariationPosition { coordinates: &coords });
    if face.variation_design_parameters().is_some_and(|a| !a.is_empty()) { face.clone_with_arguments(&args).unwrap_or_else(|| face.clone()) } else { face.clone() }
}

fn units(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Byte offset of UTF-16 unit `u` in `s`.
fn byte_at(s: &str, u: u32) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= u {
            return i;
        }
        n += c.len_utf16() as u32;
    }
    s.len()
}

/// Lays out `part` of line `index` at `size` px within `width`, wrapping at spaces like the text does; the
/// second voice's rows end at the right edge, as its text is aligned.
pub fn lay(faces: &Faces, index: usize, line: &LyricLine, part: Part, size: f32, width: f32) -> Laid {
    let key = (index, part, size.to_bits(), width.to_bits());
    let font = faces.font(line, part, size);
    let (spacing, _) = font.metrics();
    let measure = |s: &str| font.measure_str(s, None).0;
    let (text, words) = part.of(line);
    let total = units(text);
    // Cut at word edges: one piece per timed word, plus the gaps.
    let mut cuts: Vec<(u32, u32, Option<usize>)> = Vec::new();
    let mut at = 0;
    let mut words: Vec<(usize, &LyricWord)> = words.iter().enumerate().filter(|(_, w)| w.start < w.end && w.end <= total).collect();
    words.sort_by_key(|(_, w)| w.start);
    for (k, w) in words {
        if w.start < at {
            continue;
        }
        if w.start > at {
            cuts.push((at, w.start, None));
        }
        cuts.push((w.start, w.end, Some(k)));
        at = w.end;
    }
    if at < total {
        cuts.push((at, total, None));
    }
    // Split at spaces so wrapping can break there; a space stays with the piece before it.
    let mut parts: Vec<(u32, u32, Option<usize>)> = Vec::new();
    for (s, e, k) in cuts {
        let piece = &text[byte_at(text, s)..byte_at(text, e)];
        let mut from = s;
        let mut u = s;
        let mut chars = piece.chars().peekable();
        while let Some(c) = chars.next() {
            u += c.len_utf16() as u32;
            if c.is_whitespace() && chars.peek().is_some_and(|n| !n.is_whitespace()) {
                parts.push((from, u, k));
                from = u;
            }
        }
        if from < e {
            parts.push((from, e, k));
        }
    }
    // Greedy wrap by runs between spaces.
    let mut pieces = Vec::new();
    let mut x = 0.0;
    let mut row = 0;
    let mut i = 0;
    while i < parts.len() {
        // A run ends with the part that ends in a space.
        let mut j = i;
        loop {
            let (s, e, _) = parts[j];
            let t = &text[byte_at(text, s)..byte_at(text, e)];
            if t.ends_with(char::is_whitespace) || j + 1 == parts.len() {
                break;
            }
            j += 1;
        }
        let run: Vec<(String, u32, u32, Option<usize>)> = parts[i..=j].iter().map(|&(s, e, k)| (text[byte_at(text, s)..byte_at(text, e)].to_string(), s, e, k)).collect();
        let inked: f32 = run.iter().map(|(t, ..)| measure(t)).sum::<f32>() - run.last().map_or(0.0, |(t, ..)| measure(t) - measure(t.trim_end()));
        if x > 0.0 && x + inked > width {
            x = 0.0;
            row += 1;
        }
        for (t, s, e, k) in run {
            let w = measure(&t);
            pieces.push(Placed { text: t.into(), start: s, end: e, x, row, w, word: k });
            x += w;
        }
        i = j + 1;
    }
    if line.voice == 1 {
        let mut row_end = vec![0.0f32; pieces.last().map_or(0, |p: &Placed| p.row as usize + 1)];
        for p in &pieces {
            row_end[p.row as usize] = row_end[p.row as usize].max(p.x + measure(p.text.trim_end()));
        }
        for p in &mut pieces {
            p.x += width - row_end[p.row as usize];
        }
    }
    Laid { key, pieces, row_h: spacing }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Lift of a word, 0..1: rises while sung (over at least `RISE_MIN_MS`), settles over `SETTLE_MS` after.
fn lift(w: &LyricWord, ms: i64) -> f32 {
    if ms <= w.start_ms {
        return 0.0;
    }
    let up = ease((ms - w.start_ms) as f32 / (w.end_ms - w.start_ms).max(RISE_MIN_MS) as f32);
    let down = if ms <= w.end_ms { 0.0 } else { ease((ms - w.end_ms) as f32 / SETTLE_MS as f32) };
    up * (1.0 - down)
}

/// Swell of a note held at least `HELD_MS`, 0..1: grows while held, fades over `GLOW_FADE_MS` after.
fn held(w: &LyricWord, ms: i64) -> f32 {
    let long = w.end_ms - w.start_ms;
    if long < HELD_MS || ms <= w.start_ms {
        return 0.0;
    }
    if ms < w.end_ms {
        ease((ms - w.start_ms) as f32 / long as f32)
    } else {
        1.0 - ease((ms - w.end_ms) as f32 / GLOW_FADE_MS as f32)
    }
}

/// The pieces of `words`' text at `ms` with the fill `sung` UTF-16 units in: alpha `lit` before it, `dim`
/// after, over a `feather` px edge.
pub fn frame(laid: &Laid, words: &[LyricWord], sung: f32, ms: i64, lit: f32, dim: f32, feather: f32) -> Vec<LyricPiece> {
    // Row and x of the fill edge.
    let (row_s, x_s) = laid
        .pieces
        .iter()
        .find(|p| sung < p.end as f32)
        .map(|p| (p.row, p.x + p.w * ((sung - p.start as f32) / (p.end - p.start).max(1) as f32).clamp(0.0, 1.0)))
        .unwrap_or((u32::MAX, f32::MAX));
    let alpha = |row: u32, x: f32| {
        if row < row_s {
            lit
        } else if row > row_s {
            dim
        } else {
            let t = ((x - (x_s - feather / 2.0)) / feather).clamp(0.0, 1.0);
            lit + (dim - lit) * t
        }
    };
    laid.pieces
        .iter()
        .map(|p| {
            let w = p.w.max(1.0);
            let a = ((x_s - feather / 2.0 - p.x) / w).clamp(0.0, 1.0);
            let b = ((x_s + feather / 2.0 - p.x) / w).clamp(0.0, 1.0);
            let word = p.word.and_then(|k| words.get(k));
            LyricPiece {
                text: p.text.clone(),
                x: p.x,
                y: p.row as f32 * laid.row_h,
                w: p.w,
                a,
                b,
                a0: alpha(p.row, p.x),
                aa: alpha(p.row, p.x + a * w),
                ab: alpha(p.row, p.x + b * w),
                a1: alpha(p.row, p.x + w),
                lift: word.map_or(0.0, |w| lift(w, ms)),
                swell: word.map_or(0.0, |w| held(w, ms)),
            }
        })
        .collect()
}

impl Laid {
    pub fn is_for(&self, index: usize, part: Part, size: f32, width: f32) -> bool {
        self.key == (index, part, size.to_bits(), width.to_bits())
    }

    /// `text` (the line laid) with a line end for the space at each of its row breaks: drawn so, a line
    /// takes the rows its pieces take, sung or not.
    pub fn rows(&self, text: &str) -> String {
        let mut out: Vec<char> = text.chars().collect();
        let mut at = Vec::new();
        for pair in self.pieces.windows(2) {
            if pair[1].row > pair[0].row {
                at.push(pair[1].start);
            }
        }
        let mut units = 0;
        for c in out.iter_mut() {
            units += c.len_utf16() as u32;
            if at.contains(&units) && c.is_whitespace() {
                *c = '\n';
            }
        }
        out.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> LyricLine {
        let w = |start_ms, end_ms, start, end| LyricWord { start_ms, end_ms, start, end };
        // "won" and "der" are one word sung in two syllables.
        LyricLine { start_ms: 0, end_ms: 3000, text: "I wonder why".into(), words: vec![w(0, 500, 0, 1), w(500, 900, 2, 5), w(900, 1400, 5, 8), w(1400, 3000, 9, 12)], ..Default::default() }
    }

    fn faces() -> Faces {
        Faces::inter().expect("a font")
    }

    #[test]
    fn words_are_measured_in_the_face_the_window_draws() {
        assert_eq!(faces().font(&line(), Part::Lead, 20.0).typeface().family_name(), "Inter Variable");
    }

    #[test]
    fn lines_are_measured_in_one_typeface_per_weight() {
        let faces = faces();
        let id = |line: &LyricLine| faces.font(line, Part::Lead, 20.0).typeface().unique_id();
        let backing = LyricLine { background: true, ..line() };
        assert_eq!(id(&line()), id(&line()));
        assert_eq!(id(&backing), id(&backing));
        assert_ne!(id(&line()), id(&backing));
    }

    #[test]
    fn line_breaks() {
        let l = lay(&faces(), 0, &line(), Part::Lead, 20.0, 1000.0);
        let texts: Vec<&str> = l.pieces.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["I", " ", "won", "der", " ", "why"]);
        assert_eq!(l.pieces.iter().map(|p| p.row).max(), Some(0));
        assert!(l.pieces.windows(2).all(|w| w[1].x > w[0].x));

        // Narrow line wraps at spaces.
        let l = lay(&faces(), 0, &line(), Part::Lead, 20.0, 40.0);
        assert!(l.pieces.iter().map(|p| p.row).max().unwrap_or(0) > 0);
        assert!(l.pieces.iter().filter(|p| p.x == 0.0).all(|p| !p.text.starts_with(' ')));
    }

    #[test]
    fn the_second_voice_ends_its_rows_at_the_right_edge() {
        let line = LyricLine { voice: 1, ..line() };
        let font = faces().font(&line, Part::Lead, 20.0);
        for width in [1000.0, 60.0] {
            let l = lay(&faces(), 0, &line, Part::Lead, 20.0, width);
            for row in 0..=l.pieces.iter().map(|p| p.row).max().unwrap() {
                let end = l.pieces.iter().filter(|p| p.row == row).map(|p| p.x + font.measure_str(p.text.trim_end(), None).0).fold(0.0, f32::max);
                assert!((end - width).abs() < 0.01, "row {row} at width {width} ends at {end}");
            }
        }
    }

    /// A line drawn as rows breaks where its pieces wrap: the space before each row's first piece is a line
    /// end, and the text is the line's otherwise.
    #[test]
    fn rows_break_where_the_pieces_wrap() {
        let line = line();
        let wide = lay(&faces(), 0, &line, Part::Lead, 20.0, 1000.0);
        assert_eq!(wide.rows(&line.text), "I wonder why");
        let narrow = lay(&faces(), 0, &line, Part::Lead, 20.0, 40.0);
        let rows = narrow.rows(&line.text);
        let starts: Vec<u32> = narrow.pieces.iter().filter(|p| p.x == 0.0).map(|p| p.start).collect();
        let mut at = 0;
        let row_starts: Vec<u32> = rows.split('\n').map(|r| { let s = at; at += units(r) + 1; s }).collect();
        assert!(starts.len() > 1);
        assert_eq!(row_starts, starts, "each row starts where the layout starts one");
        assert_eq!(rows.replace('\n', " "), line.text);
    }

    #[test]
    fn frames_share_the_piece_text() {
        let line = line();
        let l = lay(&faces(), 0, &line, Part::Lead, 20.0, 1000.0);
        let (a, b) = (frame(&l, &line.words, 1.0, 100, 1.0, 0.3, 4.0), frame(&l, &line.words, 6.0, 900, 1.0, 0.3, 4.0));
        assert!(a.iter().zip(&b).all(|(x, y)| x.text.as_ptr() == y.text.as_ptr()), "a frame reuses the laid-out text");
    }

    #[test]
    fn sung_words() {
        let w = LyricWord { start_ms: 1000, end_ms: 1500, start: 0, end: 3 };
        assert_eq!(lift(&w, 900), 0.0);
        assert!(lift(&w, 1400) > 0.5);
        assert_eq!(lift(&w, 1500 + SETTLE_MS), 0.0);
        let long = LyricWord { start_ms: 0, end_ms: HELD_MS + 100, start: 0, end: 3 };
        assert!(held(&long, HELD_MS) > 0.5);
        assert_eq!(held(&w, 1200), 0.0);

        // Fill is lit behind and dim ahead.
        let line = line();
        let l = lay(&faces(), 0, &line, Part::Lead, 20.0, 1000.0);
        let f = frame(&l, &line.words, 6.0, 1000, 1.0, 0.3, 4.0);
        assert_eq!(f.first().map(|p| p.a0), Some(1.0));
        assert_eq!(f.last().map(|p| p.a1), Some(0.3));
    }

}
