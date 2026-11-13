//! A plugin provider's account acting as another of its channels: the client lists them, the
//! library kept from the one before is not shown as the new one's, and the picture is the new one's.

use std::sync::{Arc, Mutex};

use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::library::PageShown;
use nori_core::providers::IdentityKind;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::block_on;
use nori_core::Core;

#[derive(Default)]
struct Shown(Mutex<Vec<String>>);

impl PageShown for Shown {
    fn show(&self, page: Page) {
        if let Page::Playlists { v } = page {
            self.0.lock().unwrap().extend(v.into_iter().map(|p| p.name));
        }
    }
}

fn playlists_shown(client: &Client) -> Vec<String> {
    let shown = Arc::new(Shown::default());
    block_on(client.read_cached(Read::PlaylistList, shown.clone())).unwrap();
    let names = std::mem::take(&mut *shown.0.lock().unwrap());
    names
}

#[test]
fn another_channel_chosen_shows_its_own_library() {
    let dir = nori_testdir::TempDir::new("identities");
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    let profile = SavedServer { id: "p".into(), name: "Ada".into(), kind: ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }, ..Default::default() };
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    let kinds = nori_core::providers::Kinds::with_plugins(plugins);
    core.use_kinds(kinds.clone());
    core.configure_profile(profile).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    let fixture = kinds.providers().into_iter().find(|p| p.kind == ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }).unwrap();
    assert_eq!(fixture.identities, Some(IdentityKind::Channel));
    let listed = |c: &Client| block_on(c.identities()).unwrap().into_iter().map(|i| (i.name, i.chosen)).collect::<Vec<_>>();
    assert_eq!(listed(&client), [("Ada".to_string(), true), ("Ada's covers".into(), false)]);
    assert_eq!(playlists_shown(&client), ["Ada"]);
    assert_eq!(block_on(client.account_picture()).as_deref(), Some("https://pictures.example/ada=s{size}"));

    block_on(client.choose_identity(1)).unwrap();
    assert_eq!(listed(&client), [("Ada".to_string(), false), ("Ada's covers".into(), true)]);
    assert_eq!(playlists_shown(&client), ["Ada's covers"], "the kept list was the other channel's");
    assert_eq!(block_on(client.account_picture()).as_deref(), Some("https://pictures.example/covers=s{size}"));
}
