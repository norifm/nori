//! A page a plugin provider reads in many requests, one after another (a long playlist's pages), shows
//! what the first brought while the rest are asked.

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

/// The songs of each playlist page shown.
struct Shown(Mutex<Sender<Vec<String>>>);

impl PageShown for Shown {
    fn show(&self, page: Page) {
        if let Page::PlaylistPage { v } = page {
            self.0.lock().unwrap().send(v.songs.into_iter().map(|s| s.title).collect()).unwrap();
        }
    }
}

#[test]
fn a_long_playlist_shows_its_first_page_while_the_rest_is_asked() {
    let dir = nori_testdir::TempDir::new("plugin-parts");
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
    let client = Client::new(core, net, Default::default());
    let (shown, pages) = channel();
    let shown = Arc::new(Shown(Mutex::new(shown)));
    let read = Read::PlaylistById { id: "test-long".into() };
    let reading = {
        let (client, read, shown) = (client.clone(), read.clone(), shown.clone());
        std::thread::spawn(move || block_on(client.read_cached(read, shown)))
    };
    let wait = Duration::from_secs(10);
    assert_eq!(pages.recv_timeout(wait).expect("the first page, shown before the second came"), ["first"]);
    assert_eq!(requests.recv_timeout(wait).unwrap(), "https://allowed.example/second");
    go.send(()).unwrap();
    reading.join().unwrap().unwrap();
    assert_eq!(pages.recv_timeout(wait).unwrap(), ["first", "second"]);

    block_on(client.read_cached(read, shown)).unwrap();
    assert_eq!(pages.try_iter().collect::<Vec<_>>(), [["first", "second"]], "kept whole: no part shows over it");
}
