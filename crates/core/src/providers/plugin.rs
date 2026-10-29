//! A plugin's provider: [`Provider`] answered by the plugin's own instance, the core's records turned
//! into the plugin API's and back. Its calls run on the plugin's thread; async ones wait for it there,
//! the few the trait asks for at once hop to it. What it doesn't say (the settings it opens, a guest's
//! rules) is a provider's with no server: its library is the account's, never indexed.

use std::sync::Arc;

use nori_plugins::wit::provider as w;
use nori_plugins::convert::{album_in as album, artist_in as artist, found_in as found, playlist_in as playlist, song_in as song};
use nori_plugins::wit::types as t;
use nori_plugins::{CallResult, Plugins, Store, Worker};
use parking_lot::RwLock;

use super::{resolved_song, resolved_stream, sized, AccountState, Answer, FieldSpec, FormField, Provider, ProviderInfo, ResolveStream, SignIn, SignInResult};
use crate::browse::{AlbumSort, LibrarySection, ProfileRules, SettingsPart};
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Starrable, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::{block_on, NetError, Transport};
use crate::{Album, AlbumDetail, ArtistDetail, ArtistInfo, Core, FeedShelf, PlaylistDetail, Song, Starred};

/// An artist's picture on its about page, px.
const ABOUT_PX: u32 = 800;

/// What a plugin's stream addresses start with; the song's id follows.
const ADDRESS: &str = "plugin-song:";

pub(crate) struct PluginProvider {
    id: String,
    plugins: Arc<Plugins>,
    worker: Worker,
    /// Its add form and sign-in, asked once.
    info: std::sync::OnceLock<ProviderInfo>,
    /// Its rules as it last said, for the profile open, and how far ahead songs may be fetched.
    rules: RwLock<Option<(ProfileRules, Option<usize>)>>,
    /// The profile open, as [`Provider::configure`] was given it.
    open: RwLock<SavedServer>,
    /// Where its account stands, as it last said (asked with its rules).
    account: RwLock<AccountState>,
}

impl PluginProvider {
    pub(crate) fn new(id: &str, plugins: Arc<Plugins>) -> Option<PluginProvider> {
        let worker = plugins.worker(id)?;
        Some(PluginProvider { id: id.to_string(), plugins, worker, info: std::sync::OnceLock::new(), rules: RwLock::new(None), open: RwLock::default(), account: RwLock::new(AccountState::SignedOut) })
    }

    fn kind(&self) -> ServerKind {
        ServerKind::Plugin { id: self.id.clone() }
    }

    /// Runs `f` on the plugin's thread and waits for it here.
    fn now<R: Send + 'static>(&self, f: impl FnOnce(&mut Store, &nori_plugins::wit::Plugin) -> CallResult<R> + Send + 'static) -> Option<R> {
        block_on(self.worker.call(f))
    }

    /// Its rules for `profile`, and where its account stands, asked anew and kept.
    fn ask_rules(&self, profile: &SavedServer, guest: bool) -> ProfileRules {
        let p = profile_of(profile);
        let (said, account) = self
            .now(move |s, x| {
                let rules = x.nori_plugin_provider().call_rules(&mut *s, &p, guest)?;
                Ok((rules, x.nori_plugin_provider().call_account(s)?))
            })
            .unwrap_or((None, None));
        *self.account.write() = match account {
            Some(w::AccountState::SignedIn) => AccountState::SignedIn,
            Some(w::AccountState::Ended) => AccountState::Ended,
            Some(w::AccountState::SignedOut) | None => AccountState::SignedOut,
        };
        let ahead = said.as_ref().and_then(|r| r.ahead).map(|a| a as usize);
        let rules = rules(said, !self.info().sign_in.is_empty(), guest);
        *self.rules.write() = Some((rules.clone(), ahead));
        rules
    }
}

fn net_error(e: t::Error) -> NetError {
    match e.kind {
        t::ErrorKind::Network => NetError::Transport { kind: crate::transport::FailureKind::Connect, detail: Some(e.detail) },
        t::ErrorKind::Timeout => NetError::Transport { kind: crate::transport::FailureKind::Timeout, detail: Some(e.detail) },
        t::ErrorKind::Unauthorized => NetError::Http { status: 401 },
        t::ErrorKind::NotFound => NetError::Http { status: 404 },
        t::ErrorKind::Parse => NetError::Parse { reason: e.detail },
        t::ErrorKind::Refused | t::ErrorKind::Unsupported | t::ErrorKind::Other => NetError::Api { code: 0, reason: e.detail },
    }
}

