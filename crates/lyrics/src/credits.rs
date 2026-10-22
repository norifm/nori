//! What is not sung at either end of an answer: credits ("Lyrics by …", "作词 : …"), watermarks and ads,
//! a header naming the song, an "[Instrumental]" placeholder, empty lines. Only a run at an end goes,
//! so a sung line holding "by" or a colon stays.

use nori_model::Lyrics;

use crate::fit::norm;

/// How far into an answer its opening credits may run...
const HEAD_MOST: usize = 16;
/// ...and how far back from its end its closing ones.
const TAIL_MOST: usize = 10;

/// Words on a label that says a line is a credit: "Lyrics by:", "Composer:", "OP:".
const CREDIT_WORDS: &[&str] = &[
    "lyric", "lyrics", "lyricist", "lyricists", "written", "writer", "writers", "words", "music", "composer", "composers", "composed",
    "composition", "producer", "producers", "produced", "production", "arranger", "arranged", "arrangement", "mixed", "mixing", "mix",
    "mastered", "mastering", "recorded", "recording", "engineer", "engineered", "vocals", "vocal", "publisher", "published", "op", "sp",
    "isrc", "transcribed", "transcription", "transcriber", "synced", "sync", "timed", "timing", "lrc", "edited", "editor", "translated",
    "translation", "translator", "songwriter", "songwriters", "source", "label", "copyright", "subtitles", "subs",
];

/// Words on a label that names the song itself: "Artist:", "Songs Title:", "Album:".
const DETAIL_WORDS: &[&str] = &["artist", "artists", "singer", "singers", "song", "songs", "title", "songtitle", "album"];

/// The words that may come before " by " in a credit written without a colon ("Words and music by …").
const BY_WORDS: &[&str] = &["and", "originally", "provided", "powered", "licensed", "brought", "made", "all", "additional", "the"];

/// Openings of a service's watermark or an ad, lower case, in the languages the services answer in.
const WATERMARKS: &[&str] = &[
    "lyrics from",
    "lyrics provided",
    "lyrics powered",
    "lyrics licensed",
    "lyrics courtesy",
    "paroles de la chanson",
    "paroles de ",
    "paroles: ",
    "letra de ",
    "letras de ",
    "songtext von",
    "songtext zu",
    "testo di ",
    "текст песни",
    "you might also like",
    "get tickets",
    "writer(s)",
    "source:",
    "www.",
    "http://",
    "https://",
    "this lyrics is not for commercial use",
    "lyrics are not for commercial use",
    "no lyrics",
    "lyrics not available",
    "we don't have the lyrics",
];

/// Placeholders standing in for words where a song has none, after brackets and stars are gone.
const INSTRUMENTAL: &[&str] = &["instrumental", "instrumental break", "instrumental outro", "instrumental intro", "music", "inst", "インスト", "연주곡", "間奏", "间奏"];

/// Duet speaker tags ("男：…", "여: …"): labels of sung lines, not credits.
const SPEAKERS: &[&str] = &["男", "女", "合", "合唱", "男女", "全", "齐", "齊", "남", "여", "남자", "여자", "모두", "全員"];

/// "Role: name": a short label naming a credit in English or written in another script (作词, 작사).
fn role_and_name(t: &str) -> bool {
    let Some((label, value)) = t.split_once([':', '：']) else { return false };
    let label = label.trim();
    let lower = label.to_lowercase();
    // "Lyricsby:" is "Lyrics by:" with its space lost.
    let named = lower.split(|c: char| !c.is_alphabetic()).any(|w| CREDIT_WORDS.contains(&w.strip_suffix("by").unwrap_or(w)) || DETAIL_WORDS.contains(&w));
    let native = !label.is_empty() && !SPEAKERS.contains(&label) && label.chars().count() <= 8 && !label.chars().any(|c| c.is_ascii_alphanumeric()) && label.chars().any(char::is_alphabetic);
    !value.trim().is_empty() && label.chars().count() <= 24 && (named || native)
}

