//! A plugin's code sign-in, through the plugin API: the code it makes is shown, and its check, handed
//! the code back, signs the account in.

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use nori_core::client::Client;
use nori_core::providers::code::{CodeHeard, CodeSignIn, SignInCode};
use nori_core::providers::flow::SignInEvent;
use nori_core::providers::SignInResult;
use nori_core::settings::{SavedServer, ServerKind};
use nori_core::settings_store::Settings;
use nori_core::Core;

struct Heard(Mutex<mpsc::Sender<SignInEvent>>);

impl CodeHeard for Heard {
    fn heard(&self, event: SignInEvent) {
        let _ = self.0.lock().unwrap().send(event);
    }
}

#[test]
fn a_plugins_code_is_shown_and_its_check_signs_in() {
    let dir = nori_testdir::TempDir::new("code-sign-in");
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
    let tasks = Arc::new(nori_tasks::Native::simulated("code", 0).unwrap());
    let (tx, heard) = mpsc::channel();
    let _code = CodeSignIn::with_tasks(client, Arc::new(Heard(Mutex::new(tx))), tasks.clone());
    let next = || heard.recv_timeout(Duration::from_secs(30)).expect("the plugin answered");
    assert_eq!(next(), SignInEvent::CodeShown { code: SignInCode { code: "TEST-1".into(), url: "https://allowed.example/activate/TEST-1".into() } });
    tasks.settle();
    assert_eq!(tasks.waits(), [Duration::from_secs(2)], "checked at the plugin's pace");
    tasks.advance();
    assert_eq!(next(), SignInEvent::Answered { result: SignInResult::SignedIn { name: "TEST-1 poll".into() } });
}