/// A call the plugin couldn't answer (it trapped, or ran over its budget).
fn stopped() -> NetError {
    NetError::Api { code: 0, reason: "the plugin stopped".into() }
}

fn profile_of(p: &SavedServer) -> w::Profile {
    w::Profile { id: p.id.clone(), name: p.name.clone(), address: p.url.clone(), user: p.user.clone(), password: p.password.clone(), api_key: p.api_key.clone() }
}

/// nori's rules from what the plugin said; `signs_in` when its account signs in at all.
fn rules(said: Option<w::ProfileRules>, signs_in: bool, guest: bool) -> ProfileRules {
    use SettingsPart::*;
    let r = said.unwrap_or(w::ProfileRules { account: false, downloads: false, radio: false, feed: false, stars: false, scrobbles: false, sections: Vec::new(), measures: true, ahead: None });
    let sections = r.sections.iter().map(|s| match s {
        w::LibrarySection::Songs => LibrarySection::Songs,
        w::LibrarySection::Albums => LibrarySection::Albums,
        w::LibrarySection::Favorites => LibrarySection::Favourites,
        w::LibrarySection::History => LibrarySection::History,
        w::LibrarySection::Artists => LibrarySection::Artists,
        w::LibrarySection::Playlists => LibrarySection::Playlists,
        w::LibrarySection::Genres => LibrarySection::Genres,
        w::LibrarySection::Folders => LibrarySection::Folders,
        w::LibrarySection::Radio => LibrarySection::Radio,
        w::LibrarySection::Downloads => LibrarySection::Downloads,
    });
    let mut shown = ProfileRules {
        asks: guest,
        account: false,
        downloads: r.downloads && !guest,
        radio: r.radio && !guest,
        indexed: false,
        sections: if guest { Vec::new() } else { sections.collect() },
        settings: vec![Profiles, ServerOptions, Look, Playback, Sound, Lyrics, Downloads, Storage, About],
        remote: true,
        measures: r.measures && !guest,
        stars: r.stars && !guest,
        stars_after_sign_in: signs_in && !r.account && !guest,
        scrobbles: r.scrobbles && !guest,
        feed: r.feed,
        mixes: r.account && !guest,
    };
    if guest {
        shown.settings.retain(|p| !matches!(p, ServerOptions | Downloads));
    }
    shown
}

/// The plugin's request for `read`; None when a provider has nothing for it (the empty page).
fn request(read: &Read) -> Option<w::Request> {
    let n = |v: i32| v.max(0) as u32;
    Some(match read {
        Read::Search { query, songs, albums, artists } => w::Request::Search((query.clone(), n(*songs), n(*albums), n(*artists))),
        Read::AlbumById { id } | Read::AlbumSongs { id } => w::Request::Album(id.clone()),
        Read::ArtistById { id } | Read::TopSongs { artist: id } => w::Request::Artist(id.clone()),
        Read::ArtistAbout { id } => w::Request::ArtistAbout(id.clone()),
        Read::PlaylistById { id } | Read::PlaylistSongs { id } => w::Request::Playlist(id.clone()),
        Read::PlaylistList => w::Request::Playlists,
        Read::AlbumList { kind, size, offset, .. } => w::Request::Albums((sort(*kind), n(*size), n(*offset))),
        Read::FavouriteAlbums { size } => w::Request::Albums((w::AlbumSort::Starred, n(*size), 0)),
        Read::StarredItems => w::Request::Starred,
        Read::SimilarSongs { id, count } => w::Request::SimilarSongs((id.clone(), n(*count))),
        Read::SongById { id } => w::Request::Song(id.clone()),
        Read::HomeFeed => w::Request::HomeFeed,
        _ => return None,
    })
}

fn sort(k: AlbumSort) -> w::AlbumSort {
    match k {
        AlbumSort::Newest => w::AlbumSort::Newest,
        AlbumSort::Recent => w::AlbumSort::Recent,
        AlbumSort::Frequent => w::AlbumSort::Frequent,
        AlbumSort::Random => w::AlbumSort::Random,
        AlbumSort::Starred => w::AlbumSort::Starred,
        _ => w::AlbumSort::Alphabetical,
    }
}

