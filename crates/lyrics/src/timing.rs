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
        LyricsTiming::NotChecked | LyricsTiming::DoesNotFit => None,
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
}
