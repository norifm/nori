//! A LAN jam guest's provider: its host's, asked through the host's door. Its reads are the pages the
//! host's provider gives, and its streams the URLs the host's provider resolves, fetched here; the guest
//! needs neither the host's plugin nor its account. The host's side, answering those asks, is here too.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nori_remote::wire::{HostPage, HostRead, HostSource};
use parking_lot::Mutex;

use super::{nowhere, resolved_song, resolved_stream, sized, Answer, Provider, ProviderInfo, ResolveStream, StreamParts};
use crate::browse::ProfileRules;
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::{block_on, Exchange, NetError, Transport, TransportResponse};
use crate::{AlbumDetail, ArtistDetail, Core, PlaylistDetail, SearchResult};

/// What a guest's stream addresses start with; the bit rate and song follow ([`resolved_stream`]).
const ADDRESS: &str = "jam-song:";

/// How long the host has to resolve a song's source.
const SOURCE_TIMEOUT_MS: u32 = 15_000;

/// The provider of a LAN jam's guest profile, of the host's kind.
pub(crate) struct HostedProvider {
    pub(crate) kind: ServerKind,
}

#[async_trait::async_trait]
impl Provider for HostedProvider {
    /// Never offered: a guest profile is made by joining.
    fn info(&self) -> ProviderInfo {
        ProviderInfo { kind: self.kind.clone(), fields: Vec::new(), sign_in: Vec::new() }
    }

    fn ready(&self, _: &SavedServer) -> bool {
        false
    }

    async fn check(&self, _: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        Ok(profile)
    }

    /// A guest's, with no library of its own: it searches and opens the host's pages.
    fn rules(&self, _: &SavedServer, _: bool) -> ProfileRules {
        ProfileRules { sections: Vec::new(), ..crate::browse::profile_rules(true) }
    }

    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.serves(&format!("jam:{}", profile.url))?;
        *core.server.write() = crate::api::Server::default();
        core.set_rules(self.rules(profile, true));
        *core.native_login.write() = None;
        *core.account.write() = None;
        Ok(())
    }

    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>> {
        let Some(asked) = host_read(&read) else { return Ok(None) };
        let body = serde_json::to_string(&asked).map_err(|e| NetError::Parse { reason: e.to_string() })?;
        let url = door_url(client, "noriRemote.read", &[], "POST", body.as_bytes())?;
        let page = answered(client.transport.send(Exchange { url, json: Some(body), ..Default::default() }).await?)?;
        Ok(Some(Answer::Page(Box::new(page_of(&read, page)))))
    }

    async fn write(&self, _: &Client, _: Write) -> NetResult<()> {
        Err(nowhere("place for this change"))
    }

    fn stream(&self, _: &Client, id: String, q: StreamQuality) -> StreamTarget {
        resolved_stream(ADDRESS, &id, &q)
    }

    /// Each song fetched ahead is an ask of the host: only the next one is.
    fn ahead(&self) -> Option<usize> {
        Some(0)
    }

    fn cover_address(&self, _: &Core, cover: &str, size: u32) -> String {
        sized(cover, size)
    }

    fn resolver(&self, client: &Arc<Client>) -> Option<Arc<dyn ResolveStream>> {
        Some(Arc::new(HostedResolver { client: client.clone(), parts: Mutex::default(), refused: Mutex::default() }))
    }
}

/// `endpoint` at the door of the jam this guest profile is in, signed with its member key.
fn door_url(client: &Client, endpoint: &str, params: &[(String, String)], method: &str, body: &[u8]) -> NetResult<String> {
    crate::remote::lan_jam_url(&client.core, endpoint, params, method, body).ok_or_else(|| nowhere("jam host"))
}

/// A door's JSON answer; its refusal as the status it came with.
fn answered<T: serde::de::DeserializeOwned>(r: TransportResponse) -> NetResult<T> {
    if !(200..300).contains(&r.status) {
        return Err(NetError::Http { status: r.status });
    }
    serde_json::from_slice(&r.body).map_err(|e| NetError::Parse { reason: e.to_string() })
}

/// What the host is asked for `read`; None for a read a guest has nothing for (the empty page).
fn host_read(read: &Read) -> Option<HostRead> {
    let n = |v: i32| v.max(0) as u32;
    Some(match read {
        Read::Search { query, songs, albums, artists } => HostRead::Search { query: query.clone(), songs: n(*songs), albums: n(*albums), artists: n(*artists) },
        Read::AlbumById { id } | Read::AlbumSongs { id } => HostRead::Album { id: id.clone() },
        Read::ArtistById { id } | Read::TopSongs { artist: id } => HostRead::Artist { id: id.clone() },
        Read::PlaylistById { id } | Read::PlaylistSongs { id } => HostRead::Playlist { id: id.clone() },
        Read::SongById { id } => HostRead::Song { id: id.clone() },
        _ => return None,
    })
}

