//! Lyrics state for the current song: the core's pick and the `nori_look::lyrics` clock that says which
//! line is active and when to check again.

use nori_core::lyrics_sources::LyricsOrigin;
use nori_core::race::{lyrics_replaces, LyricsPick};
use nori_look::lyrics::LyricClock;
use slint::{ModelRc, SharedString, VecModel};

use crate::sung::{self, Faces, Laid, Part};
use crate::{LyricPiece, SungLine};

/// In a duet, the room each voice leaves clear on the far side, so two singers read as a conversation.
const DUET_LANE: f32 = 48.0;
/// Backing vocals' size against their line's (app.slint draws them so).
const BACKING_SIZE: f32 = 0.72;
/// What is not sung yet is the ink at this alpha, as app.slint draws unsung lines.
const UNSUNG: f32 = 0.36;

pub struct SongLyrics {
    pick: LyricsPick,
    clock: LyricClock,
    faces: Option<Faces>,
    /// Active line as last laid out, per view (side panel, Now Playing) and part (lead, backing).
    laid: [[Option<Laid>; 2]; 2],
    /// The width every line was last broken into rows at, per view.
    rows_at: [Option<f32>; 2],
}

impl SongLyrics {
    pub fn new(pick: LyricsPick, position_ms: i64, faces: Option<Faces>) -> SongLyrics {
        let clock = nori_core::look::clock_on(&pick.lyrics, position_ms);
        SongLyrics { pick, clock, faces, laid: Default::default(), rows_at: [None, None] }
    }

    pub fn lines(&self) -> ModelRc<SungLine> {
        let lines = self.pick.lyrics.lines.iter().map(|l| SungLine {
            text: l.text.as_str().into(),
            backing: l.backing.as_str().into(),
            right: l.voice == 1,
            background: l.background,
        });
        ModelRc::new(VecModel::from(lines.collect::<Vec<_>>()))
    }

    /// The room every line leaves clear on its far side: a duet's lane, else none.
    pub fn lane(&self) -> f32 {
        if self.pick.lyrics.lines.iter().any(|l| l.voice == 1) { DUET_LANE } else { 0.0 }
    }

    pub fn pick(&self) -> &LyricsPick {
        &self.pick
    }

    pub fn synced(&self) -> bool {
        self.pick.lyrics.synced
    }

    pub fn is_empty(&self) -> bool {
        self.pick.lyrics.lines.is_empty()
    }

    /// Source credit, a plugin's service by `plugin_name`; empty for the server's own lyrics.
    pub fn credit(&self, plugin_name: &dyn Fn(&str) -> String) -> String {
        match &self.pick.origin {
            LyricsOrigin::Server => String::new(),
            other => format!("Lyrics from {}", crate::words::lyrics_origin(other, plugin_name)),
        }
    }

    /// Whether `next` should replace these (`lyrics_replaces`).
    pub fn replaced_by(&self, next: &LyricsPick) -> bool {
        lyrics_replaces(Some(&self.pick), next)
    }

    /// State at `position_ms`: active line, fill progress, and when to call again. `shown`: a view shows
    /// the lyrics; without one nothing is waited for.
    pub fn advance(&self, position_ms: i64, shown: bool, force: bool) -> Now {
        let step = self.clock.advance(position_ms, shown, true, force);
        let sweeping = self.clock.timing().sweeps();
        // While a word fills, the clock counts 60 Hz frames.
        let wait = if shown { self.clock.wait_ms(&step, true, 16) } else { None };
        let line = usize::try_from(step.frame.active).ok().and_then(|i| self.pick.lyrics.lines.get(i));
        // With a face the pieces draw the fill, so the split is not needed.
        let (sung, now, mix, rest) = match line {
            Some(l) if sweeping && self.faces.is_none() => split(&l.text, step.frame.sung),
            _ => Default::default(),
        };
        Now { active: step.frame.active, sweeping, sung, now, mix, rest, wait, at: step.frame.sung, backing_at: self.clock.backing_sung(), ms: self.clock.shown_ms() }
    }

