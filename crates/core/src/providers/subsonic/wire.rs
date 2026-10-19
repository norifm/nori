//! A Subsonic server's answers (`subsonic-response`) and the pages they make.

use serde::Deserialize;

use crate::cache_policy::Page;
use crate::client::NetResult;
use crate::{lyrics, Album, AlbumDetail, Artist, ArtistDetail, ArtistInfo, CoreError, DiscTitle, Directory, Genre, MusicFolder, PlayQueue, Playlist, PlaylistDetail, RadioStation, Result, SearchResult, ServerInfo, Song, Starred};

/// How an answer becomes a page.
#[derive(Clone, Copy)]
pub(super) enum Parser {
    AlbumList,
    Artists,
    Album,
    AlbumSongs,
    Artist,
    ArtistInfo,
    Songs,
    OneSong,
    Playlists,
    Playlist,
    PlaylistSongs,
    Starred,
    Genres,
    Radio,
    Lyrics,
    Indexes,
    Directory,
    Search,
    Folders,
    Status,
    Share,
    Queue,
}

/// `r` as the page `parser` makes of it.
pub(super) fn page(parser: Parser, r: Response) -> NetResult<Page> {
    Ok(match parser {
        Parser::AlbumList => Page::Albums { v: r.album_list2.unwrap_or_default().album },
        Parser::Artists => Page::Artists { v: r.artists.unwrap_or_default().artists() },
        Parser::Indexes => Page::Artists { v: r.indexes.unwrap_or_default().artists() },
        Parser::Album => Page::AlbumPage { v: r.album.unwrap_or_default().into() },
        Parser::AlbumSongs => Page::Songs { v: r.album.unwrap_or_default().song },
        Parser::Artist => Page::ArtistPage { v: r.artist.unwrap_or_default().into() },
        Parser::ArtistInfo => Page::About { v: r.artist_info2.unwrap_or_default().into() },
        Parser::Songs => Page::Songs { v: r.songs() },
        Parser::OneSong => Page::OneSong { v: r.songs().into_iter().next() },
        Parser::Playlists => Page::Playlists { v: r.playlists.unwrap_or_default().playlist },
        Parser::Playlist => Page::PlaylistPage { v: r.playlist.unwrap_or_default().into() },
        Parser::PlaylistSongs => Page::Songs { v: r.playlist.unwrap_or_default().entry },
        Parser::Starred => Page::StarredPage { v: r.starred2.unwrap_or_default().into() },
        Parser::Genres => {
            let mut v = r.genres.unwrap_or_default().genre;
            v.sort_by_key(|g| std::cmp::Reverse(g.song_count));
            Page::Genres { v }
        }
        Parser::Radio => Page::Stations { v: r.internet_radio_stations.unwrap_or_default().station },
        Parser::Lyrics => Page::LyricsPage { v: lyrics::build(r.lyrics_list.unwrap_or_default().structured_lyrics) },
        Parser::Directory => Page::DirectoryPage { v: r.directory.unwrap_or_default().into() },
        Parser::Search => Page::Found { v: r.search_result3.unwrap_or_default().into() },
        Parser::Folders => Page::Folders { v: r.music_folders.unwrap_or_default().music_folder },
        Parser::Status => Page::Status { v: r.into() },
        Parser::Share => {
            let url = r.shares.and_then(|s| s.share.into_iter().next()).map(|s| s.url).filter(|u| !u.is_empty());
            Page::ShareUrl { v: url.ok_or(CoreError::Parse { reason: "no share in response".into() })? }
        }
        Parser::Queue => {
            let q = r.play_queue.unwrap_or_default();
            let by = q.changed_by.clone();
            Page::Queue { v: q.into(), by }
        }
    })
}

/// A library walk's page: what it lists.
pub(crate) fn found(body: &[u8]) -> Result<SearchResult> {
    Ok(parse(body)?.search_result3.unwrap_or_default().into())
}

