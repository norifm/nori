//! Each read as a Subsonic request, and the page its answer makes.

use super::wire::{self, Parser};
use crate::browse::AlbumSort;
use crate::cache_policy::{this_year, Page, Read};
use crate::client::{pairs, Client, NetResult};

/// An endpoint, its parameters and how its answer is read.
type Request = (&'static str, Vec<(String, String)>, Parser);

/// The request `read` makes; None for a read a server has no answer for.
pub(super) fn request(read: &Read) -> Option<Request> {
    let id = |id: &str| pairs(&[("id", id.to_string())]);
    let by_year = |from: i32, to: i32, size: i32, offset: i32| {
        let p = pairs(&[("type", "byYear".into()), ("fromYear", from.to_string()), ("toYear", to.to_string()), ("size", size.to_string()), ("offset", offset.to_string())]);
        ("getAlbumList2", p, Parser::AlbumList)
    };
    Some(match read {
        Read::AlbumList { kind: AlbumSort::ByYear, size, offset, .. } => by_year(this_year(), 0, *size, *offset),
        Read::AlbumList { kind, size, offset, genre } => {
            let mut p = pairs(&[("type", kind.api().into()), ("size", size.to_string()), ("offset", offset.to_string())]);
            if let Some(g) = genre {
                p.push(("genre".into(), g.clone()));
            }
            ("getAlbumList2", p, Parser::AlbumList)
        }
        Read::AlbumsByYear { from, to, size, offset } => by_year(*from, *to, *size, *offset),
        Read::FavouriteAlbums { size } => ("getAlbumList2", pairs(&[("type", "starred".into()), ("size", size.to_string()), ("offset", "0".into())]), Parser::AlbumList),
        Read::ArtistIndex => ("getArtists", vec![], Parser::Artists),
        Read::AlbumById { id: i } => ("getAlbum", id(i), Parser::Album),
        Read::ArtistById { id: i } => ("getArtist", id(i), Parser::Artist),
        Read::ArtistAbout { id: i } => ("getArtistInfo2", pairs(&[("id", i.clone()), ("count", "10".into())]), Parser::ArtistInfo),
        Read::TopSongs { artist } => ("getTopSongs", pairs(&[("artist", artist.clone()), ("count", "20".into())]), Parser::Songs),
        Read::PlaylistList => ("getPlaylists", vec![], Parser::Playlists),
        Read::PlaylistById { id: i } => ("getPlaylist", id(i), Parser::Playlist),
        Read::StarredItems => ("getStarred2", vec![], Parser::Starred),
        Read::GenreList => ("getGenres", vec![], Parser::Genres),
        Read::RadioList => ("getInternetRadioStations", vec![], Parser::Radio),
        Read::LyricsBySong { song_id } => ("getLyricsBySongId", pairs(&[("id", song_id.clone()), ("enhanced", "true".into())]), Parser::Lyrics),
        Read::FolderIndex => ("getIndexes", vec![], Parser::Indexes),
        Read::FolderById { id: i } => ("getMusicDirectory", id(i), Parser::Directory),
        Read::MusicFolders => ("getMusicFolders", vec![], Parser::Folders),
        Read::Ping => ("ping", vec![], Parser::Status),
        Read::Search { query, songs, albums, artists } => (
            "search3",
            pairs(&[("query", query.clone()), ("songCount", songs.to_string()), ("albumCount", albums.to_string()), ("artistCount", artists.to_string())]),
            Parser::Search,
        ),
        Read::SongPage { offset, count } => (
            "search3",
            pairs(&[("query", String::new()), ("songCount", count.to_string()), ("songOffset", offset.to_string()), ("albumCount", "0".into()), ("artistCount", "0".into())]),
            Parser::Search,
        ),
        Read::RandomSongs { size, genre } => {
            let mut p = pairs(&[("size", size.to_string())]);
            if let Some(g) = genre {
                p.push(("genre".into(), g.clone()));
            }
            ("getRandomSongs", p, Parser::Songs)
        }
        Read::SongsByGenre { genre, count, offset } => ("getSongsByGenre", pairs(&[("genre", genre.clone()), ("count", count.to_string()), ("offset", offset.to_string())]), Parser::Songs),
        Read::SimilarSongs { id: i, count } => ("getSimilarSongs2", pairs(&[("id", i.clone()), ("count", count.to_string())]), Parser::Songs),
        Read::SongById { id: i } => ("getSong", id(i), Parser::OneSong),
        Read::AlbumSongs { id: i } => ("getAlbum", id(i), Parser::AlbumSongs),
        Read::PlaylistSongs { id: i } => ("getPlaylist", id(i), Parser::PlaylistSongs),
        Read::NowPlaying { id: i } => ("scrobble", pairs(&[("id", i.clone()), ("submission", "false".into())]), Parser::Status),
        Read::ShareLink { id: i } => ("createShare", id(i), Parser::Share),
        Read::PullQueue => ("getPlayQueue", vec![], Parser::Queue),
        Read::HomeFeed => return None,
    })
}

/// The server's answer `body` to `read` as a page.
pub(super) fn page(client: &Client, read: &Read, body: &[u8]) -> NetResult<Page> {
    let r = wire::parse(body)?;
    client.core.heard_from_server(r.kind());
    let (_, _, parser) = request(read).ok_or_else(|| crate::transport::NetError::Parse { reason: format!("no answer to {read:?}") })?;
    wire::page(parser, r)
}

/// The music folders `read`'s answer is limited to, as its parameters say them.
pub(super) fn scope(client: &Client, read: &Read) -> String {
    let Some((endpoint, _, _)) = request(read) else { return String::new() };
    client.scoped(endpoint, Vec::new()).into_iter().map(|(k, v)| format!("&{k}={v}")).collect()
}
