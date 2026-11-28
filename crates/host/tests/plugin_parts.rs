//! A page a plugin provider reads in many requests, one after another (a long playlist's pages), shows
//! what came while the rest are asked, and its saved copy in the next session.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::library::PageShown;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, Exchange, Network, Transport, TransportError, TransportResponse};
use nori_core::Core;
use nori_plugins::testing::{fixture, Quiet, FIXTURE};

/// Connections that hold each request until the test lets it go.
struct Held {
    asked: Mutex<Sender<String>>,
    go: Mutex<Receiver<()>>,
}

#[async_trait::async_trait]
impl Transport for Held {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().unwrap().send(url).unwrap();
        let _ = self.go.lock().unwrap().recv_timeout(Duration::from_secs(30));
        Ok(TransportResponse { status: 200, body: Vec::new(), headers: Vec::new() })
    }

    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}

/// The songs of each playlist page shown, and whether more were on their way.
struct Shown(Mutex<Sender<(Vec<String>, bool)>>);

impl PageShown for Shown {
    fn show(&self, page: Page) {
        if let Page::PlaylistPage { v } = page {
            self.0.lock().unwrap().send((v.songs.into_iter().map(|s| s.title).collect(), v.filling)).unwrap();
        }
    }
}

/// A client of a profile played from the fixture plugin, its database in `dir`, over connections the test
/// lets go one by one: the URLs asked, and where to let each go.
fn open(dir: &nori_testdir::TempDir) -> (Arc<Client>, Receiver<String>, Sender<()>) {
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(Quiet));
    plugins.install(&fixture("")).unwrap();
    plugins.turn(FIXTURE.into(), true);
    let kinds = nori_core::providers::Kinds::with_plugins(plugins);
    let profile = SavedServer { id: "p".into(), kind: ServerKind::Plugin { id: FIXTURE.into() }, ..Default::default() };
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    core.use_kinds(kinds.clone());
    core.configure_profile(profile).unwrap();
    let ((asked, requests), (go, held)) = (channel(), channel());
    let net = Arc::new(Held { asked: Mutex::new(asked), go: Mutex::new(held) });
    kinds.set_transport(net.clone());
    (Client::new(core, net, Default::default()), requests, go)
}

const WAIT: Duration = Duration::from_secs(10);

fn titles(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A long playlist shows what came before each next page is asked, marked as filling, then whole; a queue
/// started from its first page takes the rest from that read, which is not asked again.
#[test]
fn a_long_playlist_fills_in_and_its_queue_takes_the_rest() {
    let dir = nori_testdir::TempDir::new("plugin-parts");
    let (client, requests, go) = open(&dir);
    let (shown, pages) = channel();
    let shown = Arc::new(Shown(Mutex::new(shown)));
    let read = Read::PlaylistById { id: "test-long".into() };
    let reading = {
        let (client, read, shown) = (client.clone(), read.clone(), shown.clone());
        std::thread::spawn(move || block_on(client.read_cached(read, shown)))
    };
    assert_eq!(pages.recv_timeout(WAIT).expect("the first page, shown before the second came"), (titles(&["first"]), true));
    assert_eq!(requests.recv_timeout(WAIT).unwrap(), "https://allowed.example/second");

    let ids = vec![nori_core::ItemKey::own("test-first")];
    let started = client.session().set(ids, Some(0), false, Some(nori_core::PageOrigin::new(nori_core::OriginKind::Playlist, "test-long")));
    assert!(started.follows, "played from the first page");
    let following = {
        let client = client.clone();
        std::thread::spawn(move || block_on(client.queue_follow()))
    };

    go.send(()).unwrap();
    assert_eq!(pages.recv_timeout(WAIT).unwrap(), (titles(&["first", "second"]), true));
    assert_eq!(requests.recv_timeout(WAIT).unwrap(), "https://allowed.example/third");
    go.send(()).unwrap();
    reading.join().unwrap().unwrap();
    assert_eq!(pages.recv_timeout(WAIT).unwrap(), (titles(&["first", "second", "third"]), false));
    let followed = following.join().unwrap().expect("the rest joined the queue");
    assert_eq!(followed.edit.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["second", "third"]);
    assert_eq!(client.session().playlist(|p| p.ids().len()), 3);
    assert!(requests.try_recv().is_err(), "the page was not asked again for the queue");

    block_on(client.read_cached(read, shown)).unwrap();
    assert_eq!(pages.try_iter().collect::<Vec<_>>(), [(titles(&["first", "second", "third"]), false)], "kept whole: no part shows over it");
}

/// After a restart the playlist's saved copy shows before anything is asked; the same answer shows
/// nothing more.
#[test]
fn a_long_playlist_shows_its_saved_copy_after_a_restart() {
    let dir = nori_testdir::TempDir::new("plugin-saved");
    let read = Read::PlaylistById { id: "test-long".into() };
    let whole = (titles(&["first", "second", "third"]), false);
    {
        let (client, _requests, go) = open(&dir);
        go.send(()).unwrap();
        go.send(()).unwrap();
        let (shown, pages) = channel();
        block_on(client.read_cached(read.clone(), Arc::new(Shown(Mutex::new(shown))))).unwrap();
        assert_eq!(pages.try_iter().last(), Some(whole.clone()));
    }
    let (client, requests, go) = open(&dir);
    let (shown, pages) = channel();
    let reading = {
        let (client, read, shown) = (client.clone(), read.clone(), Arc::new(Shown(Mutex::new(shown))));
        std::thread::spawn(move || block_on(client.read_cached(read, shown)))
    };
    assert_eq!(pages.recv_timeout(WAIT).expect("the saved copy, before the source answered"), whole);
    assert_eq!(requests.recv_timeout(WAIT).unwrap(), "https://allowed.example/second", "asked anew");
    go.send(()).unwrap();
    go.send(()).unwrap();
    reading.join().unwrap().unwrap();
    assert!(pages.try_iter().next().is_none(), "the same answer: nothing moves");
}