/// The page `read` asked for, from the host's answer.
fn page_of(read: &Read, page: HostPage) -> Page {
    match (read, page) {
        (Read::AlbumSongs { .. }, HostPage::Album { songs, .. }) | (Read::PlaylistSongs { .. }, HostPage::Playlist { songs, .. }) | (Read::TopSongs { .. }, HostPage::Artist { top_songs: songs, .. }) => Page::Songs { v: songs },
        (_, HostPage::Found { songs, albums, artists, playlists }) => Page::Found { v: SearchResult { songs, albums, artists, playlists } },
        (_, HostPage::Album { album, songs }) => Page::AlbumPage { v: AlbumDetail::new(*album, songs, Vec::new()) },
        (_, HostPage::Artist { artist, albums, top_songs }) => Page::ArtistPage { v: ArtistDetail::new(artist, albums, top_songs) },
        (_, HostPage::Playlist { playlist, songs }) => Page::PlaylistPage { v: PlaylistDetail::new(playlist, songs) },
        (_, HostPage::Song { song }) => Page::OneSong { v: song.map(|s| *s) },
    }
}

/// Opens a guest's stream addresses at the URLs the host's provider resolves, on a loader's thread.
struct HostedResolver {
    client: Arc<Client>,
    /// How the last stream resolved is asked for, by address ([`ResolveStream::parts`]).
    parts: Mutex<HashMap<String, StreamParts>>,
    /// Addresses whose last URL was refused: the host resolves them anew.
    refused: Mutex<HashSet<String>>,
}

impl ResolveStream for HostedResolver {
    fn resolve(&self, address: &str) -> Option<NetResult<String>> {
        let (kbps, id) = resolved_song(ADDRESS, address)?;
        let mut params = vec![("id".to_string(), id), ("kbps".to_string(), kbps.to_string())];
        if self.refused.lock().remove(address) {
            params.push(("anew".into(), "1".into()));
        }
        let asked = door_url(&self.client, "noriRemote.source", &params, "GET", b"").and_then(|url| answered::<HostSource>(block_on(self.client.transport.get(url, SOURCE_TIMEOUT_MS))?));
        let source = match asked {
            Ok(source) => source,
            Err(e) => return Some(Err(e)),
        };
        let mut parts = self.parts.lock();
        match source.parts {
            Some(p) => parts.insert(address.to_string(), StreamParts { len: p.len, most: p.most }),
            None => parts.remove(address),
        };
        Some(Ok(source.url))
    }

    fn refused(&self, address: &str) {
        self.refused.lock().insert(address.to_string());
    }

    fn parts(&self, address: &str) -> Option<StreamParts> {
        self.parts.lock().get(address).copied()
    }
}

/// The page the host's provider gives for a guest's `read`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn answer_read(client: &Client, read: HostRead) -> NetResult<HostPage> {
    let n = |v: u32| v.min(i32::MAX as u32) as i32;
    let read = match read {
        HostRead::Search { query, songs, albums, artists } => Read::Search { query, songs: n(songs), albums: n(albums), artists: n(artists) },
        HostRead::Album { id } => Read::AlbumById { id },
        HostRead::Artist { id } => Read::ArtistById { id },
        HostRead::Playlist { id } => Read::PlaylistById { id },
        HostRead::Song { id } => Read::SongById { id },
    };
    use crate::{Album, Artist, Song};
    // The host's favorites are its own: a guest sees none of them.
    let songs = |v: Vec<Song>| v.into_iter().map(|s| Song { starred: false, ..s }).collect::<Vec<_>>();
    let albums = |v: Vec<Album>| v.into_iter().map(|a| Album { starred: false, ..a }).collect::<Vec<_>>();
    let artist = |a: Artist| Artist { starred: false, ..a };
    Ok(match client.cached_or_fetched(read).await? {
        Some(Page::Found { v }) => HostPage::Found { songs: songs(v.songs), albums: albums(v.albums), artists: v.artists.into_iter().map(artist).collect(), playlists: v.playlists },
        Some(Page::AlbumPage { v }) => HostPage::Album { album: Box::new(Album { starred: false, ..v.album }), songs: songs(v.songs) },
        Some(Page::ArtistPage { v }) => HostPage::Artist { artist: artist(v.artist), albums: albums(v.albums), top_songs: songs(v.songs) },
        Some(Page::PlaylistPage { v }) => HostPage::Playlist { playlist: v.playlist, songs: songs(v.songs) },
        Some(Page::OneSong { v }) => HostPage::Song { song: v.map(|s| Box::new(Song { starred: false, ..s })) },
        page => return Err(NetError::Parse { reason: format!("no guest page in {page:?}") }),
    })
}

/// Where song `id` streams from at most `kbps` (0: as it is), as the host's provider resolves it; `anew`
/// when the guest's last URL was refused. Only a provider that resolves its addresses hands one out: an
/// address that is a URL already may carry the account.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn answer_source(client: &Arc<Client>, id: String, kbps: u32, anew: bool) -> NetResult<HostSource> {
    let address = client.core.provider().stream(client, id, StreamQuality { bit_rate: kbps, format: String::new() }).url;
    let resolver = client.stream_resolver().ok_or_else(|| nowhere("stream a guest may fetch"))?;
    if anew {
        resolver.refused(&address);
    }
    let url = resolver.resolve(&address).ok_or_else(|| nowhere("stream a guest may fetch"))??;
    let parts = resolver.parts(&address).map(|p| nori_remote::wire::SourceParts { len: p.len, most: p.most });
    Ok(HostSource { url, parts })
}
