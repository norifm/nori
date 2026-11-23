//! Plugins' requests go through the open profile's transport (its proxy): checking another profile over
//! that profile's own, or an account opened apart from it, borrows them only while it lasts.

use std::sync::{Arc, Mutex};

use nori_core::client::Client;
use nori_core::providers::Kinds;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, Exchange, Network, Transport, TransportError, TransportResponse};
use nori_core::Core;
use nori_plugins::testing::{fixture, FIXTURE};

/// A profile's connections: each URL asked through them, answered empty.
#[derive(Default)]
struct Connections(Mutex<Vec<String>>);

impl Connections {
    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl Transport for Connections {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.0.lock().unwrap().push(url);
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

struct Rig {
    _dir: nori_testdir::TempDir,
    plugins: Arc<nori_plugins::Plugins>,
    kinds: Arc<Kinds>,
    settings: Arc<Settings>,
    path: String,
}

fn rig(name: &str) -> Rig {
    let dir = nori_testdir::TempDir::new(name);
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&fixture("")).unwrap();
    plugins.turn(FIXTURE.into(), true);
    let kinds = Kinds::with_plugins(plugins.clone());
    Rig { _dir: dir, plugins, kinds, settings, path }
}

impl Rig {
    /// A client of `profile` over `net`.
    fn client(&self, profile: &SavedServer, net: Arc<Connections>) -> Arc<Client> {
        let core = Core::new(self.path.clone(), profile.id.clone(), Arc::new(nori_core::queue::Session::new(self.settings.clone()))).unwrap();
        core.use_kinds(self.kinds.clone());
        core.configure_profile(profile.clone()).unwrap();
        Client::new(core, net, Default::default())
    }

    /// Where the test plugin's next request goes.
    fn plugin_asks(&self) {
        self.plugins.act(FIXTURE.into(), "put".into());
        let w = self.plugins.worker(FIXTURE).unwrap();
        assert_eq!(block_on(w.call(|_, _| Ok(1))), Some(1));
    }
}

fn plugin_profile(id: &str) -> SavedServer {
    SavedServer { id: id.into(), kind: ServerKind::Plugin { id: FIXTURE.into() }, ..Default::default() }
}

const PUT: &str = "https://allowed.example/put";

#[test]
fn a_check_borrows_the_plugins_only_while_it_lasts() {
    let r = rig("plugin-transport-check");
    let (open, other) = (Arc::new(Connections::default()), Arc::new(Connections::default()));
    let _open = r.client(&plugin_profile("open"), open.clone());
    r.kinds.set_transport(open.clone());
    let _ = block_on(r.kinds.check_profile(other.clone(), plugin_profile("other")));
    r.plugin_asks();
    assert_eq!((open.asked(), other.asked()), (vec![PUT.to_string()], Vec::new()), "the open profile's connections, not the checked one's");
}

#[test]
fn an_account_apart_gives_the_plugins_back() {
    let r = rig("plugin-transport-apart");
    let (open, apart) = (Arc::new(Connections::default()), Arc::new(Connections::default()));
    let subsonic = SavedServer { id: "own".into(), url: "http://own".into(), ..Default::default() };
    for (profile, kind) in [(plugin_profile("open"), ServerKind::Plugin { id: FIXTURE.into() }), (subsonic, ServerKind::Plugin { id: FIXTURE.into() })] {
        let session = r.client(&profile, open.clone());
        r.kinds.set_transport(open.clone());
        // As an account apart signing in, which hands the plugin its client.
        let _apart = r.client(&plugin_profile("apart"), apart.clone());
        r.kinds.set_transport(apart.clone());
        session.take_plugins_back(profile.clone(), kind).unwrap();
        r.plugin_asks();
        assert_eq!((open.asked(), apart.asked()), (vec![PUT.to_string()], Vec::new()), "{:?}", profile.kind);
    }
}