    /// The pieces of the active line's `part` for `view` (0 side panel, 1 Now Playing), the line drawn at
    /// `size`, what is sung at alpha `lit`; re-laid only when line, size or width change. Empty when it has
    /// no such part.
    pub fn pieces(&mut self, view: usize, part: Part, now: &Now, size: f32, width: f32, lit: f32) -> Vec<LyricPiece> {
        let Some(faces) = &self.faces else { return Vec::new() };
        let width = width - self.lane();
        let Some((i, line)) = usize::try_from(now.active).ok().and_then(|i| self.pick.lyrics.lines.get(i).map(|l| (i, l))) else { return Vec::new() };
        let (text, words) = part.of(line);
        if text.is_empty() {
            return Vec::new();
        }
        let (size, sung) = match part {
            Part::Lead => (size, now.at),
            Part::Backing => (size * BACKING_SIZE, now.backing_at),
        };
        let slot = &mut self.laid[view][part as usize];
        if !slot.as_ref().is_some_and(|l| l.is_for(i, part, size, width)) {
            *slot = Some(sung::lay(faces, i, line, part, size, width));
        }
        slot.as_ref().map_or_else(Vec::new, |l| sung::frame(l, words, sung, now.ms, lit, UNSUNG, size * 0.6))
    }

    /// Every line and its backing vocals broken into the rows their pieces take for `view` at `size` within
    /// `width`, when that width is new; None when the lines are already so, or there is no face to measure in.
    pub fn rows(&mut self, view: usize, size: f32, width: f32) -> Option<[ModelRc<SharedString>; 2]> {
        let faces = self.faces.as_ref()?;
        let width = width - self.lane();
        if self.rows_at[view] == Some(width) {
            return None;
        }
        self.rows_at[view] = Some(width);
        let lines = &self.pick.lyrics.lines;
        let rows = |part: Part, size: f32| {
            let rows = lines.iter().enumerate().map(|(i, l)| SharedString::from(sung::lay(faces, i, l, part, size, width).rows(part.of(l).0)));
            ModelRc::new(VecModel::from(rows.collect::<Vec<_>>()))
        };
        Some([rows(Part::Lead, size), rows(Part::Backing, size * BACKING_SIZE)])
    }

    /// Seek target for a click on `line`.
    pub fn tap(&self, line: usize) -> i64 {
        self.clock.tap(line)
    }
}

/// Lyrics state at one moment ([`SongLyrics::advance`]).
pub struct Now {
    pub active: i32,
    /// Word-by-word fill: the active line is `sung` + `now` (the current character, `mix` filled) + `rest`.
    pub sweeping: bool,
    pub sung: String,
    pub now: String,
    pub mix: f32,
    pub rest: String,
    pub wait: Option<u64>,
    /// Fill position in the active line and in its backing vocals (UTF-16 units), and the lyric-time ms it
    /// corresponds to.
    pub at: f32,
    pub backing_at: f32,
    pub ms: i64,
}

