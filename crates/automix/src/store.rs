//! The `track_analysis` and `vocal_curve` tables, and the streaming analyser that feeds them.

use nori_model::{BarProfile, Beat, TrackAnalysis};
use nori_player::automix::vocal::VocalCurve;
use rusqlite::{params, Connection, OptionalExtension};

use super::analysis::Analyzer;
use super::ANALYSIS_VERSION;

/// The `track_analysis` columns in [`TrackAnalysis`] field order; `or` gives what a column that does not read as
/// its type stands for.
macro_rules! columns {
    ($($field:ident $(or $default:expr)?),* $(,)?) => {
        const COLUMNS: &[&str] = &[$(stringify!($field)),*];

        fn values(a: &TrackAnalysis) -> [&dyn rusqlite::ToSql; COLUMNS.len()] {
            [$(&a.$field),*]
        }

        fn row(r: &rusqlite::Row) -> rusqlite::Result<TrackAnalysis> {
            let mut at = 0..;
            Ok(TrackAnalysis { $($field: column!(r, at.next().expect("unbounded") $(, $default)?),)* ..TrackAnalysis::default() })
        }
    };
}

macro_rules! column {
    ($r:expr, $i:expr) => {
        $r.get($i)?
    };
    ($r:expr, $i:expr, $default:expr) => {
        $r.get($i).unwrap_or($default)
    };
}

columns! {
    song_id, analysis_version, duration_ms, bpm, bpm_confidence, beat_offset_ms, stability, downbeat_phase,
    downbeat_confidence, lufs, key, key_confidence, silence_start_ms, silence_end_ms, mixramp_start_ms, mixramp_end_ms,
    intro_end_ms, outro_start_ms, outro_centroid or 0.0, intro_centroid or 0.0,
    analysed_ms, outro_bpm or 0.0, outro_bpm_confidence or 0.0, outro_beat_offset_ms or 0.0, outro_stability or 0.0,
    outro_downbeat_phase or 0, intro_bpm or 0.0, intro_bpm_confidence or 0.0, intro_beat_offset_ms or 0.0,
    intro_stability or 0.0, intro_downbeat_phase or 0, beats_per_bar or 0, drop_ms or 0,
    exit_ms or 0, gap_ms or 0, gap_end_ms or 0, drop_runup_tonal_db or 0.0,
    intro_beats_per_bar or 0, outro_beats_per_bar or 0, intro_grid_source or 0, outro_grid_source or 0, last_drop_ms or 0, vocal_start_ms or 0, vocal_end_ms or 0,
    steady_intro_ms or 0, steady_end_ms or 0, steady_exit_ms or 0,
    intro_grid_from_ms or 0, intro_grid_to_ms or 0, outro_grid_from_ms or 0, outro_grid_to_ms or 0,
}

/// The bar profile and beat columns, after [`COLUMNS`].
const BARS: [&str; 4] = ["intro_bars", "outro_bars", "intro_beats", "outro_beats"];

pub fn put(c: &Connection, a: &TrackAnalysis) -> rusqlite::Result<()> {
    let marks: Vec<String> = (1..=COLUMNS.len() + BARS.len()).map(|i| format!("?{i}")).collect();
    let bars = [encode_bars(&a.intro_bars), encode_bars(&a.outro_bars), encode_beats(&a.intro_beats), encode_beats(&a.outro_beats)];
    let mut all: Vec<&dyn rusqlite::ToSql> = values(a).to_vec();
    all.extend(bars.iter().map(|b| b as &dyn rusqlite::ToSql));
    c.prepare_cached(&format!("INSERT OR REPLACE INTO track_analysis(server, {}, {}) VALUES(sid(), {})", COLUMNS.join(", "), BARS.join(", "), marks.join(",")))?
        .execute(&all[..])
        .map(|_| ())
}

pub fn get(c: &Connection, song_id: &str) -> rusqlite::Result<Option<TrackAnalysis>> {
    c.prepare_cached(&format!("SELECT {}, {} FROM track_analysis WHERE server=sid() AND song_id=?1", COLUMNS.join(", "), BARS.join(", ")))?
        .query_row([song_id], |r| {
            let mut a = row(r)?;
            let blob = |i: usize| r.get::<_, Vec<u8>>(COLUMNS.len() + i).map(|b| decode_bars(&b)).unwrap_or_default();
            (a.intro_bars, a.outro_bars) = (blob(0), blob(1));
            let beats = |i: usize| r.get::<_, Vec<u8>>(COLUMNS.len() + i).map(|b| decode_beats(&b)).unwrap_or_default();
            (a.intro_beats, a.outro_beats) = (beats(2), beats(3));
            Ok(a)
        })
        .optional()
}

