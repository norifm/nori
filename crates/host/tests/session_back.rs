//! A plugin provider whose session ended takes it up again by itself: the core asks it anew, its
//! library comes back and the offer to sign in again closes.

use std::sync::Arc;

use nori_core::cache_policy::Read;
use nori_core::client::Client;
use nori_core::providers::flow::SignInEvent;
use nori_core::providers::AccountState;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::transport::block_on;
use nori_core::Core;

#[test]
fn a_session_the_plugin_takes_up_again_is_signed_in_here() {
    let dir = nori_testdir::TempDir::new("session-back");
    let settings = Settings::new();
    let path = nori_host::db_path(dir.path());
    settings.open(&path).unwrap();
    let plugins = nori_plugins::open(dir.join("plugins"), settings.clone(), Arc::new(nori_plugins::testing::Quiet));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    let profile = SavedServer { id: "p".into(), name: "ended".into(), kind: ServerKind::Plugin { id: nori_plugins::testing::FIXTURE.into() }, ..Default::default() };
    let core = Core::new(path, profile.id.clone(), Arc::new(nori_core::queue::Session::new(settings))).unwrap();
    core.use_kinds(nori_core::providers::Kinds::with_plugins(plugins));
    core.configure_profile(profile).unwrap();
    let client = Client::new(core, nori_http::Http::new(), Default::default());
    assert_eq!(client.account(), AccountState::Ended);
    assert_eq!(client.sign_in_due(None), Some(SignInEvent::Offer { ended: true }), "offered again at start");
    assert!(!client.core().rules().stars);

    block_on(client.read_now(Read::ArtistIndex)).unwrap();
    assert_eq!(client.account(), AccountState::SignedIn, "asked anew once a read went through");
    assert!(client.core().rules().stars, "its likes are offered again");
    assert_eq!(client.sign_in_due(None), Some(SignInEvent::Close), "the offer closes");
}