impl Response {
    /// The server's name for itself (`navidrome`); empty when it gives none.
    pub(super) fn kind(&self) -> &str {
        &self.kind
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiError {
    code: i32,
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Found {
    artist: Vec<Artist>,
    album: Vec<Album>,
    song: Vec<Song>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AlbumWire {
    #[serde(flatten)]
    album: Album,
    song: Vec<Song>,
    #[serde(rename = "discTitles")]
    disc_titles: Vec<DiscTitle>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ArtistWire {
    #[serde(flatten)]
    artist: Artist,
    album: Vec<Album>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PlaylistWire {
    #[serde(flatten)]
    playlist: Playlist,
    entry: Vec<Song>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Albums {
    album: Vec<Album>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Songs {
    song: Vec<Song>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Index {
    artist: Vec<Artist>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Artists {
    index: Vec<Index>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Playlists {
    playlist: Vec<Playlist>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Genres {
    genre: Vec<Genre>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Stations {
    #[serde(rename = "internetRadioStation")]
    station: Vec<RadioStation>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ArtistInfoWire {
    biography: Option<String>,
    last_fm_url: Option<String>,
    music_brainz_id: Option<String>,
    large_image_url: Option<String>,
    medium_image_url: Option<String>,
    similar_artist: Vec<Artist>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LyricsList {
    structured_lyrics: Vec<lyrics::Structured>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Folders {
    music_folder: Vec<MusicFolder>,
}

/// getMusicDirectory mixes folders and songs in one `child` list, told apart by `isDir`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct DirectoryWire {
    #[serde(deserialize_with = "crate::model::id_string")]
    id: String,
    name: String,
    child: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Share {
    url: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Shares {
    share: Vec<Share>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct QueueWire {
    entry: Vec<Song>,
    current: Option<serde_json::Value>,
    position: u64,
    /// The app that saved it, by the name it gave the server.
    #[serde(rename = "changedBy")]
    changed_by: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Response {
    status: String,
    version: String,
    #[serde(rename = "type")]
    kind: String,
    server_version: String,
    open_subsonic: bool,
    error: Option<ApiError>,
    search_result3: Option<Found>,
    starred2: Option<Found>,
    album: Option<AlbumWire>,
    artist: Option<ArtistWire>,
    playlist: Option<PlaylistWire>,
    album_list2: Option<Albums>,
    artists: Option<Artists>,
    playlists: Option<Playlists>,
    genres: Option<Genres>,
    random_songs: Option<Songs>,
    songs_by_genre: Option<Songs>,
    similar_songs2: Option<Songs>,
    top_songs: Option<Songs>,
    song: Option<Song>,
    internet_radio_stations: Option<Stations>,
    artist_info2: Option<ArtistInfoWire>,
    lyrics_list: Option<LyricsList>,
    play_queue: Option<QueueWire>,
    shares: Option<Shares>,
    music_folders: Option<Folders>,
    indexes: Option<Artists>,
    directory: Option<DirectoryWire>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "subsonic-response")]
    response: Response,
}

pub(crate) fn parse(body: &[u8]) -> Result<Response> {
    let r = serde_json::from_slice::<Envelope>(body).map_err(|e| CoreError::Parse { reason: e.to_string() })?.response;
    if r.status != "ok" {
        let e = r.error.unwrap_or_default();
        return Err(CoreError::Api { code: e.code, reason: if e.message.is_empty() { "request failed".into() } else { e.message } });
    }
    Ok(r)
}

impl Response {
    /// randomSongs, songsByGenre, similarSongs2, topSongs and getSong all land here.
    fn songs(self) -> Vec<Song> {
        let lists = self.random_songs.or(self.songs_by_genre).or(self.similar_songs2).or(self.top_songs);
        lists.map(|s| s.song).or(self.song.map(|s| vec![s])).unwrap_or_default()
    }
}

impl Artists {
    fn artists(self) -> Vec<Artist> {
        self.index.into_iter().flat_map(|i| i.artist).collect()
    }
}

impl From<Response> for ServerInfo {
    fn from(r: Response) -> Self {
        ServerInfo { version: r.version, server_type: r.kind, server_version: r.server_version, open_subsonic: r.open_subsonic }
    }
}

impl From<Found> for SearchResult {
    fn from(f: Found) -> Self {
        SearchResult { artists: f.artist, albums: f.album, songs: f.song, playlists: Vec::new() }
    }
}

impl From<Found> for Starred {
    fn from(f: Found) -> Self {
        Starred::new(f.artist, f.album, f.song)
    }
}

impl From<AlbumWire> for AlbumDetail {
    fn from(a: AlbumWire) -> Self {
        AlbumDetail::new(a.album, a.song, a.disc_titles)
    }
}

impl From<ArtistWire> for ArtistDetail {
    fn from(a: ArtistWire) -> Self {
        ArtistDetail::new(a.artist, a.album, Vec::new())
    }
}

impl From<PlaylistWire> for PlaylistDetail {
    fn from(p: PlaylistWire) -> Self {
        PlaylistDetail::new(p.playlist, p.entry)
    }
}

impl From<ArtistInfoWire> for ArtistInfo {
    fn from(i: ArtistInfoWire) -> Self {
        ArtistInfo {
            biography: i.biography.filter(|b| !b.trim().is_empty()),
            image_url: i.large_image_url.or(i.medium_image_url).filter(|u| !u.is_empty()),
            similar: i.similar_artist,
            last_fm_url: i.last_fm_url.filter(|u| u.starts_with("http")),
            music_brainz_id: i.music_brainz_id.filter(|m| !m.is_empty()),
        }
    }
}

impl From<DirectoryWire> for Directory {
    fn from(d: DirectoryWire) -> Self {
        let mut out = Directory { id: d.id, name: d.name, ..Default::default() };
        for c in d.child {
            if c.get("isDir").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Ok(mut a) = serde_json::from_value::<Artist>(c.clone()) {
                    if a.name.is_empty() {
                        a.name = c.get("title").and_then(|t| t.as_str()).unwrap_or_default().to_string();
                    }
                    out.folders.push(a);
                }
            } else if let Ok(s) = serde_json::from_value::<Song>(c) {
                out.songs.push(s);
            }
        }
        out
    }
}

impl From<QueueWire> for PlayQueue {
    fn from(q: QueueWire) -> Self {
        let current = q.current.map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
        let index = current.and_then(|c| q.entry.iter().position(|s| s.id == c)).unwrap_or(0) as u32;
        PlayQueue { songs: q.entry, index, position_ms: q.position, origin: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_lyrics_preferred() {
        let l = lyrics::build(parse(r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[
          {"synced":false,"line":[{"value":"plain"}]},{"synced":true,"line":[{"start":1500,"value":"timed"}]}]}}}"#.as_bytes()).unwrap().lyrics_list.unwrap_or_default().structured_lyrics);
        assert!(l.synced);
        assert_eq!((l.lines[0].start_ms, l.lines[0].text.as_str(), l.lines[0].words.len()), (1500, "timed", 1));
    }
}