/// "Written by Someone": only credit words before " by ", and a name after it (a capital, a digit or
/// another script), so "stand by the water" is not one. A bare "By 某人" signs an upload when the name is
/// in a script other than Latin.
fn by_line(t: &str) -> bool {
    if let Some(name) = t.get(..3).filter(|by| by.eq_ignore_ascii_case("by ")).map(|_| t[3..].trim()) {
        return name.chars().next().is_some_and(|c| c.is_alphabetic() && c >= '\u{370}');
    }
    let Some(at) = t.to_ascii_lowercase().find(" by ") else { return false };
    let (head, tail) = (t[..at].to_lowercase(), t[at + 4..].trim());
    let words: Vec<&str> = head.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).collect();
    let credits = !words.is_empty() && words.iter().all(|w| CREDIT_WORDS.contains(w) || BY_WORDS.contains(w)) && words.iter().any(|w| CREDIT_WORDS.contains(w));
    let name = tail.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit() || (c.is_alphabetic() && !c.is_ascii()));
    credits && name && words.len() <= 5
}

/// A watermark, an ad or a web page's furniture: "Lyrics from …", "123Embed", "See Artist Live".
fn watermark(t: &str) -> bool {
    let lower = t.trim().to_lowercase();
    if WATERMARKS.iter().any(|w| lower.starts_with(w)) {
        return true;
    }
    let embed = lower.strip_suffix("embed").is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()));
    let live = lower.starts_with("see ") && (lower.ends_with(" live") || lower.contains("get tickets"));
    let contributors = lower.split_whitespace().next().is_some_and(|w| w.chars().all(|c| c.is_ascii_digit())) && lower.contains("contributor");
    // Musixmatch's tracking number, "(1409617462395)", alone on its line.
    let tracking = lower.strip_prefix('(').and_then(|r| r.strip_suffix(')')).is_some_and(|n| n.len() >= 8 && n.chars().all(|c| c.is_ascii_digit()));
    embed || live || contributors || tracking || lower.ends_with(".com") || lower.contains("***")
}

/// "[Instrumental]", "(Instrumental)", "纯音乐，请欣赏": a placeholder where there are no words.
fn instrumental(t: &str) -> bool {
    let bare = t.trim().trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | '*' | '♪' | '-' | '~' | '【' | '】' | '（' | '）' | ' '));
    let lower = bare.to_lowercase();
    INSTRUMENTAL.contains(&lower.as_str()) || ["纯音乐", "純音樂", "没有填词", "沒有填詞"].iter().any(|w| bare.contains(w))
}

/// Nothing sung: empty (a time-only line), or only punctuation and music signs.
fn blank(t: &str) -> bool {
    !t.chars().any(char::is_alphanumeric)
}

/// A header naming the song: the title, "Title Lyrics", "Artist - Title" or "Title by Artist".
fn header(t: &str, title: &str, artist: &str) -> bool {
    // The release a header names may differ from the library's: "Title (2018 Mix)" for "Title (Remastered)".
    let title = crate::lrclib::clean(title);
    let title = title.find(['(', '[']).filter(|at| *at > 0).map_or(title.as_str(), |at| &title[..at]);
    let (line, title, artist) = (norm(t), norm(title), norm(artist));
    if title.is_empty() || line.is_empty() {
        return false;
    }
    let bare = line.strip_suffix(" lyrics").unwrap_or(&line);
    let rest = bare.replacen(&title, "", 1);
    let artist_named = !artist.is_empty() && format!(" {rest} ").contains(&format!(" {artist} "));
    let rest = if artist.is_empty() { rest } else { rest.replacen(&artist, "", 1) };
    let dashed = (t.contains(" - ") || t.contains(" – ")) && line.split_whitespace().count() <= title.split_whitespace().count() + 6;
    bare.contains(&title) && (dashed || artist_named || rest.split_whitespace().all(|w| w == "by"))
}

/// Whether `text` is a credit; with `title` and `artist`, a header naming the song counts too.
pub(crate) fn is_credit(text: &str, title: &str, artist: &str) -> bool {
    let t = text.trim();
    role_and_name(t) || by_line(t) || watermark(t) || instrumental(t) || header(t, title, artist)
}

