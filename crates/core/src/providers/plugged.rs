//! The open profile as plugins reach it (`nori_plugins::Library`): its queue, its library through the
//! profile's client, and its Subsonic API signed here. Every client hands the profile's client over
//! with [`Kinds::open_client`], so a plugin's session reads the same queue and library on each.

use std::sync::{Arc, Weak};

use nori_model::Song;
use nori_net::requests::{Starrable, Write};
use nori_net::transport::NetError;
use nori_plugins::wit::types as t;
use nori_plugins::{Found, ItemKind, Library, Profile, Repeat};

use crate::cache_policy::{Page, Read};
use crate::client::Client;
use crate::providers::Kinds;
use crate::settings::ServerKind;

/// The profile's client, held weakly: a plugin never keeps a closed profile open.
struct Opened(Weak<Client>);

fn failed(e: NetError) -> t::Error {
    let kind = match &e {
        NetError::Transport { kind: nori_net::transport::FailureKind::Timeout, .. } => t::ErrorKind::Timeout,
        NetError::Transport { .. } => t::ErrorKind::Network,
        NetError::Http { status: 401 | 403 } => t::ErrorKind::Unauthorized,
        NetError::Http { status: 404 } | NetError::Api { code: 70, .. } => t::ErrorKind::NotFound,
        NetError::Api { code: 40 | 41 | 50, .. } => t::ErrorKind::Unauthorized,
        NetError::Http { .. } | NetError::Api { .. } => t::ErrorKind::Refused,
        NetError::Parse { .. } => t::ErrorKind::Parse,
        NetError::Db { .. } => t::ErrorKind::Other,
    };
    t::Error { kind, detail: e.to_string() }
}

fn closed() -> t::Error {
    t::Error { kind: t::ErrorKind::Network, detail: "the profile closed".into() }
}

impl Opened {
    fn client(&self) -> Result<Arc<Client>, t::Error> {
        self.0.upgrade().ok_or_else(closed)
    }

    fn read(&self, read: Read) -> Result<Page, t::Error> {
        let c = self.client()?;
        nori_tasks::block_on(c.read_now(read)).map_err(failed)
    }
}

impl Library for Opened {
    fn profile(&self) -> Option<Profile> {
        let c = self.0.upgrade()?;
        let p = c.core.session.settings.current()?;
        let s = p.servers.iter().find(|s| s.id == p.active_server_id)?;
        let kind = match &s.kind {
            ServerKind::Subsonic => "subsonic".to_string(),
            ServerKind::Plugin { id } => id.clone(),
        };
        Some(Profile { id: s.id.clone(), kind, name: s.name.clone() })
    }

    fn queue(&self, start: u32, count: u32) -> Vec<Song> {
        let Some(c) = self.0.upgrade() else { return Vec::new() };
        let q = &c.core.session;
        let ids: Vec<String> = q.playlist(|p| p.ids().iter().skip(start as usize).take(count as usize).cloned().collect());
        q.songs(ids)
    }

    fn queue_length(&self) -> u32 {
        self.0.upgrade().map_or(0, |c| c.core.session.playlist(|p| p.ids().len() as u32))
    }

    fn queue_rev(&self) -> u64 {
        self.0.upgrade().map_or(0, |c| c.core.session.rev())
    }

    fn shuffle(&self) -> bool {
        self.0.upgrade().is_some_and(|c| c.core.session.shuffle_shown())
    }

    fn repeat(&self) -> Repeat {
        // media3's numbering, as the queue keeps it.
        match self.0.upgrade().map_or(0, |c| c.core.session.playlist(|p| p.repeat())) {
            1 => Repeat::One,
            2 => Repeat::All,
            _ => Repeat::Off,
        }
    }

    fn songs(&self, ids: &[String]) -> Result<Vec<Song>, t::Error> {
        let c = self.client()?;
        ids.iter()
            .map(|id| match c.core.session.song(id) {
                Some(s) => Ok(s),
                None => match self.read(Read::SongById { id: id.clone() })? {
                    Page::OneSong { v: Some(s) } => Ok(s),
                    _ => Err(t::Error { kind: t::ErrorKind::NotFound, detail: id.clone() }),
                },
            })
            .collect()
    }

    fn search(&self, query: &str, count: u32) -> Result<Vec<Found>, t::Error> {
        let n = count as i32;
        let Page::Found { v } = self.read(Read::Search { query: query.into(), songs: n, albums: n, artists: n })? else { return Ok(Vec::new()) };
        let songs = v.songs.into_iter().map(|s| Found::Song(Box::new(s)));
        let albums = v.albums.into_iter().map(Found::Album);
        let artists = v.artists.into_iter().map(Found::Artist);
        Ok(songs.chain(albums).chain(artists).chain(v.playlists.into_iter().map(Found::Playlist)).take(count as usize).collect())
    }