/// The page `read` asked for, from the plugin's answer; None for nothing (the empty page).
fn page(read: &Read, p: w::Page) -> Option<Page> {
    Some(match (read, p) {
        (Read::AlbumSongs { .. }, w::Page::Album(a)) => Page::Songs { v: a.songs.into_iter().map(song).collect() },
        (Read::PlaylistSongs { .. }, w::Page::Playlist(l)) => Page::Songs { v: l.songs.into_iter().map(song).collect() },
        (Read::TopSongs { .. }, w::Page::Artist(a)) => Page::Songs { v: a.top_songs.into_iter().map(song).collect() },
        (Read::StarredItems, w::Page::Found(f)) => {
            let f = found(f);
            Page::StarredPage { v: Starred { artists: f.artists, albums: f.albums, songs: f.songs, ..Default::default() } }
        }
        (_, w::Page::Songs(v)) => Page::Songs { v: v.into_iter().map(song).collect() },
        (_, w::Page::OneSong(v)) => Page::OneSong { v: v.map(song) },
        (_, w::Page::Albums(v)) => Page::Albums { v: v.into_iter().map(album).collect() },
        (_, w::Page::Artists(v)) => Page::Artists { v: v.into_iter().map(artist).collect() },
        (_, w::Page::Playlists(v)) => Page::Playlists { v: v.into_iter().map(playlist).collect() },
        (_, w::Page::Album(a)) => {
            let songs: Vec<Song> = a.songs.into_iter().map(song).collect();
            let mut al = album(a.album);
            al.song_count = songs.len() as u32;
            al.duration = songs.iter().map(|s| s.duration).sum();
            Page::AlbumPage { v: AlbumDetail::new(al, songs, Vec::new()) }
        }
        (_, w::Page::Artist(a)) => {
            let albums: Vec<Album> = a.albums.into_iter().map(album).collect();
            let mut ar = artist(a.artist);
            ar.album_count = Some(albums.len() as u32);
            Page::ArtistPage { v: ArtistDetail::new(ar, albums, a.top_songs.into_iter().map(song).collect()) }
        }
        (_, w::Page::About(a)) => Page::About { v: ArtistInfo { biography: a.biography, image_url: a.image_url.map(|u| sized(&u, ABOUT_PX)), similar: a.similar.into_iter().map(artist).collect(), ..Default::default() } },
        (_, w::Page::Playlist(l)) => {
            let songs: Vec<Song> = l.songs.into_iter().map(song).collect();
            Page::PlaylistPage { v: PlaylistDetail::new(playlist(l.playlist), songs) }
        }
        (_, w::Page::Found(f)) => Page::Found { v: found(f) },
        (_, w::Page::Feed(v)) => Page::Feed {
            v: v.into_iter()
                .map(|s| FeedShelf {
                    title: s.title,
                    songs: s.songs.into_iter().map(song).collect(),
                    albums: s.albums.into_iter().map(album).collect(),
                    artists: s.artists.into_iter().map(artist).collect(),
                    playlists: s.playlists.into_iter().map(playlist).collect(),
                })
                .collect(),
        },
        (_, w::Page::None) => return None,
    })
}

/// The plugin's change for `w`; None for what a provider has nowhere to send.
fn change(c: Write) -> Option<w::Change> {
    let star = |k: Starrable| match k {
        Starrable::Song => w::StarKind::Song,
        Starrable::Album => w::StarKind::Album,
        Starrable::Artist => w::StarKind::Artist,
    };
    Some(match c {
        Write::Star { kind, id, on } => w::Change::Star((star(kind), id, on)),
        Write::CreatePlaylist { name, song_ids } => w::Change::CreatePlaylist((name, song_ids)),
        Write::AddToPlaylist { id, song_ids } => w::Change::AddToPlaylist((id, song_ids)),
        Write::RemoveFromPlaylist { id, index } => w::Change::RemoveFromPlaylist((id, index.max(0) as u32)),
        Write::RenamePlaylist { id, name } => w::Change::RenamePlaylist((id, name)),
        Write::DeletePlaylist { id } => w::Change::DeletePlaylist(id),
        Write::Scrobble { id, submission, time_ms } => w::Change::Scrobble((id, submission, time_ms)),
        Write::ReplacePlaylist { .. } | Write::CreateRadio { .. } | Write::DeleteRadio { .. } | Write::SaveQueue { .. } => return None,
    })
}

#[async_trait::async_trait]
impl Provider for PluginProvider {
    fn info(&self) -> ProviderInfo {
        self.info.get_or_init(|| self.ask_info()).clone()
    }