/// Drops credits and empty lines from the first [`HEAD_MOST`] and last [`TAIL_MOST`] of `lines`.
pub(crate) fn strip_lines<T>(lines: &mut Vec<T>, text: impl Fn(&T) -> &str, title: &str, artist: &str) {
    let head = lines.iter().take(HEAD_MOST).take_while(|l| blank(text(l)) || is_credit(text(l), title, artist)).count();
    lines.drain(..head);
    let tail = lines.iter().rev().take(TAIL_MOST).take_while(|l| blank(text(l)) || is_credit(text(l), "", "")).count();
    lines.truncate(lines.len() - tail);
}

/// `l` without the credits at either end; empty when nothing is left.
pub(crate) fn strip_edges(l: &mut Lyrics, title: &str, artist: &str) {
    strip_lines(&mut l.lines, |x| x.text.as_str(), title, artist);
    if l.lines.is_empty() {
        *l = Lyrics::default();
    }
}

/// How many lines still look like credits or placeholders (the middle, which [`strip_edges`] keeps).
pub(crate) fn credits_inside(l: &Lyrics) -> usize {
    l.lines.iter().filter(|x| !blank(&x.text) && (role_and_name(&x.text) || watermark(&x.text) || instrumental(&x.text))).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_model::LyricLine;

    /// Lines at `start` + 4 s each; `-1` start for plain words.
    fn lyrics(lines: &[&str], synced: bool) -> Lyrics {
        let lines = lines
            .iter()
            .enumerate()
            .map(|(i, t)| LyricLine { start_ms: if synced { 4_000 * i as i64 } else { -1 }, end_ms: if synced { 4_000 * i as i64 + 3_000 } else { -1 }, text: t.to_string(), ..Default::default() })
            .collect();
        Lyrics { synced, lines, ..Default::default() }
    }

    fn texts(l: &Lyrics) -> Vec<&str> {
        l.lines.iter().map(|x| x.text.as_str()).collect()
    }

    const SUNG: [&str; 4] = ["first line of the song", "line two goes here", "la la la", "the last line of the song"];

    #[test]
    fn headers_and_credits_go() {
        let mut l = lyrics(&["作词 : 某人", "作曲 : 某人", "编曲：另一人", "制作人 : 某人", SUNG[0], SUNG[1], SUNG[2], SUNG[3], "混音 : 某人", "母带 : 某人"], true);
        let times: Vec<i64> = l.lines[4..8].iter().map(|x| x.start_ms).collect();
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), SUNG);
        assert_eq!(l.lines.iter().map(|x| x.start_ms).collect::<Vec<_>>(), times, "the lines left keep their times");
        let mut k = lyrics(&["The Lanterns - Glass Harbour", "作詞：誰か", "작곡: 누군가", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], true);
        strip_edges(&mut k, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&k), SUNG, "KuGou's opening names the song; Japanese and Korean labels are credits too");

        // Genius headers and furniture go.
        let mut l = lyrics(&["12 ContributorsGlass Harbour Lyrics", "Glass Harbour Lyrics", "", SUNG[0], SUNG[1], SUNG[2], SUNG[3], "You might also like", "3Embed"], false);
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), SUNG);
        let mut see = lyrics(&[SUNG[0], SUNG[1], "See The Lanterns LiveGet tickets as low as $40", "Embed"], false);
        strip_edges(&mut see, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&see), &SUNG[..2]);

        // Service headers and watermarks go.
        let mut l = lyrics(
            &["", "Lyrics by: Someone Person", "Written by Someone Person & Another", "Transcribed by A. Listener", SUNG[0], SUNG[1], SUNG[2], SUNG[3], "", "******* This Lyrics is NOT for Commercial use *******", "(1409617462395)"],
            true,
        );
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), SUNG);
        let mut fr = lyrics(&["Paroles de la chanson Glass Harbour par The Lanterns", SUNG[0], SUNG[1], SUNG[2], "Lyrics from somewhere.example", "Lyrics provided by SomeService", "Produced by Some Producer"], false);
        strip_edges(&mut fr, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&fr), &SUNG[..3]);
    }

    #[test]
    fn speaker_tags_are_sung() {
        for tags in [["男：你好", "女：再见", "合：我们"], ["남: 안녕", "여: 잘 가", "모두: 우리"], ["男女：你好", "合唱：再见", "全員：我们"]] {
            let mut l = lyrics(&[tags[0], tags[1], tags[2], "作词：某人", SUNG[0]], true);
            strip_edges(&mut l, "Glass Harbour", "The Lanterns");
            assert_eq!(texts(&l), [tags[0], tags[1], tags[2], "作词：某人", SUNG[0]], "{tags:?}");
        }
        let mut l = lyrics(&["作词：某人", "男：你好", SUNG[0]], true);
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), ["男：你好", SUNG[0]], "a credit still goes");
    }

    #[test]
    fn song_details_go() {
        let cases: [(&[&str], &str); 6] = [
            // KuGou and NetEase open with the song's details as labels, and sign off with the uploader.
            (&["Artist：the lanterns", "Songs Title：glass harbour", SUNG[0], SUNG[1], SUNG[2], SUNG[3], "By 某人"], "Glass Harbour"),
            (&["Artist: The Lanterns", "Song : Glass Harbour （Someone）", "Album : Low Tide （c）1975", "Album:Low Tide", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], "Glass Harbour"),
            (&["Lyricsby：SOMEONE/ANOTHER", "Composedby：SOMEONE", "Originally recorded by Another Band", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], "Glass Harbour"),
            // A header naming another release of the song than the library's.
            (&["Glass Harbour (2018 Mix) - The Lanterns (灯笼)", "Lyrics by：Someone Person", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], "Glass Harbour (2011 Remastered Version)"),
            (&["Glass Harbour (Live At Some Studio Somewhere, Far City) - The Lanterns (灯笼)", "Written by：Someone", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], "Glass Harbour"),
            // NetEase's word-timed header, its words run together.
            (&["GLASS HARBOUR（グラス（ハーバー-The Lanterns（灯笼", "Lyricsby：SOMEONE", SUNG[0], SUNG[1], SUNG[2], SUNG[3]], "Glass Harbour"),
        ];
        for (lines, title) in cases {
            let mut l = lyrics(lines, true);
            strip_edges(&mut l, title, "The Lanterns");
            assert_eq!(texts(&l), SUNG, "{lines:?}");
        }
        let sung = ["By the water we stood", SUNG[1], SUNG[2], "By Your Side"];
        let mut l = lyrics(&sung, true);
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), sung, "a sung line opening with 'by' stays");
    }

    #[test]
    fn lyrics_stay() {
        for only in [["[Instrumental]"], ["(Instrumental)"], ["纯音乐，请欣赏"], ["此歌曲为没有填词的纯音乐，请您欣赏"]] {
            let mut l = lyrics(&only, true);
            strip_edges(&mut l, "Glass Harbour", "The Lanterns");
            assert_eq!(l, Lyrics::default(), "{only:?}");
        }

        // Sung lines and middle stay.
        let edges = ["Stand by the water", "Music by the river tonight", "She said: stay a while", "Written in the stars above", "İzmir by Çağrı"];
        let mut l = lyrics(&edges, true);
        strip_edges(&mut l, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&l), edges, "sung lines holding 'by', a colon or a credit word are not credits");
        let middle = [SUNG[0], "Lyrics by: Someone", "[Instrumental]", SUNG[1]];
        let mut m = lyrics(&middle, true);
        strip_edges(&mut m, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&m), middle, "only the ends are stripped");
        assert_eq!(credits_inside(&m), 2, "what is left in the middle counts against the answer");
        let mut titled = lyrics(&["Glass harbour", SUNG[0], SUNG[1], "Glass harbour, glass harbour"], true);
        strip_edges(&mut titled, "Glass Harbour", "The Lanterns");
        assert_eq!(texts(&titled), [SUNG[0], SUNG[1], "Glass harbour, glass harbour"], "a header naming the song goes; the title sung at the end stays");
    }

}