    fn lookup(&self, kind: ItemKind, id: &str) -> Result<Found, t::Error> {
        let id = id.to_string();
        let not_found = || t::Error { kind: t::ErrorKind::NotFound, detail: String::new() };
        match (kind, self.read(lookup_read(kind, id))?) {
            (ItemKind::Song, Page::OneSong { v: Some(s) }) => Ok(Found::Song(Box::new(s))),
            (ItemKind::Album, Page::AlbumPage { v }) => Ok(Found::Album(v.album)),
            (ItemKind::Artist, Page::ArtistPage { v }) => Ok(Found::Artist(v.artist)),
            (ItemKind::Playlist, Page::PlaylistPage { v }) => Ok(Found::Playlist(v.playlist)),
            _ => Err(not_found()),
        }
    }

    fn songs_of(&self, kind: ItemKind, id: &str) -> Result<Vec<Song>, t::Error> {
        let id = id.to_string();
        match kind {
            ItemKind::Song => self.songs(&[id]),
            ItemKind::Artist => Ok(self.read(Read::TopSongs { artist: id })?.songs()),
            _ => Ok(self.read(lookup_read(kind, id))?.songs()),
        }
    }

    fn star(&self, kind: ItemKind, id: &str, on: bool) -> Result<(), t::Error> {
        let kind = match kind {
            ItemKind::Song => Starrable::Song,
            ItemKind::Album => Starrable::Album,
            ItemKind::Artist => Starrable::Artist,
            ItemKind::Playlist => return Err(t::Error { kind: t::ErrorKind::Unsupported, detail: "a playlist has no star".into() }),
        };
        let c = self.client()?;
        nori_tasks::block_on(c.star(kind, id.to_string(), on, Arc::new(Unshown))).map_err(failed)
    }

    fn add_to_playlist(&self, playlist: &str, ids: &[String]) -> Result<(), t::Error> {
        let c = self.client()?;
        nori_tasks::block_on(c.write(Write::AddToPlaylist { id: playlist.to_string(), song_ids: ids.to_vec() })).map_err(failed)
    }

    fn subsonic(&self, method: &str, params: &[(String, String)]) -> Result<Vec<u8>, t::Error> {
        let c = self.client()?;
        if !c.core.provider().subsonic_api() {
            return Err(t::Error { kind: t::ErrorKind::Unsupported, detail: "the profile has no Subsonic API".into() });
        }
        let url = c.core.server.read().url(method, params);
        nori_tasks::block_on(nori_net::transport::get(&*c.transport, url, 0)).map_err(failed)
    }

    fn synced_lyrics(&self, song: &Song) -> Option<Vec<(u32, String)>> {
        let c = self.0.upgrade()?;
        let kept = Arc::new(Kept::default());
        nori_tasks::block_on(c.lyrics_for(song.id.clone(), kept.clone())).ok()?;
        let lyrics = kept.0.lock().take()?;
        Some(lyrics.lines.into_iter().filter(|l| l.start_ms >= 0).map(|l| (l.start_ms as u32, l.text)).collect())
    }
}

fn lookup_read(kind: ItemKind, id: String) -> Read {
    match kind {
        ItemKind::Song => Read::SongById { id },
        ItemKind::Album => Read::AlbumById { id },
        ItemKind::Artist => Read::ArtistById { id },
        ItemKind::Playlist => Read::PlaylistById { id },
    }
}

/// A star a plugin set: the screens read it with the session's marks when they draw next.
struct Unshown;

impl crate::library::StarsShown for Unshown {
    fn marks(&self, _: crate::stars::StarMarks) {}
}

/// The last synced lyrics a lookup showed.
#[derive(Default)]
struct Kept(parking_lot::Mutex<Option<nori_model::Lyrics>>);

impl nori_lyrics::race::LyricsShown for Kept {
    fn show(&self, pick: nori_lyrics::race::LyricsPick) {
        if pick.lyrics.synced {
            *self.0.lock() = Some(pick.lyrics);
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Kinds {
    /// `client`'s profile is the one open now: plugins' sessions reach it, and those that hear it are
    /// told it opened.
    pub fn open_client(&self, client: Arc<Client>) {
        let Some(p) = &self.plugins else { return };
        p.set_library(Some(Arc::new(Opened(Arc::downgrade(&client)))));
        p.set_transport(Some(client.transport.clone()));
        if p.hears(nori_plugins::Hook::ProfileOpened) {
            let id = Opened(Arc::downgrade(&client)).profile().map(|p| p.id).unwrap_or_default();
            p.profile_opened(id);
        }
    }
}

impl Kinds {
    /// The plugin host, where the client has one.
    pub fn plugins(&self) -> Option<&Arc<nori_plugins::Plugins>> {
        self.plugins.as_ref()
    }
}
