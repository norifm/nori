//! The listener's own timing for a song's lyrics: a step sooner or later takes the place of the vocal
//! sync check's offset, kept per song and per set of words, and reset gives the check's back.

use nori_look::lyrics::NUDGE_STEP_MS;
use nori_model::{Lyrics, LyricsTiming};
use nori_settings::lyrics_sources::LyricsOrigin;

/// The timing after one step: `dir` > 0 sooner, < 0 later; 0 back to the check's.
pub fn nudged(t: LyricsTiming, dir: i32) -> LyricsTiming {
    let automatic = automatic(t);
    if dir == 0 {
        return automatic.map_or(LyricsTiming::NotChecked, |ms| LyricsTiming::Automatic { ms });
    }
    let rounded = (t.offset_ms() as f64 / NUDGE_STEP_MS as f64).round() as i64 * NUDGE_STEP_MS;
    LyricsTiming::Yours { ms: rounded + NUDGE_STEP_MS * dir.signum() as i64, automatic }
}

/// `t` with the listener's offset `yours`, when they set one.
pub fn with_yours(t: LyricsTiming, yours: Option<i64>) -> LyricsTiming {
    match yours {
        Some(ms) => LyricsTiming::Yours { ms, automatic: automatic(t) },
        None => t,
    }
}

/// What the sync check gave, none if it has not run.
fn automatic(t: LyricsTiming) -> Option<i64> {
    match t {
        LyricsTiming::NotChecked | LyricsTiming::DoesNotFit | LyricsTiming::Unsure => None,
        LyricsTiming::Automatic { ms } => Some(ms),
        LyricsTiming::Yours { automatic, .. } => automatic,
    }
}

/// Where the listener's timing for `lyrics` of song `song_id` from `origin` is kept: another source, or
/// the same source's words timed anew, is another key.
pub fn key(song_id: &str, origin: LyricsOrigin, lyrics: &Lyrics) -> String {
    // FNV-1a over the source and every line's time and words.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(format!("{origin:?}").as_bytes());
    eat(&[lyrics.synced as u8, lyrics.word_timed as u8]);
    for l in &lyrics.lines {
        eat(&l.start_ms.to_le_bytes());
        eat(l.text.as_bytes());
    }
    format!("lyrics_timing:{song_id}:{h:016x}")
}

/// The offset the clock adds to the playhead for `timing`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn lyrics_offset_ms(timing: LyricsTiming) -> i64 {
    timing.offset_ms()
}

/// What a one-line view of lyrics shows at one moment: indices into the lyrics' lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct WidgetLines {
    /// The line being sung; none between lines and in instrumental stretches (a note).
    pub line: Option<u32>,
    /// The next line with words under it.
    pub next: Option<u32>,
    /// Until the line changes at the given speed, if it will.
    pub wait_ms: Option<u64>,
}

/// Fewest ms a view waits for a line change.
const WIDGET_WAIT_MIN_MS: i64 = 50;
/// Slowest speed the wait is scaled by.
const WIDGET_SPEED_MIN: f32 = 0.1;

