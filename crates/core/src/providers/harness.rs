//! The differential harness: one scripted session of reads and writes per provider, each step's pages,
//! freshness and requests written as a transcript and compared with the one recorded from the code
//! before providers shared the page store (tests/golden/). `NORI_BLESS=1` writes it anew; every change
//! to a transcript is an old bug with its own test.

use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::cache_policy::{Page, Read, Stored};
use crate::client::{Client, NetResult};
use crate::transport::{Exchange, FailureKind, Network, Transport, TransportError, TransportResponse};
use crate::{Album, Artist, Song};

/// A session's steps as text.
pub(crate) struct Transcript {
    name: &'static str,
    out: String,
}

impl Transcript {
    pub(crate) fn new(name: &'static str) -> Self {
        Transcript { name, out: String::new() }
    }

    pub(crate) fn step(&mut self, label: &str) {
        let _ = writeln!(self.out, "== {label}");
    }

    pub(crate) fn line(&mut self, s: impl std::fmt::Display) {
        let _ = writeln!(self.out, "{s}");
    }

    pub(crate) fn page(&mut self, p: &Page) {
        self.line(format!("page {}", said(p)));
    }

    pub(crate) fn stored(&mut self, read: &Read, s: &NetResult<Stored>) {
        match s {
            Ok(s) => self.line(format!("stored {read:?}: {} digest={} fresh={}", s.page.as_ref().map_or("none".into(), said), s.digest.is_some(), s.fresh)),
            Err(e) => self.line(format!("stored {read:?}: error {e:?}")),
        }
    }

    pub(crate) fn result<T>(&mut self, what: &str, r: &NetResult<T>) {
        match r {
            Ok(_) => self.line(format!("{what}: ok")),
            Err(e) => self.line(format!("{what}: error {e:?}")),
        }
    }

    /// Compares with the golden transcript, or writes it with `NORI_BLESS=1`.
    pub(crate) fn finish(self) {
        let path = format!("{}/tests/golden/{}.txt", env!("CARGO_MANIFEST_DIR"), self.name);
        if std::env::var("NORI_BLESS").is_ok_and(|v| v == "1") {
            std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
            std::fs::write(&path, &self.out).unwrap();
            return;
        }
        let golden = std::fs::read_to_string(&path).unwrap_or_default();
        if golden != self.out {
            let at = golden.lines().zip(self.out.lines()).position(|(a, b)| a != b).unwrap_or_else(|| golden.lines().count().min(self.out.lines().count()));
            let step = self.out.lines().take(at + 1).filter(|l| l.starts_with("== ")).last().unwrap_or_default().to_string();
            std::fs::write(format!("{path}.new"), &self.out).unwrap();
            panic!(
                "{} differs from {path} at line {} ({step}):\n  was: {}\n  now: {}\n(the whole run is in {path}.new)",
                self.name,
                at + 1,
                golden.lines().nth(at).unwrap_or("<end>"),
                self.out.lines().nth(at).unwrap_or("<end>")
            );
        }
    }
}

fn hash(p: &Page) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{p:?}").hash(&mut h);
    format!("{:016x}", h.finish())
}

fn song(s: &Song) -> String {
    format!("{}{}/{}", s.id, if s.starred { "*" } else { "" }, s.duration)
}

fn album(a: &Album) -> String {
    format!("{}{}/{}", a.id, if a.starred { "*" } else { "" }, a.song_count)
}

fn artist(a: &Artist) -> String {
    format!("{}{}", a.id, if a.starred { "*" } else { "" })
}

fn list<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
    v.iter().map(f).collect::<Vec<_>>().join(",")
}

