//! The jam's share against an in-memory Navidrome, alone or behind octo-fiesta, on a simulated clock.

use std::collections::{HashSet, VecDeque};

use super::*;
use crate::client::NetProfile;
use crate::transport::{Exchange, Transport, TransportError, TransportResponse};
use crate::{Core, ServerConfig};

/// The address the host's profile reaches the server at; the server names itself by another.
const PROFILE: &str = "http://music.example";
const INTERNAL: &str = "http://internal:4533";

/// Navidrome as the share calls see it; behind octo-fiesta when `parts_as_whole`.
struct Navi {
    kind: &'static str,
    version: &'static str,
    sharing: bool,
    /// Answers a request for part of a song with that part labeled as the whole file (octo-fiesta).
    parts_as_whole: bool,
    /// What each try to add a provider's song answers, in order: the library song it became, or an
    /// error code.
    provider: Mutex<VecDeque<Result<&'static str, i32>>>,
    /// Albums with no cover.
    bare: HashSet<&'static str>,
    state: Mutex<NaviState>,
    /// Each request: what it asked for and the time it was allowed.
    asked: Mutex<Vec<(String, u32)>>,
}

#[derive(Default)]
struct NaviState {
    /// Playlists: id, name, song ids.
    playlists: Vec<(String, String, Vec<String>)>,
    /// Shares: id, playlist, expiry, and how often it was extended (its songs' tokens hold the expiry).
    shares: Vec<(String, String, i64, u32)>,
    made: u32,
    /// Provider songs fetched: the `ext-` id and the library song.
    fetched: Vec<(String, String)>,
}

impl Default for Navi {
    fn default() -> Self {
        Navi {
            kind: "navidrome",
            version: "0.64.2 (8a8c0c0)",
            sharing: true,
            parts_as_whole: false,
            provider: Default::default(),
            bare: HashSet::new(),
            state: Default::default(),
            asked: Default::default(),
        }
    }
}

