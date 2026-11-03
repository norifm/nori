//! Moves stores kept as JSON (before 0.8) into their columns, once, when the database opens. Remove this
//! file and its calls in `open` once no install predates 0.8.

use nori_model::{Album, Artist, Song};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::items::{self, Item};
use crate::ItemKind;

/// The library index: rowids kept, so the search index still points at its rows.
pub(crate) fn items(c: &Connection) -> rusqlite::Result<()> {
    if !has_json(c, "items")? {
        return Ok(());
    }
    pack_gain(c)?;
    c.create_scalar_function("natural_key", 1, rusqlite::functions::FunctionFlags::SQLITE_UTF8 | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        Ok(ctx.get::<Option<String>>(0)?.map(|t| crate::natural_key(&t)))
    })?;
    let copy = |kind: ItemKind, cols: &[&str], sorted: &[&str]| {
        let keys: Vec<String> = sorted.iter().map(|c| format!("{c}_key")).collect();
        let key_values: Vec<String> = sorted.iter().map(|c| format!("natural_key({})", value(c))).collect();
        format!(
            "INSERT INTO items(rowid, server, kind, id, {}, {}) SELECT o.rowid, o.server, o.kind, o.id, {}, {} FROM items_json o WHERE o.kind={};",
            cols.join(", "),
            keys.join(", "),
            values(cols),
            key_values.join(", "),
            kind as i64
        )
    };
    let tx = c.unchecked_transaction()?;
    tx.execute_batch("ALTER TABLE items RENAME TO items_json;")?;
    for index in ["items_genre", "items_artist", "items_year", "items_duration", "items_created", "items_plays", "items_starred", "items_rated", "items_title_natural", "items_artist_natural", "items_album_natural"] {
        tx.execute_batch(&format!("DROP INDEX IF EXISTS {index};"))?;
    }
    tx.execute_batch(items::TABLE)?;
    tx.execute_batch(&copy(ItemKind::Song, Song::COLUMNS, Song::SORTED))?;
    tx.execute_batch(&copy(ItemKind::Album, Album::COLUMNS, Album::SORTED))?;
    tx.execute_batch(&copy(ItemKind::Artist, Artist::COLUMNS, Artist::SORTED))?;
    tx.execute_batch("DROP TABLE items_json;")?;
    tx.commit()
}

/// The downloads, each with the song it keeps.
pub(crate) fn downloads(c: &Connection) -> rusqlite::Result<()> {
    if !has_json(c, "downloads")? {
        return Ok(());
    }
    pack_gain(c)?;
    let tx = c.unchecked_transaction()?;
    tx.execute_batch("ALTER TABLE downloads RENAME TO downloads_json;")?;
    tx.execute_batch(items::DOWNLOADS)?;
    tx.execute_batch(&format!(
        "INSERT INTO downloads(server, id, ts, done, {}) SELECT o.server, o.id, o.ts, o.done, {} FROM downloads_json o; DROP TABLE downloads_json;",
        Song::COLUMNS.join(", "),
        values(Song::COLUMNS)
    ))?;
    tx.commit()
}

fn has_json(c: &Connection, table: &str) -> rusqlite::Result<bool> {
    let mut st = c.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|n| n == "json"))
}

/// The column values read from the old row `o`'s JSON, in `cols`' order.
fn values(cols: &[&str]) -> String {
    cols.iter().map(|c| value(c)).collect::<Vec<_>>().join(", ")
}

fn value(col: &str) -> String {
    // `o.json`: json_each has a hidden column of that name.
    let j = |path: &str| format!("json_extract(o.json, '$.{path}')");
    match col {
        // Written by nori's own serializer: booleans.
        "starred" | "is_external" | "is_compilation" => format!("coalesce({}, 0)", j(&camel(col))),
        "explicit_status" => format!("CASE {} WHEN 'clean' THEN 1 WHEN 'explicit' THEN 2 ELSE 0 END", j("explicitStatus")),
        "replay_gain" => {
            let f = |k: &str| j(&format!("replayGain.{k}"));
            format!("CASE WHEN {} IS NULL THEN NULL ELSE pack_gain({}, {}, {}, {}, {}, {}) END", j("replayGain"), f("trackGain"), f("albumGain"), f("trackPeak"), f("albumPeak"), f("baseGain"), f("fallbackGain"))
        }
        "artists" => "(SELECT group_concat(json_extract(value, '$.id') || char(31) || json_extract(value, '$.name'), char(30)) FROM json_each(o.json, '$.artists'))".to_string(),
        "release_types" => "(SELECT group_concat(value, char(30)) FROM json_each(o.json, '$.releaseTypes'))".to_string(),
        _ => j(&camel(col)),
    }
}

/// A column's name as serde wrote it (camelCase).
fn camel(col: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for ch in col.chars() {
        if ch == '_' {
            up = true;
        } else if up {
            out.extend(ch.to_uppercase());
            up = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// `pack_gain(six gains)`: the replay gain blob [`items`] keeps, NaN for a missing one.
fn pack_gain(c: &Connection) -> rusqlite::Result<()> {
    c.create_scalar_function("pack_gain", 6, rusqlite::functions::FunctionFlags::SQLITE_UTF8, |ctx| {
        let mut out = Vec::with_capacity(24);
        for k in 0..6 {
            let v = match ctx.get::<Value>(k)? {
                Value::Real(f) => f as f32,
                Value::Integer(n) => n as f32,
                _ => f32::NAN,
            };
            out.extend(v.to_le_bytes());
        }
        Ok(out)
    })
}