/// Bytes per stored bar: its start (ms, i32), level and lows (half dB, i8), pulse, voice and change (0..255).
const BAR_BYTES: usize = 9;

fn encode_bars(bars: &[BarProfile]) -> Vec<u8> {
    let half_db = |db: f32| (db * 2.0).round().clamp(-128.0, 127.0) as i8 as u8;
    let share = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    bars.iter()
        .flat_map(|b| {
            let s = (b.start_ms.clamp(i32::MIN as i64, i32::MAX as i64) as i32).to_le_bytes();
            [s[0], s[1], s[2], s[3], half_db(b.level_db), half_db(b.low_db), share(b.pulse), share(b.voice), share(b.change)]
        })
        .collect()
}

/// Bytes per stored beat: its time (ms, i32) and flags (bit 0 downbeat, bit 1 sure).
const BEAT_BYTES: usize = 5;

fn encode_beats(beats: &[Beat]) -> Vec<u8> {
    beats
        .iter()
        .flat_map(|b| {
            let t = (b.at_ms.clamp(i32::MIN as i64, i32::MAX as i64) as i32).to_le_bytes();
            [t[0], t[1], t[2], t[3], b.downbeat as u8 | (b.sure as u8) << 1]
        })
        .collect()
}

fn decode_beats(b: &[u8]) -> Vec<Beat> {
    b.as_chunks::<BEAT_BYTES>().0.iter().map(|x| Beat { at_ms: i32::from_le_bytes([x[0], x[1], x[2], x[3]]) as i64, downbeat: x[4] & 1 != 0, sure: x[4] & 2 != 0 }).collect()
}

fn decode_bars(b: &[u8]) -> Vec<BarProfile> {
    b.as_chunks::<BAR_BYTES>().0.iter()
        .map(|x| BarProfile {
            start_ms: i32::from_le_bytes([x[0], x[1], x[2], x[3]]) as i64,
            level_db: x[4] as i8 as f32 / 2.0,
            low_db: x[5] as i8 as f32 / 2.0,
            pulse: x[6] as f32 / 255.0,
            voice: x[7] as f32 / 255.0,
            change: x[8] as f32 / 255.0,
        })
        .collect()
}

/// Stores a fresh classical analysis, keeping the Beat This! grids and "model has looked" marks of the stored row
/// (`beats::carry`). Returns the row as stored.
pub fn put_measured(c: &Connection, mut a: TrackAnalysis) -> rusqlite::Result<TrackAnalysis> {
    if let Some(old) = get(c, &a.song_id)? {
        super::beats::carry(&old, &mut a);
    }
    put(c, &a)?;
    Ok(a)
}

/// A whole song's measurement turned into its analysis and vocal curve; costly, so done before taking the database.
pub fn measure(song_id: &str, features: &crate::analysis::Features) -> (TrackAnalysis, VocalCurve) {
    (crate::finish(song_id, features).track, features.voice_curve())
}

/// Stores a measured song: its analysis (keeping the beat model's grids) and vocal curve.
pub fn put_finished(c: &Connection, track: TrackAnalysis, curve: &VocalCurve) -> rusqlite::Result<TrackAnalysis> {
    let song_id = track.song_id.clone();
    let stored = put_measured(c, track)?;
    put_voice(c, &song_id, curve)?;
    Ok(stored)
}

/// Stores a song's vocal activity curve (`vocal.rs`), used to check synced lyrics against the audio.
pub fn put_voice(c: &Connection, song_id: &str, curve: &VocalCurve) -> rusqlite::Result<()> {
    c.prepare_cached("INSERT OR REPLACE INTO vocal_curve(server, song_id, curve) VALUES(sid(), ?1, ?2)")?.execute(params![song_id, curve.encode()]).map(|_| ())
}

/// A song's vocal activity curve; None when absent or stored in an old format.
pub fn get_voice(c: &Connection, song_id: &str) -> rusqlite::Result<Option<VocalCurve>> {
    let blob: Option<Vec<u8>> = c.prepare_cached("SELECT curve FROM vocal_curve WHERE server=sid() AND song_id=?1")?.query_row([song_id], |r| r.get(0)).optional()?;
    Ok(blob.and_then(|b| VocalCurve::decode(&b)))
}

/// Keeps where song `song_id`'s synced lyrics are sung, ms of the song (`nori_lyrics::sync::sung_span`); true when
/// that changed.
pub fn put_sung(c: &Connection, song_id: &str, (start, end): (i64, i64)) -> rusqlite::Result<bool> {
    let key = sung_key(song_id);
    let value = format!("{start} {end}");
    if nori_db::kv_get(c, &key)?.as_deref() == Some(value.as_str()) {
        return Ok(false);
    }
    nori_db::kv_put(c, &key, &value).map(|_| true)
}