fn ok(fields: &str, kind: &str, version: &str) -> String {
    let comma = if fields.is_empty() { "" } else { "," };
    format!(r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","type":"{kind}","serverVersion":"{version}"{comma}{fields}}}}}"#)
}

fn failed(code: i32) -> String {
    format!(r#"{{"subsonic-response":{{"status":"failed","version":"1.16.1","error":{{"code":{code},"message":"no"}}}}}}"#)
}

fn song_json(id: &str) -> String {
    format!(r#"{{"id":"{id}","title":"{id}","albumId":"al-{id}","isDir":false}}"#)
}

impl Navi {
    fn answer(&self, url: &str) -> (u16, String) {
        let path = url.strip_prefix(INTERNAL).or_else(|| url.strip_prefix(PROFILE)).expect("one of the server's addresses");
        if let Some(rest) = path.strip_prefix("/share/") {
            let id = rest.strip_suffix("/m3u").expect("a share's list");
            let st = self.state.lock();
            let Some((_, playlist, _, extended)) = st.shares.iter().find(|s| s.0 == id) else { return (404, "Share not found".into()) };
            let songs = &st.playlists.iter().find(|p| &p.0 == playlist).expect("its playlist").2;
            let lines: String = songs.iter().map(|s| format!("#EXTINF:60,A - {s}\n{INTERNAL}/share/s/{s}.{id}.{extended}\n")).collect();
            return (200, format!("#EXTM3U\n{lines}"));
        }
        let (endpoint, query) = path.strip_prefix("/rest/").expect("a Subsonic call").split_once('?').expect("signed");
        let params: Vec<(&str, &str)> = query.split('&').filter_map(|p| p.split_once('=')).collect();
        let param = |k: &str| params.iter().find(|p| p.0 == k).map(|p| p.1.to_string()).unwrap_or_default();
        let ok = |fields: &str| (200, ok(fields, self.kind, self.version));
        let mut st = self.state.lock();
        match endpoint {
            "ping" => ok(""),
            "createPlaylist" => {
                st.made += 1;
                let id = format!("pl{}", st.made);
                st.playlists.push((id.clone(), param("name"), Vec::new()));
                ok(&format!(r#""playlist":{{"id":"{id}","name":"{}","songCount":0}}"#, param("name")))
            }
            "updatePlaylist" => {
                let playlist = param("playlistId");
                let added = param("songIdToAdd");
                let song = if added.starts_with("ext-") {
                    match self.provider.lock().pop_front().expect("an answer for the provider's song") {
                        Ok(library) => {
                            st.fetched.push((added.clone(), library.to_string()));
                            Some(library.to_string())
                        }
                        Err(code) => return (200, failed(code)),
                    }
                } else {
                    Some(added).filter(|a| !a.is_empty())
                };
                let Some(list) = st.playlists.iter_mut().find(|p| p.0 == playlist) else { return (200, failed(70)) };
                match song {
                    Some(s) => list.2.push(s),
                    None => {
                        list.2.remove(param("songIndexToRemove").parse().expect("an index"));
                    }
                }
                ok("")
            }
            "getPlaylist" => match st.playlists.iter().find(|p| p.0 == param("id")) {
                Some((id, name, songs)) => {
                    let entry: Vec<String> = songs.iter().map(|s| song_json(s)).collect();
                    ok(&format!(r#""playlist":{{"id":"{id}","name":"{name}","entry":[{}]}}"#, entry.join(",")))
                }
                None => (200, failed(70)),
            },
            "deletePlaylist" => {
                let before = st.playlists.len();
                st.playlists.retain(|p| p.0 != param("id"));
                if st.playlists.len() == before { (200, failed(70)) } else { ok("") }
            }
            "createShare" | "updateShare" | "deleteShare" if !self.sharing => (501, "This endpoint is not implemented".into()),
            "createShare" => {
                st.made += 1;
                let id = format!("sh{}", st.made);
                st.shares.push((id.clone(), param("id"), param("expires").parse().expect("an expiry"), 0));
                ok(&format!(r#""shares":{{"share":[{{"id":"{id}","url":"{INTERNAL}/share/{id}","description":"{}"}}]}}"#, param("description")))
            }
            "updateShare" => {
                let share = st.shares.iter_mut().find(|s| s.0 == param("id")).expect("the share");
                share.2 = param("expires").parse().expect("an expiry");
                share.3 += 1;
                ok("")
            }
            "deleteShare" => {
                let before = st.shares.len();
                st.shares.retain(|s| s.0 != param("id"));
                if st.shares.len() == before { (200, failed(70)) } else { ok("") }
            }
            "getSong" => {
                let id = param("id");
                let library = st.fetched.iter().find(|f| f.0 == id).map_or(id, |f| f.1.clone());
                ok(&format!(r#""song":{}"#, song_json(&library)))
            }
            "getAlbumInfo2" => {
                let id = param("id");
                if self.bare.contains(id.as_str()) {
                    return ok(r#""albumInfo":{}"#);
                }
                ok(&format!(r#""albumInfo":{{"largeImageUrl":"{INTERNAL}/share/img/{id}?size=1200","smallImageUrl":"{INTERNAL}/share/img/{id}?size=300"}}"#))
            }
            other => panic!("not asked of a jam's server: {other}"),
        }
    }
}

#[async_trait::async_trait]
impl Transport for Navi {
    async fn get(&self, url: String, timeout_ms: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().push((url.clone(), timeout_ms));
        let (status, body) = self.answer(&url);
        Ok(TransportResponse { status, body: body.into_bytes(), headers: Vec::new() })
    }

    /// A song's address asked for its first byte.
    async fn send(&self, r: Exchange) -> Result<TransportResponse, TransportError> {
        self.asked.lock().push((r.url.clone(), r.timeout_ms));
        assert!(r.url.starts_with(&format!("{PROFILE}/share/s/")) && r.headers.get("Range").map(String::as_str) == Some("bytes=0-0"), "{r:?}");
        Ok(TransportResponse { status: if self.parts_as_whole { 200 } else { 206 }, body: vec![0], headers: Vec::new() })
    }

    fn address_changed(&self) {}

    fn network(&self) -> crate::transport::Network {
        crate::transport::Network::Unmetered
    }
}

#[derive(Default)]
struct Heard(Mutex<Vec<JamShareEvent>>);

impl JamShareHeard for Heard {
    fn heard(&self, event: JamShareEvent) {
        self.0.lock().push(event);
    }
}

impl Heard {
    fn take(&self) -> Vec<JamShareEvent> {
        std::mem::take(&mut self.0.lock())
    }
}

struct Rig {
    navi: Arc<Navi>,
    client: Arc<Client>,
    tasks: Arc<nori_tasks::Native>,
    heard: Arc<Heard>,
}

impl Rig {
    fn new(navi: Navi) -> Rig {
        let core = Core::new(String::new(), "t".into(), Arc::default()).unwrap();
        core.configure(ServerConfig { url: PROFILE.into(), user: "u".into(), password: "p".into(), ..Default::default() }).unwrap();
        let navi = Arc::new(navi);
        let client = Client::new(core, navi.clone(), Default::default());
        client.set_profile(NetProfile { url: PROFILE.into(), ..Default::default() });
        Rig { navi, client, tasks: Arc::new(nori_tasks::Native::simulated("jam-share", 0).unwrap()), heard: Arc::default() }
    }

    fn start(&self) -> Result<JamShare, ShareError> {
        let share = self.tasks.run("the share", self.client.start_jam_share(self.tasks.clone(), self.heard.clone()));
        self.tasks.settle();
        share
    }

    /// The calls asked since the last look, each as its endpoint (or public path) and its parameters
    /// other than the signature's.
    fn asked(&self) -> Vec<String> {
        let signed = ["u", "t", "s", "v", "c", "f"];
        std::mem::take(&mut *self.navi.asked.lock())
            .into_iter()
            .map(|(url, _)| {
                let path = url.strip_prefix(PROFILE).unwrap_or(&url).to_string();
                let Some((endpoint, query)) = path.strip_prefix("/rest/").and_then(|p| p.split_once('?')) else { return path };
                let params: Vec<&str> = query.split('&').filter(|p| p.split_once('=').is_none_or(|(k, _)| !signed.contains(&k))).collect();
                format!("{endpoint} {}", params.join(" "))
            })
            .collect()
    }

    fn timeouts(&self) -> Vec<u32> {
        self.navi.asked.lock().iter().map(|a| a.1).collect()
    }

    fn playlists(&self) -> Vec<(String, Vec<String>)> {
        self.navi.state.lock().playlists.iter().map(|p| (p.1.clone(), p.2.clone())).collect()
    }

    fn expiry(&self) -> i64 {
        self.navi.state.lock().shares[0].2
    }
}

fn song(id: &str) -> Song {
    Song { id: id.into(), album_id: Some(format!("al-{id}")), ..Default::default() }
}

fn provider_song(id: &str) -> Song {
    Song { id: id.into(), is_external: true, ..Default::default() }
}

fn ready(address: &str, cover: Option<&str>) -> SharedSong {
    SharedSong::Ready { address: format!("{PROFILE}{address}"), cover: cover.map(|c| format!("{PROFILE}{c}")) }
}

/// The expiry a share made or extended now gets, give or take the test's own time.
fn about_now_plus(lasts: Duration, expires: i64) -> bool {
    (expires - crate::db::now_ms() - lasts.as_millis() as i64).abs() < 60_000
}

#[test]
fn a_jam_starts_a_hidden_playlist_and_one_share_of_it() {
    let rig = Rig::new(Navi::default());
    let _share = rig.start().unwrap();
    let asked = rig.asked();
    assert_eq!(asked[0], "ping ");
    let name = asked[1].strip_prefix("createPlaylist name=").expect("a playlist made");
    assert!(is_jam_playlist(name) && name.len() == "nori-jam-".len() + 8, "{name}");
    assert!(asked[2].starts_with(&format!("createShare id=pl1 description={name} expires=")), "{asked:?}");
    assert_eq!(asked.len(), 3);
    assert!(about_now_plus(LASTS, rig.expiry()), "lasts a few hours");
    assert_eq!(rig.tasks.waits(), [EXTEND_EVERY], "extended while it runs");
    assert!(rig.heard.take().is_empty());
}

#[test]
fn a_song_is_told_at_the_profiles_address_with_its_cover() {
    let rig = Rig::new(Navi { bare: ["al-b"].into(), ..Navi::default() });
    let share = rig.start().unwrap();
    rig.asked();
    share.add(7, song("a"));
    rig.tasks.settle();
    assert_eq!(
        rig.heard.take(),
        [
            JamShareEvent::Fetch { how: GuestFetch::FromAnyByte },
            JamShareEvent::Song { entry: 7, state: ready("/share/s/a.sh2.0", Some("/share/img/al-a?size=1200")) },
        ]
    );
    assert_eq!(rig.asked(), ["updatePlaylist playlistId=pl1 songIdToAdd=a", "getPlaylist id=pl1", "/share/sh2/m3u", "/share/s/a.sh2.0", "getAlbumInfo2 id=al-a"]);

    // A song whose album has no public cover is told so; the same album is asked once.
    share.add(8, song("b"));
    share.add(9, song("a"));
    rig.tasks.settle();
    let mut heard = rig.heard.take();
    heard.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(
        heard,
        [
            JamShareEvent::Song { entry: 8, state: ready("/share/s/b.sh2.0", None) },
            JamShareEvent::Song { entry: 9, state: ready("/share/s/a.sh2.0", Some("/share/img/al-a?size=1200")) },
        ]
    );
    assert_eq!(rig.asked().iter().filter(|a| a.starts_with("getAlbumInfo2")).count(), 1, "al-b only");
}

#[test]
fn behind_octo_fiesta_guests_fetch_whole_songs() {
    for (parts_as_whole, how) in [(false, GuestFetch::FromAnyByte), (true, GuestFetch::WholeFromStart)] {
        let rig = Rig::new(Navi { parts_as_whole, ..Navi::default() });
        let share = rig.start().unwrap();
        share.add(1, song("a"));
        share.add(2, song("b"));
        rig.tasks.settle();
        let fetch: Vec<JamShareEvent> = rig.heard.take().into_iter().filter(|e| matches!(e, JamShareEvent::Fetch { .. })).collect();
        assert_eq!(fetch, [JamShareEvent::Fetch { how }], "learned once");
    }
}

#[test]
fn a_provider_song_is_prepared_then_shared_as_its_library_song() {
    let rig = Rig::new(Navi { provider: Mutex::new([Ok("lib-x")].into()), ..Navi::default() });
    let share = rig.start().unwrap();
    rig.asked();
    share.add(3, provider_song("ext-ytm-song-x"));
    rig.tasks.settle();
    let heard: Vec<JamShareEvent> = rig.heard.take().into_iter().filter(|e| matches!(e, JamShareEvent::Song { .. })).collect();
    assert_eq!(
        heard,
        [
            JamShareEvent::Song { entry: 3, state: SharedSong::Preparing },
            JamShareEvent::Song { entry: 3, state: ready("/share/s/lib-x.sh2.0", Some("/share/img/al-lib-x?size=1200")) },
        ]
    );
    assert_eq!(rig.timeouts()[0], PREPARE_MS, "the server fetches it first");
    assert_eq!(rig.asked()[..2], ["updatePlaylist playlistId=pl1 songIdToAdd=ext-ytm-song-x", "getSong id=ext-ytm-song-x"]);
}

#[test]
fn a_provider_song_whose_scan_was_missed_is_added_once_more() {
    // Missed once, then there; missed twice; the provider failed.
    for (answers, end) in [
        (vec![Err(70), Ok("lib-x")], None),
        (vec![Err(70), Err(70)], Some(Skip::NotInLibrary)),
        (vec![Err(10)], Some(Skip::ProviderFailed)),
    ] {
        let tries = answers.len();
        let rig = Rig::new(Navi { provider: Mutex::new(answers.into()), ..Navi::default() });
        let share = rig.start().unwrap();
        rig.asked();
        share.add(1, provider_song("ext-ytm-song-x"));
        rig.tasks.settle();
        let mut heard = rig.heard.take();
        assert_eq!(heard[0], JamShareEvent::Song { entry: 1, state: SharedSong::Preparing });
        if tries == 2 {
            assert_eq!(heard.len(), 1, "waits for the scan");
            assert_eq!(rig.tasks.waits(), [SCAN_MISSED_WAIT, EXTEND_EVERY], "octo-fiesta's next scan");
            rig.tasks.pass(SCAN_MISSED_WAIT);
            heard.extend(rig.heard.take());
        }
        let adds = rig.asked().iter().filter(|a| a.starts_with("updatePlaylist")).count();
        assert_eq!(adds, tries, "{end:?}");
        let last = heard.into_iter().rfind(|e| matches!(e, JamShareEvent::Song { .. }));
        match end {
            Some(why) => assert_eq!(last, Some(JamShareEvent::Song { entry: 1, state: SharedSong::Skipped { why } })),
            None => assert!(matches!(last, Some(JamShareEvent::Song { state: SharedSong::Ready { .. }, .. })), "{last:?}"),
        }
        assert_eq!(rig.tasks.waits().len(), 1, "only the extension waits");
    }
}

#[test]
fn the_share_is_extended_while_the_jam_runs_and_addresses_follow() {
    let rig = Rig::new(Navi::default());
    let share = rig.start().unwrap();
    share.add(1, song("a"));
    rig.tasks.settle();
    let first = rig.expiry();
    rig.heard.take();
    rig.asked();
    rig.tasks.pass(EXTEND_EVERY);
    let asked = rig.asked();
    assert!(asked[0].starts_with("updateShare id=sh2 description=nori-jam-") && asked[0].contains(" expires="), "{asked:?}");
    let now = rig.expiry();
    assert!(now >= first && about_now_plus(LASTS, now));
    // An address carries the share's expiry as it was: the new one is told.
    assert_eq!(rig.heard.take(), [JamShareEvent::Song { entry: 1, state: ready("/share/s/a.sh2.1", Some("/share/img/al-a?size=1200")) }]);
    assert_eq!(rig.tasks.waits(), [EXTEND_EVERY]);
}

#[test]
fn a_song_leaving_the_queue_leaves_the_share() {
    let rig = Rig::new(Navi { provider: Mutex::new([Ok("lib-x")].into()), ..Navi::default() });
    let share = rig.start().unwrap();
    share.add(1, song("a"));
    share.add(2, song("b"));
    share.add(3, song("a"));
    rig.tasks.settle();
    share.remove(3);
    share.remove(2);
    rig.tasks.settle();
    assert_eq!(rig.playlists()[0].1, ["a"], "one copy of a stays for entry 1");

    // Taken out while it was prepared: it leaves once in the playlist.
    rig.heard.take();
    share.add(4, provider_song("ext-ytm-song-x"));
    share.remove(4);
    rig.tasks.settle();
    assert_eq!(rig.playlists()[0].1, ["a"]);
    assert_eq!(rig.heard.take(), [], "nothing told of an entry gone");
}

#[test]
fn the_end_deletes_the_share_and_the_playlist() {
    let rig = Rig::new(Navi::default());
    let share = rig.start().unwrap();
    share.add(1, song("a"));
    rig.tasks.settle();
    rig.asked();
    rig.tasks.run("the end", share.end()).unwrap();
    assert_eq!(rig.asked(), ["deleteShare id=sh2", "deletePlaylist id=pl1"]);
    assert!(rig.playlists().is_empty() && rig.navi.state.lock().shares.is_empty());
    rig.tasks.settle();
    assert!(rig.tasks.waits().is_empty(), "nothing extends it");

    // Nothing left to clean up.
    rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
    assert!(rig.asked().is_empty());
}

#[test]
fn a_jam_the_app_never_ended_is_cleaned_up_when_the_profile_opens() {
    let rig = Rig::new(Navi::default());
    drop(rig.start().unwrap());
    rig.tasks.settle();
    assert!(rig.tasks.waits().is_empty(), "nothing runs on");
    rig.asked();
    rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
    assert_eq!(rig.asked(), ["deleteShare id=sh2", "deletePlaylist id=pl1"]);
    assert!(rig.playlists().is_empty());
    rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
    assert!(rig.asked().is_empty(), "once");

    // Gone already (deleted elsewhere): forgotten all the same.
    let share = rig.start().unwrap();
    drop(share);
    rig.navi.state.lock().playlists.clear();
    rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
    rig.asked();
    rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
    assert!(rig.asked().is_empty());
}

#[test]
fn a_server_that_does_not_share_says_why() {
    for (kind, version, sharing, why) in [
        ("navidrome", "0.58.5", false, "Off { off_by_default: true }"),
        ("navidrome", "0.64.2 (8a8c0c0)", false, "Off { off_by_default: false }"),
        ("gonic", "0.16.4", true, "NotNavidrome"),
    ] {
        let rig = Rig::new(Navi { kind, version, sharing, ..Navi::default() });
        let Err(e) = rig.start() else { panic!("{kind} {version} shared") };
        assert_eq!(format!("{e:?}"), why);
        assert!(rig.playlists().is_empty(), "no playlist left behind");
        rig.asked();
        rig.tasks.run("the clean-up", rig.client.clean_jam_leftovers()).unwrap();
        assert!(rig.asked().is_empty(), "nothing to clean up");
    }
}
