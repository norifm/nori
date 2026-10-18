//! The `track_analysis` and `vocal_curve` tables, and the streaming analyser that feeds them.

use nori_model::TrackAnalysis;
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
            Ok(TrackAnalysis { $($field: column!(r, at.next().expect("unbounded") $(, $default)?)),* })
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
    intro_end_ms, outro_start_ms, outro_vocal or 0.0, intro_vocal or 0.0, outro_centroid or 0.0, intro_centroid or 0.0,
    analysed_ms, outro_bpm or 0.0, outro_bpm_confidence or 0.0, outro_beat_offset_ms or 0.0, outro_stability or 0.0,
    outro_downbeat_phase or 0, intro_bpm or 0.0, intro_bpm_confidence or 0.0, intro_beat_offset_ms or 0.0,
    intro_stability or 0.0, intro_downbeat_phase or 0, beats_per_bar or 0, drop_ms or 0, drop_runup_vocal or 0.0,
    drop_vocal or 0.0, exit_ms or 0, gap_ms or 0, gap_end_ms or 0, exit_vocal or 0.0, drop_runup_tonal_db or 0.0,
    intro_beats_per_bar or 0, outro_beats_per_bar or 0, intro_grid_source or 0, outro_grid_source or 0,
}

pub fn put(c: &Connection, a: &TrackAnalysis) -> rusqlite::Result<()> {
    let marks: Vec<String> = (1..=COLUMNS.len()).map(|i| format!("?{i}")).collect();
    c.prepare_cached(&format!("INSERT OR REPLACE INTO track_analysis(server, {}) VALUES(sid(), {})", COLUMNS.join(", "), marks.join(",")))?
        .execute(&values(a)[..])
        .map(|_| ())
}

pub fn get(c: &Connection, song_id: &str) -> rusqlite::Result<Option<TrackAnalysis>> {
    c.prepare_cached(&format!("SELECT {} FROM track_analysis WHERE server=sid() AND song_id=?1", COLUMNS.join(", ")))?.query_row([song_id], row).optional()
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

/// Stores a whole song's measurement: its analysis (keeping the beat model's grids) and vocal curve.
pub fn put_finished(c: &Connection, song_id: &str, features: &crate::analysis::Features) -> rusqlite::Result<TrackAnalysis> {
    let stored = put_measured(c, crate::finish(song_id, features).track)?;
    put_voice(c, song_id, &features.voice_curve())?;
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

/// The `ids` with no row or a row from an older analysis version, in order.
pub fn missing(c: &Connection, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut st = c.prepare_cached("SELECT analysis_version FROM track_analysis WHERE server=sid() AND song_id=?1")?;
    let mut out = Vec::new();
    for id in ids {
        let v: Option<i32> = st.query_row([id], |r| r.get(0)).optional()?;
        if v.is_none_or(|v| v < ANALYSIS_VERSION) {
            out.push(id.clone());
        }
    }
    Ok(out)
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
}
