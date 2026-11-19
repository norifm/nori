//! Whole flows through the core's public calls against a small Subsonic server kept in memory: log in,
//! fill the offline index, search it, change things while the server is unreachable and see them reach
//! it in order once it is back. The server answers as Navidrome does, from its own library.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use nori_core::client::{login_check, Client, NetProfile, Starrable, Write};
use nori_core::library::StarsShown;
use nori_core::stars::StarMarks;
use nori_core::transport::{Exchange, FailureKind, Transport, TransportError, TransportResponse};
use nori_core::{Core, ServerConfig};
use parking_lot::Mutex;

/// A song of the server's library: id, title, artist, album id, genre.
type Track = (&'static str, &'static str, &'static str, &'static str, &'static str);

const LIBRARY: [Track; 5] = [
    ("s1", "Dogs", "Pink Floyd", "al1", "Rock"),
    ("s2", "Pigs", "Pink Floyd", "al1", "Rock"),
    ("s3", "Sheep", "Pink Floyd", "al1", "Rock"),
    ("s4", "So What", "Miles Davis", "al2", "Jazz"),
    ("s5", "Blue in Green", "Miles Davis", "al2", "Jazz"),
];

/// Songs octo-fiesta offers from a provider: never in the library, downloaded when streamed.
const PROVIDER: [Track; 1] = [("ext-deezer-song-9", "Wish You Were Here", "Pink Floyd", "ext-deezer-album-1", "Rock")];

struct Server {
    reachable: AtomicBool,
    starred: Mutex<BTreeSet<String>>,
    /// Changes the server took, as "endpoint id".
    took: Mutex<Vec<String>>,
    /// The account's playlists: id and song ids.
    playlists: Mutex<Vec<(&'static str, Vec<&'static str>)>>,
}

fn song_json(t: &Track, starred: bool) -> String {
    let (id, title, artist, album, genre) = t;
    let provider = if id.starts_with("ext-") { r#","isExternal":true"# } else { "" };
    let star = if starred { r#","starred":"2026-01-01T00:00:00Z""# } else { "" };
    format!(r#"{{"id":"{id}","title":"{title}","artist":"{artist}","album":"{album}","albumId":"{album}","genre":"{genre}","duration":300,"isDir":false{provider}{star}}}"#)
}

fn ok(body: &str) -> Vec<u8> {
    format!(r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1"{}{body}}}}}"#, if body.is_empty() { "" } else { "," }).into_bytes()
}

/// `%XX` escapes decoded.
fn decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(x)) => {
                out.push(x);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

impl Server {
    fn new() -> Arc<Server> {
        Arc::new(Server { reachable: AtomicBool::new(true), starred: Mutex::new(BTreeSet::new()), took: Mutex::new(Vec::new()), playlists: Mutex::new(Vec::new()) })
    }

    fn reachable(&self, on: bool) {
        self.reachable.store(on, Ordering::SeqCst);
    }

    fn took(&self) -> Vec<String> {
        self.took.lock().clone()
    }

    fn songs(&self, of: impl Fn(&Track) -> bool) -> String {
        let starred = self.starred.lock();
        LIBRARY.iter().chain(&PROVIDER).filter(|t| of(t)).map(|t| song_json(t, starred.contains(t.0))).collect::<Vec<_>>().join(",")
    }

    fn answer(&self, endpoint: &str, p: &HashMap<String, String>) -> Vec<u8> {
        let num = |k: &str| p.get(k).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let id = p.get("id").cloned().unwrap_or_default();
        match endpoint {
            "ping" => ok(""),
            "search3" => {
                let query = p.get("query").cloned().unwrap_or_default().to_lowercase();
                let hits: Vec<&Track> = LIBRARY.iter().filter(|t| t.1.to_lowercase().contains(&query)).collect();
                let page: Vec<String> = hits.iter().skip(num("songOffset")).take(num("songCount")).map(|t| song_json(t, false)).collect();
                ok(&format!(r#""searchResult3":{{"song":[{}]}}"#, page.join(",")))
            }
            "getSong" => ok(&format!(r#""song":{}"#, self.songs(|t| t.0 == id))),
            "getSimilarSongs2" => {
                let genre = LIBRARY.iter().chain(&PROVIDER).find(|t| t.0 == id).map_or("", |t| t.4);
                ok(&format!(r#""similarSongs2":{{"song":[{}]}}"#, self.songs(|t| t.4 == genre && t.0 != id)))
            }
            "star" | "unstar" | "scrobble" => {
                self.took.lock().push(format!("{endpoint} {id}"));
                match endpoint {
                    "star" => self.starred.lock().insert(id),
                    "unstar" => self.starred.lock().remove(&id),
                    _ => true,
                };
                ok("")
            }
            "getPlaylists" => {
                let listed: Vec<String> = self.playlists.lock().iter().map(|(id, songs)| format!(r#"{{"id":"{id}","name":"{id}","songCount":{}}}"#, songs.len())).collect();
                ok(&format!(r#""playlists":{{"playlist":[{}]}}"#, listed.join(",")))
            }
            "getPlaylist" => {
                let held = self.playlists.lock().iter().find(|p| p.0 == id).map(|p| p.1.clone()).unwrap_or_default();
                let entries: Vec<String> = held.iter().map(|s| self.songs(|t| t.0 == *s)).collect();
                ok(&format!(r#""playlist":{{"id":"{id}","name":"{id}","entry":[{}]}}"#, entries.join(",")))
            }
            "updatePlaylist" => {
                let id = p.get("playlistId").cloned().unwrap_or_default();
                let added = p.get("songIdToAdd").cloned().unwrap_or_default();
                if let Some(pl) = self.playlists.lock().iter_mut().find(|pl| pl.0 == id) {
                    pl.1.extend(LIBRARY.iter().map(|t| t.0).find(|s| *s == added));
                }
                ok("")
            }
            "deletePlaylist" => {
                self.playlists.lock().retain(|pl| pl.0 != id);
                ok("")
            }
            "getStarred2" => {
                let starred = self.starred.lock().clone();
                ok(&format!(r#""starred2":{{"song":[{}]}}"#, self.songs(|t| starred.contains(t.0))))
            }
            _ => format!(r#"{{"subsonic-response":{{"status":"failed","error":{{"code":0,"message":"no {endpoint}"}}}}}}"#).into_bytes(),
        }
    }
}

#[async_trait::async_trait]
impl Transport for Server {
    async fn get(&self, url: String, _timeout_ms: u32) -> Result<TransportResponse, TransportError> {
        if !self.reachable.load(Ordering::SeqCst) {
            return Err(TransportError::Failed { kind: FailureKind::Connect, detail: Some("unreachable".into()) });
        }
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let endpoint = path.rsplit('/').next().unwrap_or_default();
        let params = query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), decode(v))).collect();
        Ok(TransportResponse { status: 200, body: self.answer(endpoint, &params), headers: Vec::new() })
    }

    async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(request.url, request.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> nori_core::transport::Network {
        nori_core::transport::Network::Unmetered
    }
}

/// Polls `f` to completion; the server answers at once.
fn block<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

struct Unseen;

impl StarsShown for Unseen {
    fn marks(&self, _: StarMarks) {}
}

fn logged_in(server: &Arc<Server>) -> (Arc<Core>, Arc<Client>) {
    let config = ServerConfig { url: "http://nas:4533".into(), user: "u".into(), password: "p".into(), ..Default::default() };
    assert!(!block(login_check(server.clone(), config.clone(), String::new())).unwrap(), "token auth");
    let core = Core::new(String::new(), "scenario".into(), Default::default()).unwrap();
    core.configure(config).unwrap();
    let client = Client::new(core.clone(), server.clone(), Default::default());
    client.set_profile(NetProfile { url: "http://nas:4533".into(), ..Default::default() });
    (core, client)
}

#[test]
fn offline_changes_reach_server() {
    let server = Server::new();
    let (core, client) = logged_in(&server);
    block(client.sync_library()).unwrap();
    assert_eq!(core.index_size().unwrap().songs, 5);
    assert_eq!(core.local_search("blue gre".into(), 5).unwrap().songs[0].id, "s5");

    server.reachable(false);
    block(client.star(Starrable::Song, "s1".into(), true, Arc::new(Unseen))).unwrap();
    block(client.write(Write::Scrobble { id: "s2".into(), submission: true, time_ms: Some(1) })).unwrap();
    block(client.star(Starrable::Song, "s1".into(), false, Arc::new(Unseen))).unwrap();
    assert!(server.took().is_empty());

    server.reachable(true);
    block(client.star(Starrable::Song, "s3".into(), true, Arc::new(Unseen))).unwrap();
    block(client.flush_pending()).unwrap();
    assert_eq!(server.took(), ["star s1", "scrobble s2", "unstar s1", "star s3"], "in order, each once");
    assert_eq!(server.starred.lock().iter().collect::<Vec<_>>(), ["s3"]);
}

/// A playlist downloaded is kept as the server changes it: songs added are queued, and a song leaving
/// goes only when keeping it brought it; a playlist deleted lets its songs go.
#[test]
fn a_kept_playlist_follows_the_server() {
    let server = Server::new();
    server.playlists.lock().push(("p", vec!["s1", "s2", "ext-deezer-song-9"]));
    let (core, client) = logged_in(&server);
    let change = |c: nori_core::transfers::KeptChange| (c.queued.fresh, c.gone);
    let settle = |ids: &[&str]| core.download_settle(ids.iter().map(|s| s.to_string()).collect(), vec![true; ids.len()]).unwrap();
    assert_eq!(block(client.kept_playlists_sync(nori_core::transfers::KeptCheck::Opened)).unwrap(), Default::default(), "nothing kept");

    let songs = block(client.playlist_songs_of("p".into())).unwrap();
    assert_eq!(change(core.download_playlist("p".into(), songs).unwrap()), (vec!["s1".into(), "s2".into(), "ext-deezer-song-9".into()], vec![]), "the provider's too");
    settle(&["s1", "s2", "ext-deezer-song-9"]);
    let own = block(client.playlist_songs_of("p".into())).unwrap().into_iter().find(|s| s.id == "s1").map(|s| nori_core::Song { id: "s3".into(), ..s }).unwrap();
    core.download_queue(vec![own]).unwrap();
    settle(&["s3"]);

    server.playlists.lock()[0].1 = vec!["s2", "s3", "s5"];
    assert_eq!(change(block(client.kept_playlists_sync(nori_core::transfers::KeptCheck::Opened)).unwrap()), (vec!["s5".into()], vec!["ext-deezer-song-9".into(), "s1".into()]));
    settle(&["s5"]);
    server.playlists.lock()[0].1 = vec!["s5"];
    assert_eq!(change(block(client.kept_playlists_sync(nori_core::transfers::KeptCheck::Opened)).unwrap()), (vec![], vec!["s2".into()]), "s3 was downloaded by itself");
    server.playlists.lock().clear();
    assert_eq!(change(block(client.kept_playlists_sync(nori_core::transfers::KeptCheck::Opened)).unwrap()), (vec![], vec!["s5".into()]));
    assert_eq!(block(client.kept_playlists_sync(nori_core::transfers::KeptCheck::Opened)).unwrap(), Default::default(), "let go");
}

/// A kept playlist this device edits follows at once; coming back in sight looks at the server again
/// only after half an hour.
#[test]
fn a_kept_playlist_edited_here_follows_at_once() {
    use nori_core::transfers::KeptCheck;
    let server = Server::new();
    server.playlists.lock().push(("p", vec!["s1"]));
    let (core, client) = logged_in(&server);
    let change = |c: nori_core::transfers::KeptChange| (c.queued.fresh, c.gone);
    let settle = |ids: &[&str]| core.download_settle(ids.iter().map(|s| s.to_string()).collect(), vec![true; ids.len()]).unwrap();
    core.download_playlist("p".into(), block(client.playlist_songs_of("p".into())).unwrap()).unwrap();
    settle(&["s1"]);
    assert_eq!(block(client.kept_playlists_sync(KeptCheck::Opened)).unwrap(), Default::default());

    server.playlists.lock()[0].1.push("s2");
    assert_eq!(block(client.kept_playlists_sync(KeptCheck::Shown)).unwrap(), Default::default(), "looked at a moment ago");
    let add = Write::AddToPlaylist { id: "p".into(), song_ids: vec!["s3".into()] };
    block(client.write(add.clone())).unwrap();
    assert_eq!(change(block(client.kept_playlist_written(add)).unwrap()), (vec!["s2".into(), "s3".into()], vec![]));
    settle(&["s2", "s3"]);
    let rename = Write::RenamePlaylist { id: "p".into(), name: "q".into() };
    assert_eq!(block(client.kept_playlist_written(rename)).unwrap(), Default::default(), "the songs stay");

    let delete = Write::DeletePlaylist { id: "p".into() };
    block(client.write(delete.clone())).unwrap();
    assert_eq!(change(block(client.kept_playlist_written(delete)).unwrap()), (vec![], vec!["s1".into(), "s2".into(), "s3".into()]));
}

#[test]
fn a_radio_plays_the_library_only() {
    let server = Server::new();
    let (_, client) = logged_in(&server);
    let radio: Vec<String> = block(client.radio("s1".into())).unwrap().into_iter().map(|s| s.id).collect();
    assert_eq!(radio, ["s1", "s2", "s3"], "the seed, then its like, no provider song");
}

#[test]
fn favourites_mix_is_the_starred_songs() {
    let server = Server::new();
    server.starred.lock().extend(["s2".to_string(), "s4".to_string(), "ext-deezer-song-9".to_string()]);
    let (_, client) = logged_in(&server);
    let ids: Vec<String> = block(client.mix_songs("favourites".into())).unwrap().into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["s2", "s4"], "the starred songs of the library, no provider song");
}
