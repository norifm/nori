//! A local profile's sync: what the platform lists against the files kept in `local_files`. Only files
//! new or changed are read; a new file whose audio is a gone file's takes that file's row, so its song
//! keeps its id, plays and seal. When anything changed, the songs, albums and artists are made again
//! from every kept file and indexed, the seals carried over; nothing changed, nothing is written.

use std::collections::{HashMap, HashSet};
use std::io::Read as _;

use nori_local::group::{self, Entry};
use nori_local::scan::{self, Known};
use nori_local::tags::{self, Tags};
use nori_local::{DevicePlaylist, FileKind, Files, FoundFile};
use rusqlite::{params, Connection};

use super::SyncProgress;
use crate::{db, Core, IngestStats, Result};

/// Files read between writes of what was read: a first sync of a large library fills in parts.
const BATCH: usize = 256;

/// The `kv` key of the playlists found ([`Playlists`]).
pub(super) const PLAYLISTS: &str = "local_playlists";
/// The `kv` key of what the pictures and playlists listed were, to tell when they changed.
const OTHERS: &str = "local_others";

/// A playlist file found, with its songs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Playlist {
    pub(super) path: String,
    pub(super) name: String,
    pub(super) songs: Vec<String>,
}

/// What a file read changes of its row.
enum Kept {
    /// Row `row` is the file at `found` now (a changed file, or a moved one known again).
    Moved { row: i64, found: FoundFile, read: tags::Read },
    /// A song new to the library.
    New { found: FoundFile, read: tags::Read },
    /// A file that holds no music nori plays, kept so it is not read again until it changes.
    Left { found: FoundFile },
    /// Row `row`'s file is gone.
    Dropped { row: i64 },
}

/// Syncs `core`'s library with what `files` lists under `folders`, telling `progress` how far it is;
/// what the index holds after. While the files can't be read, the library stays as it is.
pub(super) async fn scan(core: &Core, files: &dyn Files, folders: &[String], mut progress: impl FnMut(Option<SyncProgress>)) -> Result<IngestStats> {
    let Some(listed) = files.list(folders) else { return core.index_size() };
    let of_kind = |kind| listed.iter().filter(move |f| nori_local::kind_of(&f.path) == Some(kind));
    let music: Vec<FoundFile> = of_kind(FileKind::Music).cloned().collect();
    let mut others: Vec<&FoundFile> = of_kind(FileKind::Picture).chain(of_kind(FileKind::Playlist)).collect();
    others.sort_by(|a, b| a.path.cmp(&b.path));
    let kept_lists = files.playlists();
    let others_now = others
        .iter()
        .map(|f| format!("{}\u{1f}{}\u{1f}{}", f.path, f.size, f.modified_ms))
        .chain(kept_lists.iter().map(|p| format!("{}\u{1f}{}\u{1f}{}", p.path, p.name, p.songs.join("\u{1f}"))))
        .collect::<Vec<_>>()
        .join("\u{1e}");
    let (known, others_then) = {
        let c = core.db.lock();
        (known(&c)?, db::kv_get(&c, OTHERS)?)
    };
    let mut plan = scan::plan(known, &music);
    if plan.is_empty() && others_then.as_deref() == Some(others_now.as_str()) {
        return core.index_size();
    }
    let of = plan.to_read() as u32;
    let mut read = 0;
    progress(Some(SyncProgress { read, of }));
    let mut batch = Vec::new();
    let changed = std::mem::take(&mut plan.changed);
    for (row, found) in changed {
        batch.push(match read_file(files, &found.path) {
            Some(r) => Kept::Moved { row, found, read: r },
            None => Kept::Left { found },
        });
        read += 1;
        flush(core, &mut batch, read, of, &mut progress)?;
        turn(read).await;
    }
    for found in std::mem::take(&mut plan.new) {
        batch.push(match read_file(files, &found.path) {
            Some(r) => match scan::rematch(&mut plan.gone, r.fingerprint) {
                Some(row) => Kept::Moved { row, found, read: r },
                None => Kept::New { found, read: r },
            },
            None => Kept::Left { found },
        });
        read += 1;
        flush(core, &mut batch, read, of, &mut progress)?;
        turn(read).await;
    }
    batch.extend(plan.gone.iter().map(|k| Kept::Dropped { row: k.row }));
    keep(core, std::mem::take(&mut batch))?;
    let pictures: Vec<String> = of_kind(FileKind::Picture).map(|f| f.path.clone()).collect();
    let mut playlists: Vec<DevicePlaylist> = of_kind(FileKind::Playlist)
        .filter_map(|f| {
            let m = nori_local::playlist::parse(&f.path, &text(files, &f.path)?);
            Some(DevicePlaylist { path: f.path.clone(), name: m.name, songs: m.paths })
        })
        .collect();
    playlists.extend(kept_lists);
    index(core, files, &pictures, &playlists)?;
    db::kv_put(&core.db.lock(), OTHERS, &others_now)?;
    core.cache_stale("page:")?;
    core.index_size()
}

/// Writes `batch` once it holds a batch's files, and says how far the sync is.
fn flush(core: &Core, batch: &mut Vec<Kept>, read: u32, of: u32, progress: &mut impl FnMut(Option<SyncProgress>)) -> Result<()> {
    if batch.len() >= BATCH {
        keep(core, std::mem::take(batch))?;
    }
    progress(Some(SyncProgress { read, of }));
    Ok(())
}