    fn ready(&self, profile: &SavedServer) -> bool {
        let p = profile_of(profile);
        self.now(move |s, x| x.nori_plugin_provider().call_ready(s, &p)).unwrap_or(false)
    }

    async fn check(&self, transport: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        self.plugins.set_transport(Some(transport));
        let p = profile_of(&profile);
        let checked = self.worker.call(move |s, x| x.nori_plugin_provider().call_check(s, &p)).await.ok_or_else(stopped)?.map_err(net_error)?;
        Ok(SavedServer { name: checked.name, url: checked.address, user: checked.user, password: checked.password, api_key: checked.api_key, ..profile })
    }

    fn rules(&self, profile: &SavedServer, guest: bool) -> ProfileRules {
        if let Some((r, _)) = self.rules.read().clone() {
            return r;
        }
        self.ask_rules(profile, guest)
    }

    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.serves(&format!("plugin:{}", self.id))?;
        *core.server.write() = crate::api::Server::default();
        *self.open.write() = profile.clone();
        let p = profile_of(profile);
        self.now(move |s, x| x.nori_plugin_provider().call_open(s, &p));
        let guest = nori_remote::is_guest_key(&profile.api_key);
        core.set_rules(self.ask_rules(profile, guest));
        *core.native_login.write() = None;
        *core.account.write() = None;
        Ok(())
    }

    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>> {
        let Some(asked) = request(&read) else { return Ok(None) };
        let said = self.worker.call(move |s, x| x.nori_plugin_provider().call_read(s, &asked)).await.ok_or_else(stopped)?;
        let said = self.heard(client, said)?;
        Ok(page(&read, said).map(|p| Answer::Page(Box::new(p))))
    }

    async fn write(&self, client: &Client, c: Write) -> NetResult<()> {
        let Some(c) = change(c) else { return Err(super::nowhere("place for this change")) };
        let said = self.worker.call(move |s, x| x.nori_plugin_provider().call_write(s, &c)).await.ok_or_else(stopped)?;
        self.heard(client, said)
    }

    /// The plugin resolves its songs where they open; the bytes are kept by song and bit rate.
    fn stream(&self, _: &Client, id: String, q: StreamQuality) -> StreamTarget {
        resolved_stream(ADDRESS, &id, &q)
    }

    fn ahead(&self) -> Option<usize> {
        self.rules.read().as_ref().and_then(|(_, a)| *a)
    }

    /// A plugin's covers are URLs with their size to fill in: nothing is asked of it per cover.
    fn cover_address(&self, _: &Core, cover: &str, size: u32) -> String {
        sized(cover, size)
    }

    fn resolver(&self, _client: &Arc<Client>) -> Option<Arc<dyn ResolveStream>> {
        Some(Arc::new(Resolver { worker: self.worker.clone(), parts: Default::default(), ciphers: Default::default() }))
    }

    fn sign_in_done(&self, address: &str, cookies: &str) -> bool {
        let (a, c) = (address.to_string(), cookies.to_string());
        self.now(move |s, x| x.nori_plugin_provider().call_sign_in_done(s, &a, &c)).unwrap_or(false)
    }

    async fn sign_in(&self, client: &Client, answer: String) -> SignInResult {
        self.plugins.set_transport(Some(client.transport.clone()));
        let said = self.worker.call(move |s, x| x.nori_plugin_provider().call_sign_in(s, &answer)).await.flatten();
        let result = match said {
            Some(w::SignInResult::SignedIn(name)) => SignInResult::SignedIn { name },
            Some(w::SignInResult::NoSession) => SignInResult::NoSession,
            Some(w::SignInResult::Unreachable(reason)) => SignInResult::Unreachable { reason },
            Some(w::SignInResult::Refused) | None => SignInResult::Refused,
        };
        let profile = self.open.read().clone();
        client.core.set_rules(self.ask_rules(&profile, false));
        result
    }

    fn sign_out(&self, client: &Client) {
        self.now(|s, x| x.nori_plugin_provider().call_sign_out(s));
        let profile = self.open.read().clone();
        client.core.set_rules(self.ask_rules(&profile, false));
    }

    fn account(&self, _client: &Client) -> AccountState {
        *self.account.read()
    }

    fn reminds_to_sign_in(&self, _client: &Client) -> bool {
        self.now(|s, x| x.nori_plugin_provider().call_reminds(s)).unwrap_or(false)
    }

    fn stop_reminding(&self, _client: &Client) {
        self.worker.post(|s, x| x.nori_plugin_provider().call_stop_reminding(s));
    }
}

