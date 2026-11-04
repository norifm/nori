//! What a saved profile of a plugin's kind waits for before it opens.

use std::sync::Arc;
use nori_core::providers::{Kinds, ProfileWait};
use nori_core::settings::{SavedServer, ServerKind, StoredPrefs};
use nori_core::settings_store::Settings;
use nori_plugins::testing::{fixture, Quiet, FIXTURE};

/// It waits for its plugin to be installed, then turned on, and for the switch over every plugin; a
/// profile of another plugin keeps waiting for that one.
#[test]
fn a_plugin_profile_waits_until_its_plugin_runs() {
    let dir = nori_testdir::TempDir::new("profile-wait");
    let settings = Settings::new();
    settings.open(dir.join("nori.db").to_str().unwrap()).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(Quiet));
    let kinds = Kinds::with_plugins(plugins.clone());
    let of = |id: &str| SavedServer { id: "p".into(), kind: ServerKind::Plugin { id: id.into() }, ..SavedServer::default() };
    assert_eq!(kinds.waits(of(FIXTURE)), Some(ProfileWait::Install));
    plugins.install(&fixture("")).unwrap();
    plugins.turn(FIXTURE.into(), false);
    assert_eq!(kinds.waits(of(FIXTURE)), Some(ProfileWait::TurnOn));
    plugins.turn(FIXTURE.into(), true);
    assert_eq!(kinds.waits(of(FIXTURE)), None);
    settings.put(StoredPrefs { plugins_off: true, ..StoredPrefs::default() });
    assert_eq!(kinds.waits(of(FIXTURE)), Some(ProfileWait::PluginsOn));
    assert_eq!(kinds.waits(of("fm.nori.elsewhere")), Some(ProfileWait::Install));
    assert_eq!(kinds.waits(SavedServer::default()), None, "a server's");
}
