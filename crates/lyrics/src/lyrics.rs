//! The server's lyrics (OpenSubsonic structured lyrics with word cues) and LRC (with enhanced `<mm:ss.xx>`
//! word tags) in the app's shape. Plain line timing gets estimated word times so the player can sweep.
//! Text offsets are UTF-16, as Kotlin strings index.

use nori_model::model::{LyricLine, LyricWord, Lyrics};
use serde::Deserialize;

/// The latest time a lyric is taken at; later or garbled times are not times.
pub(crate) const LONGEST_MS: i64 = 24 * 3_600_000;

/// `ms` as a lyric time, if it is one.
pub(crate) fn time_ms(ms: f64) -> Option<i64> {
    (ms.is_finite() && (0.0..=LONGEST_MS as f64).contains(&ms)).then(|| ms.round() as i64)
}

/// A file's `[offset:]`; out of range is none.
pub(crate) fn offset_ms(v: &str) -> i64 {
    v.trim().parse::<i64>().ok().filter(|o| (-LONGEST_MS..=LONGEST_MS).contains(o)).unwrap_or(0)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Line {
    start: Option<i64>,
    value: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Cue {
    start: i64,
    end: i64,
    byte_start: usize,
    byte_end: usize,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CueLine {
    index: usize,
    start: i64,
    end: i64,
    value: String,
    agent_id: Option<String>,
    cue: Vec<Cue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Agent {
    id: String,
    role: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Structured {
    synced: bool,
    kind: Option<String>,
    offset: i64,
    line: Vec<Line>,
    cue_line: Vec<CueLine>,
    agents: Vec<Agent>,
}

pub(crate) fn utf16_at(text: &str, byte: usize) -> u32 {
    let mut b = byte.min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].encode_utf16().count() as u32
}

/// Splits enhanced-LRC word tags out of a line: the clean text and each tag's (ms, byte offset).
fn inline_words(raw: &str) -> (String, Vec<(i64, usize)>) {
    let mut text = String::with_capacity(raw.len());
    let mut marks = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else { break };
        match stamp(&rest[open + 1..open + close]) {
            Some(ms) => {
                text.push_str(&rest[..open]);
                marks.push((ms, text.len()));
            }
            None => text.push_str(&rest[..open + close + 1]),
        }
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);
    (text, marks)
}

/// Word times estimated by length over the line, the last tenth left as breath.
fn estimate(text: &str, start: i64, end: i64) -> Vec<LyricWord> {
    let spans: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut at = None;
        for (i, c) in text.char_indices() {
            match (c.is_whitespace(), at) {
                (false, None) => at = Some(i),
                (true, Some(s)) => {
                    v.push((s, i));
                    at = None;
                }
                _ => {}
            }
        }
        if let Some(s) = at {
            v.push((s, text.len()));
        }
        v
    };
    let weight: usize = spans.iter().map(|(s, e)| text[*s..*e].chars().count() + 1).sum();
    if weight == 0 || end <= start {
        return Vec::new();
    }
    let usable = ((end - start) as f64 * 0.9).min(12_000.0);
    let mut t = start as f64;
    spans
        .iter()
        .map(|(s, e)| {
            let d = usable * (text[*s..*e].chars().count() + 1) as f64 / weight as f64;
            let w = LyricWord { start_ms: t.round() as i64, end_ms: (t + d).round() as i64, start: utf16_at(text, *s), end: utf16_at(text, *e) };
            t += d;
            w
        })
        .collect()
}

/// `mm:ss.xx`, `mm:ss` or `mm:ss:xx` in ms.
fn stamp(tag: &str) -> Option<i64> {
    let (m, rest) = tag.split_once(':')?;
    let rest = rest.replacen(':', ".", 1);
    let secs: f64 = rest.parse().ok()?;
    time_ms(m.trim().parse::<u32>().ok()? as f64 * 60_000.0 + secs * 1000.0)
}

/// LRC, or plain text when it has no timestamps. A line with several timestamps repeats; `[offset:]` is
/// applied; other tags are skipped.
pub(crate) fn from_lrc(text: &str) -> Lyrics {
    let mut lines: Vec<Line> = Vec::new();
    let mut offset = 0i64;
    for raw in text.lines() {
        let mut rest = raw.trim_start_matches('\u{feff}').trim();
        let mut starts = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let Some(close) = body.find(']') else { break };
            let tag = &body[..close];
            if let Some(v) = tag.strip_prefix("offset:") {
                offset = offset_ms(v);
            } else if let Some(ms) = stamp(tag) {
                starts.push(ms);
            }
            rest = body[close + 1..].trim_start();
        }
        for s in starts {
            lines.push(Line { start: Some(s), value: rest.to_string() });
        }
    }
    if lines.is_empty() {
        let plain: Vec<Line> = text.lines().map(|l| Line { start: None, value: l.trim().to_string() }).collect();
        return build(vec![Structured { synced: false, line: plain, ..Default::default() }]);
    }
    lines.sort_by_key(|l| l.start);
    // A positive offset is sooner, in LRC as in OpenSubsonic.
    build(vec![Structured { synced: true, offset, line: lines, ..Default::default() }])
}

/// The server's lyrics layers as one: the main layer (synced over unsynced, never a translation or
/// pronunciation) with a translation matched in.
pub fn build(mut all: Vec<Structured>) -> Lyrics {
    all.sort_by_key(|l| (l.kind.as_deref() == Some("translation") || l.kind.as_deref() == Some("pronunciation"), !l.synced));
    let mut layers = all.into_iter();
    let Some(main) = layers.next().filter(|m| !m.line.is_empty()) else { return Lyrics::default() };
    let translation: Option<Structured> = layers.find(|l| l.kind.as_deref() == Some("translation") && l.synced == main.synced);
    let synced = main.synced;
    let offset = main.offset;
    let background: Vec<&str> = main.agents.iter().filter(|a| a.role == "bg").map(|a| a.id.as_str()).collect();
    let mut word_timed = false;

    // The server's times are taken as sent, within the day a lyric can run.
    let at = |ms: i64| ms.saturating_sub(offset).clamp(0, LONGEST_MS);
    let starts: Vec<i64> = main.line.iter().map(|l| at(l.start.unwrap_or(0))).collect();
    // Each line, and whether the source timed its words.
    let mut lines: Vec<(LyricLine, bool)> = Vec::with_capacity(main.line.len());
    for (i, l) in main.line.iter().enumerate() {
        let start = if synced { starts[i] } else { -1 };
        let next = starts[i + 1..].iter().copied().find(|n| *n > starts[i]).unwrap_or(starts[i] + 5_000);
        // A word whose end is not given, from `from`, is sung for as long as a word of `units` is, until a
        // line starting after it at the latest.
        let guessed = |from: i64, units: u32| {
            let later = starts[i + 1..].iter().copied().filter(|n| *n > from).min().unwrap_or(LONGEST_MS);
            (from + nori_look::lyrics::word_ms_estimate(units)).min(later)
        };
        let sung_until = |words: &[LyricWord], given: Option<i64>| words.iter().map(|w| w.end_ms).chain(given).max().filter(|e| *e > start).unwrap_or(next);
        let cues = main.cue_line.iter().find(|c| c.index == i && !c.cue.is_empty());
        let (text, end, words, bg, own) = if let Some(c) = cues {
            word_timed = true;
            let words: Vec<LyricWord> = c
                .cue
                .iter()
                .map(|w| {
                    let (start, end) = (utf16_at(&c.value, w.byte_start), utf16_at(&c.value, w.byte_end.saturating_add(1)));
                    let end_ms = if w.end > w.start { at(w.end) } else { guessed(at(w.start), end.saturating_sub(start)) };
                    LyricWord { start_ms: at(w.start), end_ms, start, end }
                })
                .collect();
            let end = sung_until(&words, (c.end > c.start).then(|| at(c.end)));
            (c.value.clone(), end, words, c.agent_id.as_deref().is_some_and(|a| background.contains(&a)), true)
        } else {
            let (mut text, marks) = inline_words(&l.value);
            if synced && !marks.is_empty() {
                word_timed = true;
                // A closing mark often follows a space.
                text.truncate(text.trim_end().len());
                let len = text.len();
                // A word runs to the next mark.
                let words: Vec<LyricWord> = marks.iter().enumerate().filter_map(|(k, (ms, byte))| {
                    let (byte, to) = ((*byte).min(len), marks.get(k + 1).map_or(len, |m| m.1.min(len)));
                    let (start, end) = (utf16_at(&text, byte), utf16_at(&text, to));
                    let end_ms = marks.get(k + 1).map_or_else(|| guessed(at(*ms), end - start), |m| at(m.0));
                    (to > byte).then_some(LyricWord { start_ms: at(*ms), end_ms, start, end })
                }).collect();
                let end = sung_until(&words, None);
                (text, end, words, false, true)
            } else {
                let words = if synced { estimate(&text, start, next) } else { Vec::new() };
                (text, next, words, false, false)
            }
        };
        let translated = translation.as_ref().and_then(|t| t.line.iter().find(|x| synced && x.start == l.start).or_else(|| (!synced).then(|| t.line.get(i)).flatten())).map(|x| x.value.clone()).filter(|v| !v.trim().is_empty());
        lines.push((LyricLine { start_ms: start, end_ms: if synced { end } else { -1 }, text, words, translation: translated, background: bg, ..Default::default() }, synced && own));
    }
    Lyrics { synced, word_timed, lines: sung_together(lines), ..Default::default() }
}

/// Lines overlapping by less than this follow each other: a closing time rounded past the next start.
const TOGETHER_MS: i64 = 300;

/// A line whose words the source times, starting while the line before is still being sung, is sung over
/// it: it becomes that line's backing vocals, since the player lights one line with its backing at a
/// time. A line that has backing already keeps it, and a third voice starts a line of its own.
fn sung_together(lines: Vec<(LyricLine, bool)>) -> Vec<LyricLine> {
    let mut out: Vec<(LyricLine, bool)> = Vec::with_capacity(lines.len());
    for (line, own) in lines {
        if let Some((prev, prev_own)) = out.last_mut() {
            let sung_until = prev.words.iter().map(|w| w.end_ms).max();
            if own && *prev_own && prev.backing.is_empty() && sung_until.is_some_and(|e| line.start_ms + TOGETHER_MS <= e) {
                prev.end_ms = prev.end_ms.max(line.end_ms);
                prev.backing = line.text;
                prev.backing_words = line.words;
                continue;
            }
        }
        out.push((line, own));
    }
    out.into_iter().map(|(l, _)| l).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Lyrics {
        build(serde_json::from_str::<Vec<Structured>>(json).unwrap())
    }

    #[test]
    fn server_word_cues_win() {
        let l = parse(r#"[{"synced":true,"line":[{"start":1000,"value":"Żółć and I"}],
          "cueLine":[{"index":0,"start":1000,"end":4000,"value":"Żółć and I","cue":[
            {"start":1000,"end":1800,"value":"Żółć ","byteStart":0,"byteEnd":7},{"start":1800,"end":2400,"value":"and ","byteStart":9,"byteEnd":11},{"start":2400,"end":3200,"value":"I","byteStart":13,"byteEnd":13}]}]}]"#);
        assert!(l.word_timed);
        let w = &l.lines[0].words;
        assert_eq!((w[0].start, w[0].end), (0, 4), "four letters, eight bytes");
        assert_eq!((w[1].start, w[1].end, w[2].start, w[2].end), (5, 8, 9, 10));
        assert_eq!(l.lines[0].end_ms, 4000);
    }

    #[test]
    fn lrc_words() {
        let l = parse(r#"[{"synced":true,"line":[{"start":0,"value":"one three"},{"start":4000,"value":"next"}]}]"#);
        assert!(!l.word_timed);
        let w = &l.lines[0].words;
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].start_ms, w[0].start, w[0].end), (0, 0, 3));
        assert!(w[1].end_ms <= 3600 && w[1].end_ms > w[1].start_ms && w[0].end_ms == w[1].start_ms);
        assert!(w[1].end_ms - w[1].start_ms > w[0].end_ms - w[0].start_ms, "the longer word takes longer");

        // Lrc last word before pause.
        // The last word has no closing mark and the next line is thirty seconds away.
        let l = from_lrc("[00:10.00]<00:10.00>Hold <00:10.40>on <00:10.80>tonight\n[00:40.00]<00:40.00>Again\n");
        let last = l.lines[0].words.last().unwrap().clone();
        assert_eq!(last.start_ms, 10_800);
        assert!(last.end_ms > 10_800 && last.end_ms <= 10_800 + 2_000, "sung for about as long as the word: {last:?}");
        assert_eq!(l.lines[0].words[1].end_ms, 10_800, "a word before it runs to the next mark");
        // A closing mark says when it ends; a next line sooner than the guess ends it there.
        let closed = from_lrc("[00:10.00]<00:10.00>Hold <00:10.40>on <00:10.80>tonight <00:14.00>\n[00:40.00]Again\n");
        assert_eq!(closed.lines[0].words.last().unwrap().end_ms, 14_000);
        let quick = from_lrc("[00:10.00]<00:10.00>Hold <00:10.40>on <00:10.80>tonight\n[00:11.00]Again\n");
        assert_eq!(quick.lines[0].words.last().unwrap().end_ms, 11_000);
    }

    #[test]
    fn lrc_tags() {
        let l = parse(r#"[{"kind":"translation","synced":true,"line":[{"start":1500,"value":"cześć"}]},
          {"synced":true,"offset":500,"line":[{"start":1500,"value":"<00:01.50>hel<00:01.90>lo <b>x"}]}]"#);
        assert_eq!(l.lines[0].text, "hello <b>x");
        assert_eq!(l.lines[0].start_ms, 1000);
        assert_eq!((l.lines[0].words[0].start_ms, l.lines[0].words[1].start_ms, l.lines[0].words[1].start), (1000, 1400, 3));
        assert_eq!(l.lines[0].translation.as_deref(), Some("cześć"));
        let plain = parse(r#"[{"synced":false,"line":[{"value":"just text"}]}]"#);
        assert_eq!((plain.synced, plain.lines[0].start_ms, plain.lines[0].words.len()), (false, -1, 0));
        assert!(parse("[]").lines.is_empty());

        // Lrc repeats offset and plain.
        let l = from_lrc("[ar:Someone]\n[offset:+200]\n[00:01.00][00:10.50]chorus\n[00:05.25]verse\n\n");
        assert!(l.synced);
        let got: Vec<(i64, &str)> = l.lines.iter().map(|x| (x.start_ms, x.text.as_str())).collect();
        assert_eq!(got, [(800, "chorus"), (5050, "verse"), (10300, "chorus")]);
        assert!(!l.lines[0].words.is_empty(), "words are estimated for plain LRC too");
        let plain = from_lrc("just\nwords");
        assert!(!plain.synced);
        assert_eq!(plain.lines.len(), 2);

        // Garbled times are not times.
        for lrc in ["[01:inf]x", "[00:inf]x", "[99999999999999:00.00]x", "[offset:-9223372036854775808]\n[00:00.00]x", "[-5:00.00]x"] {
            let l = from_lrc(lrc);
            assert!(!l.synced || l.lines.iter().all(|x| (0..=LONGEST_MS).contains(&x.start_ms) && x.end_ms >= x.start_ms), "{lrc}");
        }
        for json in [r#"{"lyrics":[{"time":1e300,"duration":5,"text":"x"}]}"#, r#"{"lyrics":[{"time":5,"duration":1e300,"text":"x"}]}"#] {
            let l = crate::json::from_lyricsplus(json);
            assert!(l.lines.iter().all(|x| x.start_ms <= LONGEST_MS), "{json}");
        }
        for file in [
            "version: '1.0'\nlines:\n  - {text: x, start_ms: 9223372036854775807}\n",
            "version: '1.0'\nlines:\n  - {text: x, start_ms: 1000, words: [{text: x, start_ms: 9223372036854775807}]}\n",
        ] {
            let l = crate::formats::from_lyricsfile(file);
            assert!(l.lines.iter().all(|x| (0..=LONGEST_MS).contains(&x.start_ms) && x.words.iter().all(|w| (0..=LONGEST_MS).contains(&w.start_ms))), "{file}");
        }
        let l = parse(r#"[{"synced":true,"line":[{"start":0,"value":"x"}],"cueLine":[{"index":0,"value":"x","cue":[{"start":0,"end":5,"byteStart":0,"byteEnd":18446744073709551615}]}]}]"#);
        assert!(l.lines[0].words[0].end >= l.lines[0].words[0].start, "a byte end past the text ends at the text");
        let early = from_lrc("[offset:1]\n[00:00.00]Title\n[00:05.00]x");
        assert_eq!(early.lines[0].start_ms, 0, "an offset before the start is the start, not untimed");
        let shared = from_lrc("[00:01.00]line\n[00:01.00]translated\n[00:02.00]next");
        assert_eq!(shared.lines[0].end_ms, 2_000, "a line sharing its time runs to the next later one");
    }

    /// A line as (start, text, backing, end).
    type Shape<'a> = (i64, &'a str, &'a str, i64);

    fn shapes(l: &Lyrics) -> Vec<Shape<'_>> {
        l.lines.iter().map(|x| (x.start_ms, x.text.as_str(), x.backing.as_str(), x.end_ms)).collect()
    }

    #[test]
    fn enhanced_lrc_lines_sung_together_are_one_lit_line() {
        let cases: &[(&str, &str, &[Shape])] = &[
            (
                "the next line starts before the last word",
                "[00:12.00]<00:12.00>I <00:12.40>was <00:12.80>walking <00:14.90>home\n[00:14.20]<00:14.20>(walking <00:14.60>home) <00:15.80>\n[00:20.00]<00:20.00>Next\n",
                &[(12_000, "I was walking home", "(walking home)", 15_800), (20_000, "Next", "", 20_690)],
            ),
            (
                "a closing time past the next line's start",
                "[00:12.00]<00:12.00>I <00:12.40>was <00:12.80>walking <00:14.00>\n[00:13.50]<00:13.50>(walking) <00:14.20>\n[00:20.00]<00:20.00>Next <00:21.00>\n",
                &[(12_000, "I was walking", "(walking)", 14_200), (20_000, "Next", "", 21_000)],
            ),
            (
                "two lines at one time",
                "[00:12.00]<00:12.00>Hey <00:12.50>you <00:13.00>\n[00:12.00]<00:12.00>Oh <00:12.80>yeah <00:13.40>\n[00:20.00]<00:20.00>Next <00:21.00>\n",
                &[(12_000, "Hey you", "Oh yeah", 13_400), (20_000, "Next", "", 21_000)],
            ),
            (
                "a voice prefix, kept as written",
                "[00:12.00]v1:<00:12.00>Hey <00:12.50>you <00:14.00>\n[00:13.00]v2:<00:13.00>Oh <00:13.80>yeah <00:14.40>\n[00:20.00]v1:<00:20.00>Next <00:21.00>\n",
                &[(12_000, "v1:Hey you", "v2:Oh yeah", 14_400), (20_000, "v1:Next", "", 21_000)],
            ),
            (
                "a closing time rounded past the next start is one line after another",
                "[00:12.00]<00:12.00>Hey <00:12.50>you <00:13.01>\n[00:13.00]<00:13.00>Oh <00:13.80>yeah <00:14.40>\n",
                &[(12_000, "Hey you", "", 13_010), (13_000, "Oh yeah", "", 14_400)],
            ),
            (
                "a third voice over a pair starts its own line",
                "[00:12.00]<00:12.00>One <00:14.00>\n[00:13.00]<00:13.00>Two <00:15.00>\n[00:14.50]<00:14.50>Three <00:16.00>\n",
                &[(12_000, "One", "Two", 15_000), (14_500, "Three", "", 16_000)],
            ),
            (
                "plain lines run to the next",
                "[00:12.00]Hey you\n[00:12.00]Oh yeah\n[00:14.00]Next\n",
                &[(12_000, "Hey you", "", 14_000), (12_000, "Oh yeah", "", 14_000), (14_000, "Next", "", 19_000)],
            ),
        ];
        for (name, lrc, want) in cases {
            assert_eq!(shapes(&from_lrc(lrc)), *want, "{name}");
        }

        // Both are lit while both are sung: the first line's last word, then the second's.
        let l = from_lrc(cases[0].1);
        let w = &l.lines[0].backing_words;
        assert_eq!((w[0].start_ms, w[0].end_ms, w[1].start_ms, w[1].end_ms), (14_200, 14_600, 14_600, 15_800));
        let clock = crate::look::clock_on(&l, 0);
        for at in [14_500, 15_000, 15_500] {
            assert_eq!(clock.timing().frame(at).active, 0, "at {at}");
        }
        assert!(clock.timing().frame(15_000).sung > "I was walking ".len() as f32, "the last word is sung after the second line starts");
        assert!(clock.timing().backing_sung(0, 15_000) > 0.0);

        // The same file as Navidrome's word cues, the last word's end not given (as long as its start).
        let served = parse(r#"[{"synced":true,"line":[{"start":2000,"value":"I was walking home"},{"start":4800,"value":"(walking home)"},{"start":8000,"value":"Next"}],
          "cueLine":[{"index":0,"start":2000,"end":5600,"value":"I was walking home","cue":[{"start":2000,"end":2600,"byteStart":0,"byteEnd":1},
            {"start":2600,"end":3200,"byteStart":2,"byteEnd":5},{"start":3200,"end":5600,"byteStart":6,"byteEnd":13},{"start":5600,"end":5600,"byteStart":14,"byteEnd":17}]},
          {"index":1,"start":4800,"end":7200,"value":"(walking home)","cue":[{"start":4800,"end":5400,"byteStart":0,"byteEnd":8},{"start":5400,"end":7200,"byteStart":9,"byteEnd":13}]}]}]"#);
        assert_eq!(shapes(&served), [(2_000, "I was walking home", "(walking home)", 7_200), (8_000, "Next", "", 13_000)]);
        assert_eq!(served.lines[0].words[3].end_ms, 5_600 + nori_look::lyrics::word_ms_estimate(4), "an end not given is guessed");
    }

}
