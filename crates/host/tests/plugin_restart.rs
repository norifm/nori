//! A plugin's instance started anew after a failed call is told the profile open again: its account
//! stays the profile's, not a signed-out one's.

use std::sync::Arc;

use nori_core::cache_policy::Read;
use nori_core::client::Client;
use nori_core::providers::Kinds;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::block_on;
use nori_core::Core;

#[test]
fn an_instance_started_anew_keeps_the_profile_open() {
    let dir = nori_testdir::TempDir::new("plugin-restart");
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    let profile = SavedServer { id: "p".into(), name: "ended".into(), kind: ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }, ..Default::default() };
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    let kinds = Kinds::with_plugins(plugins);
    core.use_kinds(kinds.clone());
    core.configure_profile(profile.clone()).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    block_on(client.read_now(Read::ArtistIndex)).unwrap();
    assert!(kinds.rules_of(profile.clone(), false).mixes, "signed in once a read went through");

    assert!(block_on(client.read_now(Read::SongById { id: "trap".into() })).is_err(), "the call fails");
    block_on(client.read_now(Read::ArtistIndex)).unwrap();
    assert!(kinds.rules_of(profile, false).mixes, "the new instance has the profile's account");
}
