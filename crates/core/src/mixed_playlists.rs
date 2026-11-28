//! nori's own playlists, kept on this device in the profile's rows: their songs may come from any of the
//! profile's sources, so no source keeps them. Each song is kept whole, its source with it, and never
//! indexed.

use rusqlite::{params, OptionalExtension};

use crate::menus::PlaylistEdit;
use crate::{db, Core, Playlist, PlaylistDetail, Result, Song};

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// nori's own playlists, the one changed last first.
    pub fn mixed_playlists(&self) -> Result<Vec<Playlist>> {
        let ids: Vec<String> = {
            let c = self.db.lock();
            let mut st = c.prepare_cached("SELECT id FROM mixed_playlists WHERE server=sid() ORDER BY changed_ms DESC, id")?;
            let ids = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            ids
        };
        Ok(ids.into_iter().filter_map(|id| self.mixed_playlist(id).ok().flatten()).map(|d| d.playlist).collect())
    }

    /// nori's own playlist `id` with its songs; None when there is none.
    pub fn mixed_playlist(&self, id: String) -> Result<Option<PlaylistDetail>> {
        let c = self.db.lock();
        let Some(name) = c.prepare_cached("SELECT name FROM mixed_playlists WHERE server=sid() AND id=?1")?.query_row([&id], |r| r.get::<_, String>(0)).optional()? else { return Ok(None) };
        let songs = nori_library::merged::marked(songs_of(&c, &id)?);
        let playlist = Playlist {
            id,
            name,
            song_count: songs.len() as u32,
            duration: songs.iter().map(|s| s.duration).sum(),
            cover_art: songs.iter().find_map(|s| s.cover_art.clone()),
            mixed: true,
            ..Default::default()
        };
        Ok(Some(PlaylistDetail::new(playlist, songs)))
    }

    /// A new playlist of nori's own named `name`, holding `songs`.
    pub fn mixed_playlist_create(&self, name: String, songs: Vec<Song>) -> Result<Playlist> {
        let id = {
            let mut c = self.db.lock();
            let tx = c.transaction()?;
            let id: i64 = tx.query_row("SELECT coalesce(max(CAST(id AS INTEGER)), 0) + 1 FROM mixed_playlists WHERE server=sid()", [], |r| r.get(0))?;
            tx.execute("INSERT INTO mixed_playlists(server, id, name, changed_ms) VALUES(sid(), ?1, ?2, ?3)", params![id.to_string(), name, db::now_ms()])?;
            tx.commit()?;
            id.to_string()
        };
        self.mixed_playlist_add(id.clone(), songs)?;
        Ok(self.mixed_playlist(id)?.map(|d| d.playlist).unwrap_or_default())
    }

    /// `songs` added at the end of nori's own playlist `id`.
    pub fn mixed_playlist_add(&self, id: String, songs: Vec<Song>) -> Result<()> {
        self.mixed_playlist_change(&id, |held| held.extend(songs))
    }

    /// Row `index` of nori's own playlist `id` moved or removed ([`PlaylistEdit`]); nothing for a row the
    /// edit can't reach.
    pub fn mixed_playlist_edit(&self, id: String, index: u32, edit: PlaylistEdit) -> Result<()> {
        let at = index as usize;
        self.mixed_playlist_change(&id, |held| {
            let to = match edit {
                PlaylistEdit::Remove if at < held.len() => {
                    held.remove(at);
                    return;
                }
                PlaylistEdit::Remove => return,
                PlaylistEdit::MoveUp => at.checked_sub(1),
                PlaylistEdit::MoveDown => Some(at + 1),
            };
            if let Some(to) = to.filter(|&to| to < held.len() && at < held.len()) {
                held.swap(at, to);
            }
        })
    }

    /// nori's own playlist `id` named `name`.
    pub fn mixed_playlist_rename(&self, id: String, name: String) -> Result<()> {
        self.db.lock().execute("UPDATE mixed_playlists SET name=?2, changed_ms=?3 WHERE server=sid() AND id=?1", params![id, name, db::now_ms()])?;
        Ok(())
    }

    /// nori's own playlist `id` gone, with its songs.
    pub fn mixed_playlist_delete(&self, id: String) -> Result<()> {
        let mut c = self.db.lock();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM mixed_playlist_songs WHERE server=sid() AND playlist=?1", [&id])?;
        tx.execute("DELETE FROM mixed_playlists WHERE server=sid() AND id=?1", [&id])?;
        tx.commit()?;
        Ok(())
    }
}