impl PluginProvider {
    /// `said`'s answer as nori's; an account the source refused may have ended, so its rules are asked
    /// anew (the library and likes go while signed out).
    fn heard<T>(&self, client: &Client, said: Result<T, t::Error>) -> NetResult<T> {
        said.map_err(|e| {
            if e.kind == t::ErrorKind::Unauthorized {
                client.core.set_rules(self.ask_rules(&self.open.read().clone(), false));
            }
            net_error(e)
        })
    }

    fn ask_info(&self) -> ProviderInfo {
        let said = self.now(|s, x| x.nori_plugin_provider().call_info(s)).flatten();
        let said = said.unwrap_or(w::ProviderInfo { fields: Vec::new(), sign_in: Vec::new() });
        let field = |f: w::Field| match f {
            w::Field::Name => FormField::Name,
            w::Field::Address => FormField::Address,
            w::Field::User => FormField::User,
            w::Field::Password => FormField::Password,
            w::Field::SecondAddress => FormField::SecondAddress,
            w::Field::ApiKey => FormField::ApiKey,
            w::Field::Headers => FormField::Headers,
        };
        ProviderInfo {
            kind: self.kind(),
            fields: said.fields.into_iter().map(|f| FieldSpec { what: field(f.field), advanced: f.advanced }).collect(),
            sign_in: said.sign_in.into_iter().map(|s| match s {
                w::SignInWay::WebPage(start_url) => SignIn::WebPage { start_url },
                w::SignInWay::Paste => SignIn::Paste,
            }).collect(),
        }
    }
}

/// Opens a plugin's stream addresses at the URLs the plugin says, on a loader's thread.
struct Resolver {
    worker: Worker,
    /// How the last stream resolved is asked for, by address ([`ResolveStream::parts`]).
    parts: parking_lot::Mutex<std::collections::HashMap<String, super::StreamParts>>,
    /// How the last stream resolved is encrypted, by address ([`ResolveStream::cipher`]).
    ciphers: parking_lot::Mutex<std::collections::HashMap<String, super::StreamCipher>>,
}

impl Resolver {
    fn source(&self, address: &str, used: w::StreamUse) -> Option<NetResult<String>> {
        let (kbps, id) = resolved_song(ADDRESS, address)?;
        let kbps = (kbps > 0).then_some(kbps);
        let said = block_on(self.worker.call(move |s, x| {
            s.data_mut().take_encrypted();
            let src = x.nori_plugin_provider().call_source(&mut *s, &id, kbps, used)?;
            Ok((src, s.data_mut().take_encrypted()))
        }));
        let (src, encrypted) = match said.ok_or_else(stopped).and_then(|(r, e)| r.map(|src| (src, e)).map_err(net_error)) {
            Ok(said) => said,
            Err(e) => return Some(Err(e)),
        };
        let cipher = encrypted.filter(|e| e.url == src.url).map(|e| super::StreamCipher { key: e.key, iv: e.iv, skip: e.skip });
        match cipher {
            Some(c) => self.ciphers.lock().insert(address.to_string(), c),
            None => self.ciphers.lock().remove(address),
        };
        let mut parts = self.parts.lock();
        match src.length.zip(src.part) {
            Some((len, most)) => parts.insert(address.to_string(), super::StreamParts { len, most }),
            None => parts.remove(address),
        };
        Some(Ok(src.url))
    }
}

impl ResolveStream for Resolver {
    fn resolve(&self, address: &str) -> Option<NetResult<String>> {
        self.source(address, w::StreamUse::Playback)
    }

    fn resolve_rest(&self, address: &str, from: u64) -> Option<NetResult<String>> {
        self.source(address, w::StreamUse::Download(from))
    }

    fn parts(&self, address: &str) -> Option<super::StreamParts> {
        self.parts.lock().get(address).copied()
    }

    fn cipher(&self, address: &str) -> Option<super::StreamCipher> {
        self.ciphers.lock().get(address).copied()
    }

    fn refused(&self, address: &str) {
        if let Some((_, id)) = resolved_song(ADDRESS, address) {
            self.worker.post(move |s, x| x.nori_plugin_provider().call_refused(s, &id));
        }
    }
}
