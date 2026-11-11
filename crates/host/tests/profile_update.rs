//! A profile whose plugin an older nori installed, built for its plugin API: it waits while the plugin
//! is updated, then opens with the update.

use std::sync::Arc;

use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::providers::{Kinds, ProfileWait};
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, Exchange, Network, Transport, TransportError, TransportResponse};
use nori_core::Core;
use nori_plugins::testing::{installed_by_an_older_nori, repo_offering, Quiet, FIXTURE};

/// Answers each URL it serves with its body, any other 404.
struct Served(Vec<(String, Vec<u8>)>);

#[async_trait::async_trait]
impl Transport for Served {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        Ok(match self.0.iter().find(|(u, _)| *u == url) {
            Some((_, body)) => TransportResponse { status: 200, body: body.clone(), headers: Vec::new() },
            None => TransportResponse { status: 404, body: Vec::new(), headers: Vec::new() },
        })
    }

    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}

#[test]
fn a_profile_waits_for_its_plugins_update_then_opens_with_it() {
    let dir = nori_testdir::TempDir::new("profile-update");
    let repo = "https://own.example/index.json";
    installed_by_an_older_nori(&dir.join("plugins"), Some(repo), false);
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(Quiet));
    let kinds = Kinds::with_plugins(plugins.clone());
    let profile = SavedServer { id: "p".into(), kind: ServerKind::Plugin { id: FIXTURE.into() }, ..Default::default() };
    assert_eq!(kinds.waits(profile.clone()), Some(ProfileWait::Update), "not opened with a plugin that can't run");
    assert!(plugins.updates_due(0), "its update is due at once");

    let version = plugins.installed()[0].manifest.version.clone();
    kinds.set_transport(Arc::new(Served(repo_offering(repo, &version))));
    assert_eq!(block_on(plugins.update(0)).len(), 1);
    assert_eq!(kinds.waits(profile.clone()), None, "the update runs: the profile opens");

    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    core.use_kinds(kinds);
    core.configure_profile(profile).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    let Page::Artists { v } = block_on(client.read_now(Read::ArtistIndex)).unwrap() else { panic!("artists") };
    assert_eq!(v.len(), 2, "its library reads through the update");
}