impl Core {
    /// nori's own playlist `id`'s songs changed by `change`, kept in their new order.
    fn mixed_playlist_change(&self, id: &str, change: impl FnOnce(&mut Vec<Song>)) -> Result<()> {
        let mut c = self.db.lock();
        let tx = c.transaction()?;
        let mut held = songs_of(&tx, id)?;
        change(&mut held);
        tx.execute("DELETE FROM mixed_playlist_songs WHERE server=sid() AND playlist=?1", [id])?;
        {
            let mut add = tx.prepare_cached("INSERT INTO mixed_playlist_songs(server, playlist, at, song) VALUES(sid(), ?1, ?2, ?3)")?;
            for (at, song) in held.iter().enumerate() {
                add.execute(params![id, at as i64, serde_json::to_string(song).unwrap_or_default()])?;
            }
        }
        tx.execute("UPDATE mixed_playlists SET changed_ms=?2 WHERE server=sid() AND id=?1", params![id, db::now_ms()])?;
        tx.commit()?;
        Ok(())
    }
}

/// nori's own playlist `id`'s songs, in order.
fn songs_of(c: &rusqlite::Connection, id: &str) -> rusqlite::Result<Vec<Song>> {
    let mut st = c.prepare_cached("SELECT song FROM mixed_playlist_songs WHERE server=sid() AND playlist=?1 ORDER BY at")?;
    let songs = st.query_map([id], |r| r.get::<_, String>(0))?.filter_map(|j| serde_json::from_str::<Song>(&j.ok()?).ok()).map(Song::dressed).collect();
    Ok(songs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, source: Option<&str>) -> Song {
        Song { id: id.into(), title: id.into(), duration: 10, source: source.map(String::from), ..Default::default() }
    }

    fn held(core: &Core, id: &str) -> Vec<(String, Option<String>)> {
        core.mixed_playlist(id.into()).unwrap().unwrap().songs.into_iter().map(|s| (s.id, s.source)).collect()
    }

    /// A playlist of nori's own keeps songs of any source in its order through every change, and is gone
    /// once deleted.
    #[test]
    fn a_mixed_playlist_keeps_songs_of_any_source() {
        let core = Core::new(String::new(), "home".into(), Default::default()).unwrap();
        let made = core.mixed_playlist_create("Both".into(), vec![song("a", None), song("b", Some("yt"))]).unwrap();
        assert_eq!((made.name.as_str(), made.song_count, made.duration, made.mixed), ("Both", 2, 20, true));
        core.mixed_playlist_add(made.id.clone(), vec![song("c", Some("nd2"))]).unwrap();
        core.mixed_playlist_edit(made.id.clone(), 2, PlaylistEdit::MoveUp).unwrap();
        core.mixed_playlist_edit(made.id.clone(), 0, PlaylistEdit::Remove).unwrap();
        core.mixed_playlist_edit(made.id.clone(), 1, PlaylistEdit::MoveDown).unwrap();
        let own = |id: &str, source: &str| (id.to_string(), Some(source.to_string()));
        assert_eq!(held(&core, &made.id), [own("c", "nd2"), own("b", "yt")], "a past the end move does nothing");
        core.mixed_playlist_create("Other".into(), Vec::new()).unwrap();
        core.mixed_playlist_rename(made.id.clone(), "Renamed".into()).unwrap();
        assert_eq!(core.mixed_playlists().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Renamed", "Other"], "the one changed last first");
        core.mixed_playlist_delete(made.id.clone()).unwrap();
        assert!(core.mixed_playlist(made.id).unwrap().is_none());
        assert_eq!(core.mixed_playlists().unwrap().len(), 1);
    }
}