/// What a page shows, short, and a hash of all of it.
pub(crate) fn said(p: &Page) -> String {
    let short = match p {
        Page::Albums { v } => format!("albums [{}]", list(v, album)),
        Page::Artists { v } => format!("artists [{}]", list(v, artist)),
        Page::AlbumPage { v } => format!("album {} [{}]", album(&v.album), list(&v.songs, song)),
        Page::ArtistPage { v } => format!("artist {} [{}] [{}]", artist(&v.artist), list(&v.albums, album), list(&v.songs, song)),
        Page::About { v } => format!("about {:?} [{}]", v.image_url, list(&v.similar, artist)),
        Page::Songs { v } => format!("songs [{}]", list(v, song)),
        Page::OneSong { v } => format!("song {:?}", v.as_ref().map(song)),
        Page::Playlists { v } => format!("playlists [{}]", list(v, |p| format!("{}/{}", p.id, p.song_count))),
        Page::PlaylistPage { v } => format!("playlist {}/{} [{}]", v.playlist.id, v.playlist.duration, list(&v.songs, song)),
        Page::StarredPage { v } => format!("starred [{}] [{}] [{}] {}", list(&v.artists, artist), list(&v.albums, album), list(&v.songs, song), v.library_songs),
        Page::Genres { v } => format!("genres [{}]", list(v, |g| format!("{}:{}", g.name, g.song_count))),
        Page::Stations { v } => format!("stations [{}]", list(v, |s| s.id.clone())),
        Page::LyricsPage { v } => format!("lyrics synced={} lines={}", v.synced, v.lines.len()),
        Page::DirectoryPage { v } => format!("folder {} [{}] [{}]", v.id, list(&v.folders, artist), list(&v.songs, song)),
        Page::Found { v } => format!("found [{}] [{}] [{}] [{}]", list(&v.artists, artist), list(&v.albums, album), list(&v.songs, song), list(&v.playlists, |p| p.id.clone())),
        Page::Folders { v } => format!("folders [{}]", list(v, |f| f.name.clone())),
        Page::Status { v } => format!("status {v:?}"),
        Page::ShareUrl { v } => format!("share {v}"),
        Page::Queue { v, by } => format!("queue [{}] at {} {} ms by {by:?}", list(&v.songs, song), v.index, v.position_ms),
        Page::Feed { v } => format!(
            "feed {}",
            list(v, |s| format!("{{{}: [{}] [{}] [{}] [{}]}}", s.title, list(&s.songs, song), list(&s.albums, album), list(&s.artists, artist), list(&s.playlists, |p| p.id.clone())))
        ),
    };
    format!("{short} #{}", hash(p))
}

/// Ages every kept answer past its freshness window.
pub(crate) fn age(c: &Client) {
    c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
    c.core.kept.lock().values_mut().for_each(|k| k.at = 0);
}

/// Every page `read_each` shows, then how it ended.
pub(crate) fn each(t: &mut Transcript, c: &Client, read: Read) {
    let mut pages = Vec::new();
    let r = crate::client::tests::block(c.read_each(read.clone(), |p| pages.push(p)));
    t.line(format!("read_each {read:?}"));
    for p in &pages {
        t.page(p);
    }
    t.result("ended", &r);
}

/// `read_now`'s page.
pub(crate) fn now(t: &mut Transcript, c: &Client, read: Read) {
    let r = crate::client::tests::block(c.read_now(read.clone()));
    t.line(format!("read_now {read:?}"));
    match &r {
        Ok(p) => t.page(p),
        Err(e) => t.line(format!("error {e:?}")),
    }
}

pub(crate) struct Marked;

impl crate::library::StarsShown for Marked {
    fn marks(&self, _: crate::stars::StarMarks) {}
}

/// A Subsonic server answering by endpoint (or by endpoint and parameters, first), recording what was
/// asked without the address and the signature; offline it answers nothing.
#[derive(Default)]
struct Server {
    answers: Mutex<std::collections::HashMap<String, String>>,
    asked: Mutex<Vec<String>>,
    offline: Mutex<bool>,
}

