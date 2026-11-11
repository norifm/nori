//! A plugin provider's Library, through the plugin API: its artists and its songs a page at a time,
//! as every client reads them.

use std::sync::Arc;

use nori_core::cache_policy::{Page, Read};
use nori_core::client::Client;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::block_on;
use nori_core::Core;

#[test]
fn a_plugins_library_lists_its_artists_and_songs() {
    let dir = nori_testdir::TempDir::new("plugin-library");
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    let profile = SavedServer { id: "p".into(), kind: ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }, ..Default::default() };
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    core.use_kinds(nori_core::providers::Kinds::with_plugins(plugins));
    core.configure_profile(profile).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());

    let Page::Artists { v } = block_on(client.read_now(Read::ArtistIndex)).unwrap() else { panic!("artists") };
    assert_eq!(v.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Followed", "Subscribed"]);

    let page = block_on(client.songs_listed("title".into(), false, false, 0, 0, 1)).unwrap();
    assert_eq!(page.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["two", "three"], "from the offset asked, in the plugin's order");
    assert!(page.exhausted, "fewer than a page: the list ends there");
}