/// Splits `text` at `at` UTF-16 units (7.5 = half of char 7): before, current char, its fill, after.
fn split(text: &str, at: f32) -> (String, String, f32, String) {
    let mut units = 0.0f32;
    for (i, c) in text.char_indices() {
        let w = c.len_utf16() as f32;
        if at < units + w {
            let end = i + c.len_utf8();
            return (text[..i].to_string(), text[i..end].to_string(), ((at - units) / w).clamp(0.0, 1.0), text[end..].to_string());
        }
        units += w;
    }
    (text.to_string(), String::new(), 0.0, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::{LyricLine, LyricWord, Lyrics};

    fn pick() -> LyricsPick {
        let line = |start_ms, end_ms, text: &str| LyricLine { start_ms, end_ms, text: text.into(), ..Default::default() };
        let lyrics = Lyrics { synced: true, word_timed: false, lines: vec![line(1000, 2000, "one"), line(3000, 4000, "two")], ..Default::default() };
        LyricsPick { lyrics, origin: LyricsOrigin::Server }
    }

    #[test]
    fn lyric_lines() {
        let l = SongLyrics::new(pick(), 0, None);
        assert_eq!(l.advance(1500, true, true).active, 0);
        assert_eq!(l.advance(3500, true, true).active, 1);
        let wait = l.advance(2500, true, true).wait;
        assert!(wait.is_some_and(|ms| ms <= 500), "the next line is due at 3000");

        // Split at fill position.
        assert_eq!(split("héllo", 1.5), ("h".into(), "é".into(), 0.5, "llo".into()));
        assert_eq!(split("ab", 0.0), ("".into(), "a".into(), 0.0, "b".into()));
        assert_eq!(split("ab", 2.0), ("ab".into(), "".into(), 0.0, "".into()));

        // Timed words fill the line.
        let word = |start_ms, end_ms, start, end| LyricWord { start_ms, end_ms, start, end };
        let line = LyricLine { start_ms: 1000, end_ms: 3000, text: "one two".into(), words: vec![word(1000, 2000, 0, 3), word(2000, 3000, 4, 7)], ..Default::default() };
        let next = LyricLine { start_ms: 6000, end_ms: 7000, text: "three".into(), words: vec![word(6000, 7000, 0, 5)], ..Default::default() };
        let lyrics = Lyrics { synced: true, word_timed: true, lines: vec![line, next], ..Default::default() };
        let l = SongLyrics::new(LyricsPick { lyrics, origin: LyricsOrigin::Server }, 0, None);
        let now = l.advance(2500, true, true);
        assert!(now.sweeping);
        assert_eq!((now.sung.as_str(), now.now.as_str(), now.rest.as_str()), ("one t", "w", "o"));
        assert!(now.wait.is_some_and(|ms| ms <= 16), "a word is filling");

        // With no view showing the lyrics, nothing is waited for.
        assert_eq!(l.advance(2500, false, true).wait, None);
    }

    #[test]
    fn a_face_draws_the_pieces_not_the_split_text() {
        let word = |start_ms, end_ms, start, end| LyricWord { start_ms, end_ms, start, end };
        let line = LyricLine { start_ms: 1000, end_ms: 3000, text: "one two".into(), words: vec![word(1000, 2000, 0, 3), word(2000, 3000, 4, 7)], ..Default::default() };
        let lyrics = Lyrics { synced: true, word_timed: true, lines: vec![line], ..Default::default() };
        let l = SongLyrics::new(LyricsPick { lyrics, origin: LyricsOrigin::Server }, 0, Faces::inter());
        let now = l.advance(2500, true, true);
        assert!(now.sweeping);
        assert_eq!((now.sung.as_str(), now.now.as_str(), now.rest.as_str()), ("", "", ""));
    }

    /// Backing vocals sung over a line fill as they are sung, apart from the line's own words.
    #[test]
    fn backing_vocals_fill_as_they_are_sung() {
        let word = |start_ms, end_ms, start, end| LyricWord { start_ms, end_ms, start, end };
        let line = LyricLine {
            start_ms: 1000,
            end_ms: 3000,
            text: "one two".into(),
            words: vec![word(1000, 2000, 0, 3), word(2000, 3000, 4, 7)],
            backing: "(oh oh)".into(),
            backing_words: vec![word(1000, 1200, 0, 3), word(2600, 3000, 4, 7)],
            ..Default::default()
        };
        let lyrics = Lyrics { synced: true, word_timed: true, lines: vec![line], ..Default::default() };
        let mut l = SongLyrics::new(LyricsPick { lyrics, origin: LyricsOrigin::Server }, 0, Faces::inter());
        let now = l.advance(1500, true, true);
        let lead = l.pieces(0, Part::Lead, &now, 22.0, 300.0, 1.0);
        let backing = l.pieces(0, Part::Backing, &now, 22.0, 300.0, 0.7);
        let texts: Vec<&str> = backing.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["(oh", " ", "oh)"]);
        assert_eq!((backing[0].a0, backing[2].a1), (0.7, 0.36), "the first sung, the second not yet");
        assert_eq!((lead[0].a0, lead[0].a1), (1.0, 0.36), "the lead is part way through its first word");
    }

}