impl Server {
    fn answer(&self, key: &str, body: &str) {
        self.answers.lock().insert(key.into(), format!(r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","type":"navidrome",{body}}}}}"#));
    }

    fn refuse(&self, key: &str, code: i32) {
        self.answers.lock().insert(key.into(), format!(r#"{{"subsonic-response":{{"status":"failed","error":{{"code":{code},"message":"no"}}}}}}"#));
    }

    /// What was asked since last taken.
    fn taken(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock())
    }
}

#[async_trait::async_trait]
impl Transport for Server {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        let rest = url.split_once("/rest/").map_or(url.as_str(), |(_, r)| r);
        let (endpoint, query) = rest.split_once('?').unwrap_or((rest, ""));
        let params = query.split_once("&f=json").map_or("", |(_, p)| p);
        let asked = format!("{endpoint}{params}");
        self.asked.lock().push(asked.clone());
        if *self.offline.lock() {
            return Err(TransportError::Failed { kind: FailureKind::Connect, detail: None });
        }
        let answers = self.answers.lock();
        let body = answers.get(&asked).or_else(|| answers.get(endpoint)).cloned().unwrap_or_else(|| r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#.into());
        Ok(TransportResponse { status: 200, body: body.into_bytes(), set_cookies: Vec::new() })
    }
    async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(request.url, request.timeout_ms).await
    }
    fn address_changed(&self) {}
    fn network(&self) -> Network {
        Network::Unmetered
    }
}

const SONG: &str = r#"{"id":"s1","title":"One","artist":"A","artistId":"ar1","album":"Al","albumId":"al1","duration":200,"isDir":false}"#;
const SONG2: &str = r#"{"id":"s2","title":"Two","artist":"A","artistId":"ar1","album":"Al","albumId":"al1","duration":180,"starred":"2024-01-01T00:00:00Z","isDir":false}"#;
const PROVIDER_SONG: &str = r#"{"id":"ext-deezer-song-9","title":"Nine","isExternal":true,"duration":100}"#;
const ALBUM: &str = r#"{"id":"al1","name":"Al","artist":"A","artistId":"ar1","year":1999,"songCount":2}"#;
const ALBUM2: &str = r#"{"id":"al2","name":"Bl","artist":"B","year":2005,"songCount":1,"starred":"2024-01-01T00:00:00Z"}"#;
const ARTIST: &str = r#"{"id":"ar1","name":"A"}"#;

/// The reads a screen keeps, each in the order the steps read them.
fn kept_reads() -> Vec<Read> {
    use crate::browse::AlbumSort;
    vec![
        Read::AlbumList { kind: AlbumSort::Newest, size: 20, offset: 0, genre: None },
        Read::AlbumList { kind: AlbumSort::Starred, size: 20, offset: 0, genre: None },
        Read::AlbumList { kind: AlbumSort::ByGenre, size: 20, offset: 0, genre: Some("Rock".into()) },
        Read::AlbumsByYear { from: 2000, to: 2010, size: 20, offset: 0 },
        Read::FavouriteAlbums { size: 20 },
        Read::ArtistIndex,
        Read::AlbumById { id: "al1".into() },
        Read::ArtistById { id: "ar1".into() },
        Read::ArtistAbout { id: "ar1".into() },
        Read::TopSongs { artist: "A".into() },
        Read::PlaylistList,
        Read::PlaylistById { id: "p1".into() },
        Read::StarredItems,
        Read::GenreList,
        Read::RadioList,
        Read::LyricsBySong { song_id: "s1".into() },
        Read::FolderIndex,
        Read::FolderById { id: "d1".into() },
        Read::HomeFeed,
    ]
}

fn asked_reads() -> Vec<Read> {
    use crate::browse::AlbumSort;
    vec![
        Read::AlbumList { kind: AlbumSort::Random, size: 5, offset: 0, genre: None },
        Read::MusicFolders,
        Read::Ping,
        Read::Search { query: "a".into(), songs: 5, albums: 5, artists: 5 },
        Read::SongPage { offset: 0, count: 10 },
        Read::RandomSongs { size: 5, genre: Some("Rock".into()) },
        Read::SongsByGenre { genre: "Rock".into(), count: 5 },
        Read::SimilarSongs { id: "s1".into(), count: 5 },
        Read::SongById { id: "s1".into() },
        Read::AlbumSongs { id: "al1".into() },
        Read::PlaylistSongs { id: "p1".into() },
        Read::NowPlaying { id: "s1".into() },
        Read::ShareLink { id: "s1".into() },
        Read::PullQueue,
    ]
}

/// Which kept reads still show at once (and whether fresh), in [`kept_reads`]' order.
pub(crate) fn kept(t: &mut Transcript, c: &Client, reads: &[Read]) {
    let shown: Vec<String> = reads
        .iter()
        .map(|r| match c.read_stored(r.clone()) {
            Ok(Stored { page: Some(_), fresh, .. }) => if fresh { "F" } else { "s" }.to_string(),
            Ok(_) => "-".into(),
            Err(_) => "!".into(),
        })
        .collect();
    t.line(format!("kept {}", shown.join("")));
}

fn subsonic_answers(s: &Server) {
    s.answer("getAlbumList2", &format!(r#""albumList2":{{"album":[{ALBUM},{ALBUM2}]}}"#));
    s.answer("getArtists", &format!(r#""artists":{{"index":[{{"name":"A","artist":[{ARTIST}]}}]}}"#));
    s.answer("getAlbum", &format!(r#""album":{{"id":"al1","name":"Al","artist":"A","songCount":2,"song":[{SONG},{SONG2}]}}"#));
    s.answer("getArtist", &format!(r#""artist":{{"id":"ar1","name":"A","album":[{ALBUM}]}}"#));
    s.answer("getArtistInfo2", &format!(r#""artistInfo2":{{"biography":"Bio","mediumImageUrl":"http://m","similarArtist":[{ARTIST}]}}"#));
    s.answer("getTopSongs", &format!(r#""topSongs":{{"song":[{SONG}]}}"#));
    s.answer("getPlaylists", r#""playlists":{"playlist":[{"id":"p1","name":"P","songCount":1}]}"#);
    s.answer("getPlaylist", &format!(r#""playlist":{{"id":"p1","name":"P","entry":[{SONG}]}}"#));
    s.answer("getStarred2", &format!(r#""starred2":{{"artist":[{ARTIST}],"album":[{ALBUM2}],"song":[{SONG2}]}}"#));
    s.answer("getGenres", r#""genres":{"genre":[{"value":"Jazz","songCount":1},{"value":"Rock","songCount":9}]}"#);
    s.answer("getInternetRadioStations", r#""internetRadioStations":{"internetRadioStation":[{"id":"r1","name":"R","streamUrl":"http://r"}]}"#);
    s.answer("getLyricsBySongId", r#""lyricsList":{"structuredLyrics":[{"synced":true,"line":[{"start":1000,"value":"Hello"}]}]}"#);
    s.answer("getIndexes", r#""indexes":{"index":[{"name":"A","artist":[{"id":"d1","name":"Folder"}]}]}"#);
    s.answer("getMusicDirectory", &format!(r#""directory":{{"id":"d1","name":"Folder","child":[{{"id":"d2","title":"Sub","isDir":true}},{SONG}]}}"#));
    s.answer("getMusicFolders", r#""musicFolders":{"musicFolder":[{"id":7,"name":"Music"}]}"#);
    s.answer("search3", &format!(r#""searchResult3":{{"artist":[{ARTIST}],"album":[{ALBUM}],"song":[{SONG},{PROVIDER_SONG}]}}"#));
    s.answer("getRandomSongs", &format!(r#""randomSongs":{{"song":[{SONG},{SONG2}]}}"#));
    s.answer("getSongsByGenre", &format!(r#""songsByGenre":{{"song":[{SONG2}]}}"#));
    s.answer("getSimilarSongs2", &format!(r#""similarSongs2":{{"song":[{SONG2}]}}"#));
    s.answer("getSong", &format!(r#""song":{SONG}"#));
    s.answer("createShare", r#""shares":{"share":[{"url":"http://share/1"}]}"#);
    s.answer("getPlayQueue", &format!(r#""playQueue":{{"entry":[{SONG},{SONG2}],"current":"s2","position":1234,"changedBy":"web"}}"#));
}

/// A Subsonic profile over [`Server`], scoped to music folder 7.
fn subsonic() -> (Arc<Client>, Arc<Server>) {
    let core = crate::Core::new(String::new(), "harness".into(), Default::default()).unwrap();
    core.configure(crate::ServerConfig { url: "http://h".into(), user: "u".into(), password: "p".into(), ..Default::default() }).unwrap();
    let server = Arc::new(Server::default());
    subsonic_answers(&server);
    let c = Client::new(core, server.clone(), Default::default());
    c.set_profile(crate::client::NetProfile { url: "http://h".into(), music_folder_ids: vec!["7".into()], ..Default::default() });
    (c, server)
}

fn asked(t: &mut Transcript, s: &Server) {
    for a in s.taken() {
        t.line(format!("asked {a}"));
    }
}

#[test]
fn a_subsonic_session() {
    use crate::client::{Starrable, Write};
    use crate::client::tests::block;
    let mut t = Transcript::new("providers-subsonic");
    let (c, s) = subsonic();
    let reads = kept_reads();

    t.step("first reads: the server's pages, kept");
    for r in &reads {
        each(&mut t, &c, r.clone());
        asked(&mut t, &s);
    }
    t.step("stored");
    for r in &reads {
        t.stored(r, &c.read_stored(r.clone()));
    }
    t.step("read again while fresh: nothing asked");
    for r in &reads {
        each(&mut t, &c, r.clone());
    }
    asked(&mut t, &s);

    t.step("aged, the same answers: the kept pages only");
    age(&c);
    for r in &reads {
        each(&mut t, &c, r.clone());
    }
    asked(&mut t, &s);

    t.step("aged, changed answers: the kept page, then the server's");
    age(&c);
    s.answer("getAlbumList2", &format!(r#""albumList2":{{"album":[{ALBUM2}]}}"#));
    s.answer("getGenres", r#""genres":{"genre":[{"value":"Pop","songCount":3}]}"#);
    for r in &reads {
        each(&mut t, &c, r.clone());
    }
    asked(&mut t, &s);
    subsonic_answers(&s);

    t.step("reads always asked");
    for r in asked_reads() {
        now(&mut t, &c, r.clone());
        t.stored(&r, &c.read_stored(r.clone()));
        each(&mut t, &c, r.clone());
        asked(&mut t, &s);
    }
    t.step("read_now of kept reads keeps nothing new");
    age(&c);
    for r in &reads {
        now(&mut t, &c, r.clone());
    }
    kept(&mut t, &c, &reads);
    asked(&mut t, &s);
    for r in &reads {
        each(&mut t, &c, r.clone());
    }
    s.taken();
    kept(&mut t, &c, &reads);

    t.step("a heart: sent, and what lists hearts is asked again");
    c.core.session.register(vec![Song { id: "s9".into(), title: "Nine known".into(), duration: 90, ..Default::default() }]);
    t.result("star s1", &block(c.star(Starrable::Song, "s1".into(), true, Arc::new(Marked))));
    asked(&mut t, &s);
    kept(&mut t, &c, &reads);
    for r in [Read::StarredItems, Read::AlbumById { id: "al1".into() }, Read::FavouriteAlbums { size: 20 }] {
        each(&mut t, &c, r);
    }
    asked(&mut t, &s);

    t.step("an album unstarred leaves the favourites at once");
    t.result("unstar al2", &block(c.star(Starrable::Album, "al2".into(), false, Arc::new(Marked))));
    asked(&mut t, &s);
    kept(&mut t, &c, &reads);
    for r in [Read::FavouriteAlbums { size: 20 }, Read::AlbumList { kind: crate::browse::AlbumSort::Starred, size: 20, offset: 0, genre: None }, Read::StarredItems] {
        t.stored(&r, &c.read_stored(r.clone()));
    }

    t.step("offline: a heart is queued and the favourites show it");
    *s.offline.lock() = true;
    t.result("star s9", &block(c.star(Starrable::Song, "s9".into(), true, Arc::new(Marked))));
    t.line(format!("pending {}", c.core.pending_list().unwrap().len()));
    for r in [Read::StarredItems, Read::AlbumById { id: "al1".into() }, Read::PlaylistList, Read::HomeFeed] {
        each(&mut t, &c, r.clone());
        t.stored(&r, &c.read_stored(r.clone()));
    }
    t.result("create playlist offline", &block(c.write(Write::CreatePlaylist { name: "x".into(), song_ids: vec!["s1".into()] })));
    asked(&mut t, &s);
    kept(&mut t, &c, &reads);

    t.step("online again: the queue goes first");
    *s.offline.lock() = false;
    t.result("scrobble", &block(c.write(Write::Scrobble { id: "s1".into(), submission: true, time_ms: Some(5) })));
    t.line(format!("pending {}", c.core.pending_list().unwrap().len()));
    asked(&mut t, &s);
    each(&mut t, &c, Read::StarredItems);
    asked(&mut t, &s);

    t.step("each write and what it leaves kept");
    for r in &reads {
        each(&mut t, &c, r.clone());
    }
    s.taken();
    let writes = [
        Write::Star { kind: Starrable::Artist, id: "ar1".into(), on: false },
        Write::CreatePlaylist { name: "New".into(), song_ids: vec!["s1".into(), "s2".into()] },
        Write::AddToPlaylist { id: "p1".into(), song_ids: vec!["s2".into()] },
        Write::RemoveFromPlaylist { id: "p1".into(), index: 0 },
        Write::RenamePlaylist { id: "p1".into(), name: "Road".into() },
        Write::ReplacePlaylist { id: "p1".into(), song_ids: vec!["s2".into(), "s1".into()] },
        Write::DeletePlaylist { id: "p1".into() },
        Write::CreateRadio { name: "R".into(), stream_url: "http://r".into() },
        Write::DeleteRadio { id: "r1".into() },
        Write::Scrobble { id: "s1".into(), submission: false, time_ms: None },
        Write::SaveQueue { ids: vec!["s1".into(), "s2".into()], current: Some("s2".into()), position_ms: 7 },
    ];
    for w in writes {
        t.result(&format!("{w:?}"), &block(c.write(w)));
        asked(&mut t, &s);
        kept(&mut t, &c, &reads);
        for r in &reads {
            each(&mut t, &c, r.clone());
        }
        s.taken();
    }
    t.step("a refused write changes nothing");
    s.refuse("star", 50);
    t.result("star refused", &block(c.star(Starrable::Song, "s2".into(), false, Arc::new(Marked))));
    asked(&mut t, &s);
    kept(&mut t, &c, &reads);
    t.stored(&Read::StarredItems, &c.read_stored(Read::StarredItems));
    subsonic_answers(&s);

    t.step("Home's refresh drops the lists");
    c.drop_cached(crate::browse::home_refresh_drops()).unwrap();
    kept(&mut t, &c, &reads);

    t.step("a page the server no longer has is dropped");
    age(&c);
    s.refuse("getAlbum", 70);
    each(&mut t, &c, Read::AlbumById { id: "al1".into() });
    t.stored(&Read::AlbumById { id: "al1".into() }, &c.read_stored(Read::AlbumById { id: "al1".into() }));
    asked(&mut t, &s);
    s.refuse("getGenres", 50);
    each(&mut t, &c, Read::GenreList);
    t.stored(&Read::GenreList, &c.read_stored(Read::GenreList));
    asked(&mut t, &s);
    *s.offline.lock() = true;
    each(&mut t, &c, Read::RadioList);
    asked(&mut t, &s);
    *s.offline.lock() = false;
    subsonic_answers(&s);

    t.step("a page listing a provider item is never fresh");
    s.answer("getAlbum", &format!(r#""album":{{"id":"al1","name":"Al","song":[{SONG},{PROVIDER_SONG}]}}"#));
    age(&c);
    each(&mut t, &c, Read::AlbumById { id: "al1".into() });
    t.stored(&Read::AlbumById { id: "al1".into() }, &c.read_stored(Read::AlbumById { id: "al1".into() }));
    each(&mut t, &c, Read::AlbumById { id: "al1".into() });
    asked(&mut t, &s);
    subsonic_answers(&s);

    t.step("streams, downloads and covers");
    for id in ["s1", "al 1"] {
        let r = c.resolve(id.into(), false, false);
        t.line(format!("stream {id}: {} {}", r.url.split_once("/rest/").map_or(r.url.as_str(), |x| x.1), r.key));
        let d = c.download_target(id.into(), crate::stream::StreamQuality { bit_rate: 192, format: "mp3".into() });
        t.line(format!("download {id}: {} {}", d.url.split_once("/rest/").map_or(d.url.as_str(), |x| x.1), d.key));
    }
    t.line(format!("cover {}", c.core.cover_address("al 1".into(), 320).split_once("/rest/").unwrap().1));
    t.line(format!("cover prefix {:?}", c.core.cover_prefix().map(|p| p.split_once("/rest/").unwrap().1.to_string())));
    t.line(format!("download covers {:?}", c.core.download_cover_urls(vec!["al 1".into()]).iter().map(|u| u.split_once("/rest/").map_or(u.clone(), |x| x.1.to_string())).collect::<Vec<_>>()));

    t.step("the library walk");
    s.answer("search3&query=&songCount=500&songOffset=500&albumCount=500&albumOffset=500&artistCount=500&artistOffset=500&musicFolderId=7", r#""searchResult3":{}"#);
    match block(c.sync_library()) {
        Ok(n) => t.line(format!("synced {} {} {}", n.artists, n.albums, n.songs)),
        Err(e) => t.line(format!("sync error {e:?}")),
    }
    asked(&mut t, &s);
    t.finish();
}