/// Song `song_id`'s analysis, with where it is sung taken from its lyrics when they were heard to fit: their lines
/// say it better than the voice measured under the band.
pub fn get_with_lyrics(c: &Connection, song_id: &str) -> rusqlite::Result<Option<TrackAnalysis>> {
    let Some(mut a) = get(c, song_id)? else { return Ok(None) };
    let sung = nori_db::kv_get(c, &sung_key(song_id))?.and_then(|v| {
        let (start, end) = v.split_once(' ')?;
        Some((start.parse::<i64>().ok()?, end.parse::<i64>().ok()?))
    });
    if let Some((start, end)) = sung {
        let (start, end) = (start.max(0), end.min(a.duration_ms));
        if start < end {
            (a.vocal_start_ms, a.vocal_end_ms) = (start, end);
        }
    }
    Ok(Some(a))
}

fn sung_key(song_id: &str) -> String {
    format!("sung|{song_id}")
}

/// The `ids` with a current analysis the beat model has not yet looked at, in order. Songs without a current
/// analysis are left out.
pub fn neural_missing(c: &Connection, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut out = Vec::new();
    for id in ids {
        if get(c, id)?.is_some_and(|r| r.analysis_version >= ANALYSIS_VERSION && super::beats::needs_model(&r)) {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// Each stored row's analysis version and vocal curve.
const ROWS_AND_CURVES: &str = "SELECT t.analysis_version, v.curve FROM track_analysis t \
    LEFT JOIN vocal_curve v ON v.server=t.server AND v.song_id=t.song_id WHERE t.server=sid()";

/// A row this version plans from: measured by this analysis version, with a vocal curve it reads.
fn current(r: &rusqlite::Row) -> rusqlite::Result<bool> {
    let version: i32 = r.get(0)?;
    let curve: Option<Vec<u8>> = r.get(1)?;
    Ok(version >= ANALYSIS_VERSION && curve.as_deref().and_then(VocalCurve::decode).is_some())
}

/// The `ids` with no row, a row from an older analysis version, or no vocal curve this version reads,
/// in order.
pub fn missing(c: &Connection, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut st = c.prepare_cached(&format!("{ROWS_AND_CURVES} AND t.song_id=?1"))?;
    let mut out = Vec::new();
    for id in ids {
        if !st.query_row([id], current).optional()?.unwrap_or(false) {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// The songs measured, and how many of them are measured again when they next play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Measured {
    pub songs: u32,
    pub outdated: u32,
}

pub fn measured(c: &Connection) -> rusqlite::Result<Measured> {
    let mut st = c.prepare_cached(ROWS_AND_CURVES)?;
    let mut rows = st.query([])?;
    let mut m = Measured::default();
    while let Some(r) = rows.next()? {
        m.songs += 1;
        m.outdated += u32::from(!current(r)?);
    }
    Ok(m)
}

/// A song measured as it is decoded, fed buffer by buffer.
pub struct AnalysisStream {
    a: Analyzer,
    channels: usize,
}

impl AnalysisStream {
    /// `expected_ms` (0 if unknown) sizes the buffers so feeding never reallocates.
    pub fn new(rate: u32, channels: usize, expected_ms: u64) -> Self {
        AnalysisStream { a: Analyzer::new(rate.max(1), expected_ms), channels: channels.clamp(1, 8) }
    }

    /// Interleaved float samples.
    pub fn feed_f32(&mut self, x: &[f32]) {
        self.a.feed_interleaved(x, self.channels, |v| v);
    }

    pub fn into_analyzer(self) -> Analyzer {
        self.a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_profiles_come_back() {
        let c = nori_db::open("", "s").unwrap();
        let bar = |i: i64| BarProfile { start_ms: 1_000 + 2_000 * i, level_db: -3.5 - i as f32, low_db: -40.0, pulse: 0.25 * i as f32, voice: 1.0 - 0.25 * i as f32, change: 0.1 * i as f32 };
        let beat = |i: i64| Beat { at_ms: 500 * i - 250, downbeat: i % 4 == 0, sure: i % 3 != 0 };
        let a = TrackAnalysis { song_id: "a".into(), intro_bars: (0..4).map(bar).collect(), outro_bars: vec![bar(2)], intro_beats: (0..9).map(beat).collect(), outro_beats: vec![beat(7)], ..TrackAnalysis::default() };
        put(&c, &a).unwrap();
        let got = get(&c, "a").unwrap().expect("the row");
        assert_eq!((got.intro_bars.len(), got.outro_bars.len()), (4, 1));
        assert_eq!((&got.intro_beats, &got.outro_beats), (&a.intro_beats, &a.outro_beats), "beats come back as they went in");
        for (g, w) in got.intro_bars.iter().chain(&got.outro_bars).zip(a.intro_bars.iter().chain(&a.outro_bars)) {
            assert_eq!(g.start_ms, w.start_ms);
            assert!((g.level_db - w.level_db).abs() <= 0.25 && (g.low_db - w.low_db).abs() <= 0.25, "{g:?} {w:?}");
            assert!([(g.pulse, w.pulse), (g.voice, w.voice), (g.change, w.change)].iter().all(|(g, w)| (g - w).abs() <= 1.0 / 255.0), "{g:?} {w:?}");
        }
    }

    #[test]
    fn rows_keep_the_stored_format() {
        let c = nori_db::open("", "s").unwrap();
        // A row as stored: each column holds its position, so a field read from another column shows.
        let names = &COLUMNS[1..];
        let values: Vec<String> = (2..=COLUMNS.len()).map(|i| i.to_string()).collect();
        c.execute_batch(&format!("INSERT INTO track_analysis(server, song_id, {}) VALUES('s', 'a', {})", names.join(", "), values.join(", "))).unwrap();
        let a = get(&c, "a").unwrap().expect("the row");
        let shown = format!("{a:?}");
        for (name, v) in names.iter().zip(&values) {
            assert!([",", ".0,", " }", ".0 }"].iter().any(|end| shown.contains(&format!(" {name}: {v}{end}"))), "{name} is not {v}: {shown}");
        }
        put(&c, &TrackAnalysis { song_id: "b".into(), ..a }).unwrap();
        let stored = |id: &str| -> String {
            c.query_row(&format!("SELECT {} FROM track_analysis WHERE song_id=?1", names.join(" || ',' || ")), [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(stored("a"), stored("b"), "written back as read");
    }

    #[test]
    fn lyrics_say_where_a_song_is_sung() {
        let c = nori_db::open("", "s").unwrap();
        put(&c, &TrackAnalysis { song_id: "a".into(), duration_ms: 200_000, vocal_start_ms: 5_000, vocal_end_ms: 150_000, ..Default::default() }).unwrap();
        let sung = |id: &str| get_with_lyrics(&c, id).unwrap().map(|a| (a.vocal_start_ms, a.vocal_end_ms));
        assert_eq!(sung("a"), Some((5_000, 150_000)), "the voice measured");
        assert!(put_sung(&c, "a", (12_000, 190_000)).unwrap());
        assert!(!put_sung(&c, "a", (12_000, 190_000)).unwrap(), "unchanged");
        assert_eq!(sung("a"), Some((12_000, 190_000)));
        put_sung(&c, "a", (-300, 230_000)).unwrap();
        assert_eq!(sung("a"), Some((0, 200_000)), "within the song");
        put_sung(&c, "a", (210_000, 230_000)).unwrap();
        assert_eq!(sung("a"), Some((5_000, 150_000)), "words past its end say nothing");
        assert_eq!(sung("b"), None);
    }

    /// A song measured before the vocal curve's format changed is measured again, so its lyrics can be
    /// checked against its voice.
    #[test]
    fn an_unreadable_curve_is_measured_again() {
        let c = nori_db::open("", "s").unwrap();
        put(&c, &TrackAnalysis { song_id: "a".into(), analysis_version: ANALYSIS_VERSION, ..Default::default() }).unwrap();
        let ids = ["a".to_string()];
        assert_eq!(missing(&c, &ids).unwrap(), ids, "no curve");
        c.execute("INSERT INTO vocal_curve(server, song_id, curve) VALUES('s', 'a', ?1)", [vec![0u8; 16]]).unwrap();
        assert_eq!(missing(&c, &ids).unwrap(), ids, "a curve of an older format");
        put_voice(&c, "a", &VocalCurve { fps: 10.0, t0: 0.0, level: vec![1, 2, 3], loudness: vec![4, 5, 6] }).unwrap();
        assert!(missing(&c, &ids).unwrap().is_empty());
    }

    #[test]
    fn outdated_rows_are_counted_apart() {
        let c = nori_db::open("", "s").unwrap();
        let curve = VocalCurve { fps: 10.0, t0: 0.0, level: vec![1, 2, 3], loudness: vec![4, 5, 6] };
        for (id, version, voiced) in [("now", ANALYSIS_VERSION, true), ("old", ANALYSIS_VERSION - 1, true), ("mute", ANALYSIS_VERSION, false)] {
            put(&c, &TrackAnalysis { song_id: id.into(), analysis_version: version, ..Default::default() }).unwrap();
            if voiced {
                put_voice(&c, id, &curve).unwrap();
            }
        }
        assert_eq!(measured(&c).unwrap(), Measured { songs: 3, outdated: 2 });
        let ids: Vec<String> = ["now", "old", "mute"].map(String::from).into();
        assert_eq!(missing(&c, &ids).unwrap(), ["old", "mute"], "the same rows missing");
    }
}