/// Lets a browser's worker, whose one thread also plays and answers the page, do its other work every
/// few files read; natively a sync has a thread of its own.
async fn turn(read: u32) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if read.is_multiple_of(16) {
        use nori_tasks::Tasks;
        nori_tasks::Local.sleep(nori_time::Duration::ZERO).await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = read;
}

/// The file at `path` read; None for one that is no music nori plays.
fn read_file(files: &dyn Files, path: &str) -> Option<tags::Read> {
    tags::read(files.open(path).ok()?, &nori_local::extension(path)).ok()
}

/// The text of the file at `path`.
fn text(files: &dyn Files, path: &str) -> Option<String> {
    let mut bytes = Vec::new();
    files.open(path).ok()?.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The music files the last sync kept.
fn known(c: &Connection) -> rusqlite::Result<Vec<Known>> {
    let mut st = c.prepare_cached("SELECT rowid, path, size, modified, fingerprint FROM local_files WHERE server=sid()")?;
    let rows = st.query_map([], |r| Ok(Known { row: r.get(0)?, path: r.get(1)?, size: r.get::<_, i64>(2)? as u64, modified_ms: r.get(3)?, fingerprint: r.get::<_, Option<i64>>(4)?.map(|f| f as u64) }))?;
    rows.collect()
}

/// Writes what reading files changed of their rows.
fn keep(core: &Core, batch: Vec<Kept>) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let now = db::now_ms();
    let mut c = core.db.lock();
    let tx = c.transaction()?;
    for k in batch {
        match k {
            Kept::Moved { row, found, read } => {
                // A path another row still holds (two files swapped) is that row's no longer.
                tx.execute("DELETE FROM local_files WHERE server=sid() AND path=?1 AND rowid<>?2", params![found.path, row])?;
                let tags = serde_json::to_string(&read.tags).expect("tags are plain data");
                tx.prepare_cached("UPDATE local_files SET path=?2, size=?3, modified=?4, fingerprint=?5, tags=?6 WHERE rowid=?1")?
                    .execute(params![row, found.path, found.size as i64, found.modified_ms, read.fingerprint as i64, tags])?;
            }
            Kept::New { found, read } => {
                let tags = serde_json::to_string(&read.tags).expect("tags are plain data");
                tx.prepare_cached("INSERT OR REPLACE INTO local_files(server, path, size, modified, fingerprint, added, tags) VALUES(sid(), ?1, ?2, ?3, ?4, ?5, ?6)")?
                    .execute(params![found.path, found.size as i64, found.modified_ms, read.fingerprint as i64, now, tags])?;
            }
            Kept::Left { found } => {
                tx.prepare_cached("INSERT INTO local_files(server, path, size, modified, fingerprint, added, tags) VALUES(sid(), ?1, ?2, ?3, NULL, ?4, NULL)
                     ON CONFLICT(server, path) DO UPDATE SET size=?2, modified=?3, fingerprint=NULL, tags=NULL")?
                    .execute(params![found.path, found.size as i64, found.modified_ms, now])?;
            }
            Kept::Dropped { row } => {
                tx.prepare_cached("DELETE FROM local_files WHERE rowid=?1")?.execute([row])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Makes the songs, albums and artists of every kept file, covers among `pictures` or else found by
/// `files`, and indexes them in place of what the index held; the `playlists` listing them are kept too.
fn index(core: &Core, files: &dyn Files, pictures: &[String], playlists: &[DevicePlaylist]) -> Result<()> {
    let entries = {
        let c = core.db.lock();
        let mut st = c.prepare_cached("SELECT rowid, path, size, added, tags FROM local_files WHERE server=sid() AND tags IS NOT NULL")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?)))?;
        rows.map(|r| {
            let (row, path, size, added, tags) = r?;
            Ok(Entry { row, path, size: size as u64, added: nori_library::smart::iso_from_ms(added), tags: serde_json::from_str::<Tags>(&tags).unwrap_or_default() })
        })
        .collect::<rusqlite::Result<Vec<Entry>>>()?
    };
    let mut lib = group::group(&entries, pictures, &|music| files.album_art(music));
    let mut c = core.db.lock();
    let sealed = sealed(&c)?;
    let has = |kind: db::ItemKind, id: &str| sealed.contains(&(kind as i64, id.to_string()));
    lib.songs.iter_mut().for_each(|s| s.starred = has(db::ItemKind::Song, &s.id));
    lib.albums.iter_mut().for_each(|a| a.starred = has(db::ItemKind::Album, &a.id));
    lib.artists.iter_mut().for_each(|a| a.starred = has(db::ItemKind::Artist, &a.id));
    let kept: HashSet<i64> = db::index(&mut c, &lib.artists, &lib.albums, &lib.songs)?.into_iter().collect();
    db::prune(&mut c, &kept)?;
    let by_path: HashMap<&str, &str> = lib.songs.iter().filter_map(|s| Some((s.path.as_deref()?, s.id.as_str()))).collect();
    let lists: Vec<Playlist> = playlists
        .iter()
        .map(|p| Playlist { path: p.path.clone(), name: p.name.clone(), songs: p.songs.iter().filter_map(|s| by_path.get(s.as_str()).map(|id| id.to_string())).collect() })
        .filter(|p| !p.songs.is_empty())
        .collect();
    db::kv_put(&c, PLAYLISTS, &serde_json::to_string(&lists).expect("playlists are plain data"))?;
    Ok(())
}

/// The items sealed now, by kind and id.
fn sealed(c: &Connection) -> rusqlite::Result<HashSet<(i64, String)>> {
    let mut st = c.prepare_cached("SELECT kind, id FROM items WHERE server=sid() AND starred=1")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}
