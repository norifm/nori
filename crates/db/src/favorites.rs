//! Favorites kept on this device, for a profile whose account can't keep them: each item's record as
//! JSON (enough to draw its row and open it), by kind and id, with when it was sealed. Not the search
//! index: nothing here is searched or synced.

use rusqlite::{params, Connection};

use crate::ItemKind;

pub(crate) const TABLE: &str =
    "CREATE TABLE IF NOT EXISTS favorites_here(server TEXT NOT NULL, kind INTEGER NOT NULL, id TEXT NOT NULL, record TEXT NOT NULL, sealed_ms INTEGER NOT NULL, PRIMARY KEY(server, kind, id)) WITHOUT ROWID;";

/// One favorite kept here.
#[derive(Debug, Clone, PartialEq)]
pub struct Favorite {
    pub kind: ItemKind,
    pub id: String,
    /// The item's record, as JSON.
    pub record: String,
    pub sealed_ms: i64,
}

impl rusqlite::types::FromSql for ItemKind {
    fn column_result(v: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        match v.as_i64()? {
            0 => Ok(ItemKind::Artist),
            1 => Ok(ItemKind::Album),
            2 => Ok(ItemKind::Song),
            n => Err(rusqlite::types::FromSqlError::OutOfRange(n)),
        }
    }
}

fn favorite(r: &rusqlite::Row, from: usize) -> rusqlite::Result<Favorite> {
    Ok(Favorite { kind: r.get(from)?, id: r.get(from + 1)?, record: r.get(from + 2)?, sealed_ms: r.get(from + 3)? })
}

/// This profile's favorites kept here, the newest sealed first.
pub fn favorites(c: &Connection) -> rusqlite::Result<Vec<Favorite>> {
    let mut st = c.prepare_cached("SELECT kind, id, record, sealed_ms FROM favorites_here WHERE server=sid() ORDER BY sealed_ms DESC, id")?;
    let rows = st.query_map([], |r| favorite(r, 0))?;
    rows.collect()
}

/// Keeps `f` for this profile, replacing one of its kind and id.
pub fn keep(c: &Connection, f: &Favorite) -> rusqlite::Result<()> {
    c.prepare_cached("INSERT OR REPLACE INTO favorites_here(server, kind, id, record, sealed_ms) VALUES(sid(), ?1, ?2, ?3, ?4)")?.execute(params![f.kind, f.id, f.record, f.sealed_ms])?;
    Ok(())
}

/// Drops this profile's favorite of `kind` and `id`; whether there was one.
pub fn drop(c: &Connection, kind: ItemKind, id: &str) -> rusqlite::Result<bool> {
    Ok(c.prepare_cached("DELETE FROM favorites_here WHERE server=sid() AND kind=?1 AND id=?2")?.execute(params![kind, id])? > 0)
}

/// Every profile's favorites kept here, by profile id, for a backup.
pub fn every(c: &Connection) -> rusqlite::Result<Vec<(String, Favorite)>> {
    let mut st = c.prepare("SELECT server, kind, id, record, sealed_ms FROM favorites_here ORDER BY server, sealed_ms DESC, id")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, favorite(r, 1)?)))?;
    rows.collect()
}

/// Keeps a backup's favorites, each under its profile id; one kept here already stays as it is.
pub fn restore(c: &mut Connection, favorites: &[(String, Favorite)]) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    {
        let mut st = tx.prepare("INSERT OR IGNORE INTO favorites_here(server, kind, id, record, sealed_ms) VALUES(?1, ?2, ?3, ?4, ?5)")?;
        for (server, f) in favorites {
            st.execute(params![server, f.kind, f.id, f.record, f.sealed_ms])?;
        }
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(kind: ItemKind, id: &str, ms: i64) -> Favorite {
        Favorite { kind, id: id.into(), record: format!("{{\"id\":\"{id}\"}}"), sealed_ms: ms }
    }

    /// Each profile keeps its own, the newest first; a kind and id is kept once, and the same id of
    /// another kind is another favorite.
    #[test]
    fn favorites_are_kept_per_profile() {
        let dir = nori_testdir::TempDir::new("db-favorites");
        let path = dir.join("f.db").to_string_lossy().into_owned();
        let a = crate::open(&path, "a").unwrap();
        let b = crate::open(&path, "b").unwrap();
        keep(&a, &fav(ItemKind::Song, "s1", 1)).unwrap();
        keep(&a, &fav(ItemKind::Album, "s1", 2)).unwrap();
        keep(&a, &fav(ItemKind::Song, "s2", 3)).unwrap();
        keep(&a, &fav(ItemKind::Song, "s1", 4)).unwrap();
        keep(&b, &fav(ItemKind::Artist, "r1", 5)).unwrap();
        let ids = |c: &Connection| favorites(c).unwrap().into_iter().map(|f| (f.kind, f.id)).collect::<Vec<_>>();
        assert_eq!(ids(&a), [(ItemKind::Song, "s1".into()), (ItemKind::Song, "s2".into()), (ItemKind::Album, "s1".into())]);
        assert_eq!(ids(&b), [(ItemKind::Artist, "r1".into())]);
        assert!(drop(&a, ItemKind::Song, "s1").unwrap());
        assert!(!drop(&a, ItemKind::Song, "s1").unwrap(), "already gone");
        assert_eq!(ids(&a), [(ItemKind::Song, "s2".into()), (ItemKind::Album, "s1".into())]);
        crate::db_forget_server(path.clone(), "a".into()).unwrap();
        assert!(favorites(&a).unwrap().is_empty(), "a removed profile's go with it");
        assert_eq!(ids(&b).len(), 1);
    }

    /// A restore adds the backup's favorites beside those here and keeps one here as it is.
    #[test]
    fn a_restore_keeps_those_here() {
        let mut c = crate::open("", "a").unwrap();
        keep(&c, &fav(ItemKind::Song, "s1", 9)).unwrap();
        restore(&mut c, &[("a".into(), fav(ItemKind::Song, "s1", 1)), ("a".into(), fav(ItemKind::Album, "l1", 2)), ("z".into(), fav(ItemKind::Song, "s1", 3))]).unwrap();
        let all: Vec<(String, ItemKind, i64)> = every(&c).unwrap().into_iter().map(|(s, f)| (s, f.kind, f.sealed_ms)).collect();
        assert_eq!(all, [("a".into(), ItemKind::Song, 9), ("a".into(), ItemKind::Album, 2), ("z".into(), ItemKind::Song, 3)]);
    }
}