/// The lines a one-line view shows for `lyrics` with the playhead at `position_ms` while playing at
/// `speed`; none when no line has words. Unsynced lyrics show their first two lines with words.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn widget_lines(lyrics: Lyrics, position_ms: i64, speed: f32) -> Option<WidgetLines> {
    let said = |from: usize| lyrics.lines.iter().enumerate().skip(from).find(|(_, l)| !l.text.trim().is_empty()).map(|(i, _)| i as u32);
    let first = said(0)?;
    if !lyrics.synced {
        return Some(WidgetLines { line: Some(first), next: said(first as usize + 1), wait_ms: None });
    }
    let t = position_ms + lyrics.timing.offset_ms();
    let at = crate::look::clock_on(&lyrics, 0).timing().sung_line(t);
    let ahead = at.map_or(0, |i| i + 1);
    let wait_ms = lyrics.lines.get(ahead).map(|l| (((l.start_ms - t) as f32 / speed.max(WIDGET_SPEED_MIN)) as i64).max(WIDGET_WAIT_MIN_MS) as u64);
    Some(WidgetLines {
        line: at.filter(|&i| !lyrics.lines[i].text.trim().is_empty()).map(|i| i as u32),
        next: said(ahead),
        wait_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_model::LyricLine;

    #[test]
    fn yours_replaces_the_check_and_reset_gives_it_back() {
        let checked = LyricsTiming::Automatic { ms: 320 };
        let sooner = nudged(checked, 1);
        assert_eq!(sooner, LyricsTiming::Yours { ms: 350, automatic: Some(320) }, "from the check's, rounded to a step");
        assert_eq!(nudged(nudged(sooner, -1), -1), LyricsTiming::Yours { ms: 250, automatic: Some(320) });
        assert_eq!(nudged(sooner, 0), checked);
        assert_eq!(nudged(nudged(LyricsTiming::NotChecked, -1), 0), LyricsTiming::NotChecked);
        assert_eq!(with_yours(checked, Some(-250)), LyricsTiming::Yours { ms: -250, automatic: Some(320) });
        assert_eq!(with_yours(checked, None), checked);
    }

    #[test]
    fn other_words_are_another_key() {
        let line = |start_ms, text: &str| LyricLine { start_ms, text: text.into(), ..Default::default() };
        let l = Lyrics { synced: true, lines: vec![line(1000, "one"), line(4000, "two")], ..Default::default() };
        let k = key("s1", LyricsOrigin::Lrclib, &l);
        assert_eq!(k, key("s1", LyricsOrigin::Lrclib, &Lyrics { timing: LyricsTiming::Automatic { ms: 500 }, ..l.clone() }), "the timing is not the words");
        assert_ne!(k, key("s1", LyricsOrigin::Netease, &l));
        assert_ne!(k, key("s2", LyricsOrigin::Lrclib, &l));
        assert_ne!(k, key("s1", LyricsOrigin::Lrclib, &Lyrics { lines: vec![line(1000, "one"), line(4100, "two")], ..l.clone() }));
    }

    fn widget(timing: LyricsTiming, synced: bool, at: i64, speed: f32, lines: &[(i64, &str)]) -> Option<WidgetLines> {
        let lines = lines.iter().map(|&(start_ms, text)| LyricLine { start_ms, text: text.into(), ..Default::default() }).collect();
        widget_lines(Lyrics { synced, lines, timing, ..Default::default() }, at, speed)
    }

    #[test]
    fn widget_shows_the_sung_line_and_the_next_with_words() {
        let song = [(1000, "one"), (3000, " "), (5000, "three"), (8000, "four")];
        let shows = |line, next, wait_ms| Some(WidgetLines { line, next, wait_ms });
        let on_time = LyricsTiming::NotChecked;
        // Before the first line: a note, the first line under it.
        assert_eq!(widget(on_time, true, 400, 1.0, &song), shows(None, Some(0), Some(600)));
        assert_eq!(widget(on_time, true, 1000, 1.0, &song), shows(Some(0), Some(2), Some(2000)));
        // A blank line is a note, and the next line skips it.
        assert_eq!(widget(on_time, true, 4000, 1.0, &song), shows(None, Some(2), Some(1000)));
        // The last line has nothing after it and no change to wait for.
        assert_eq!(widget(on_time, true, 9000, 1.0, &song), shows(Some(3), None, None));
        // The wait is scaled by the speed and is never under 50 ms.
        assert_eq!(widget(on_time, true, 1000, 2.0, &song), shows(Some(0), Some(2), Some(1000)));
        assert_eq!(widget(on_time, true, 1000, 0.5, &song), shows(Some(0), Some(2), Some(4000)));
        assert_eq!(widget(on_time, true, 2990, 1.0, &song), shows(Some(0), Some(2), Some(50)));
        assert_eq!(widget(on_time, true, 1000, 0.0, &song), shows(Some(0), Some(2), Some(20_000)), "a speed of 0 counts as 0.1");
        // The lyrics' offset is added to the playhead.
        assert_eq!(widget(LyricsTiming::Automatic { ms: 2000 }, true, 1000, 1.0, &song), shows(None, Some(2), Some(2000)));
        // Starts out of order: the last one reached is sung.
        assert_eq!(widget(on_time, true, 1500, 1.0, &[(1000, "a"), (500, "b"), (2000, "c")]), shows(Some(1), Some(2), Some(500)));
    }

    #[test]
    fn widget_shows_unsynced_lyrics_from_the_top() {
        let words = [(-1, " "), (-1, "one"), (-1, "two"), (-1, "three")];
        assert_eq!(widget(LyricsTiming::NotChecked, false, 90_000, 1.0, &words), Some(WidgetLines { line: Some(1), next: Some(2), wait_ms: None }));
        assert_eq!(widget(LyricsTiming::NotChecked, false, 0, 1.0, &words[..2]), Some(WidgetLines { line: Some(1), next: None, wait_ms: None }));
    }

    #[test]
    fn widget_has_nothing_to_show_without_words() {
        assert_eq!(widget(LyricsTiming::NotChecked, true, 0, 1.0, &[(0, " "), (1000, "")]), None);
        assert_eq!(widget(LyricsTiming::NotChecked, false, 0, 1.0, &[]), None);
    }
}
