//! The plugin API's interfaces pinned to its number: a plugin built for API N must start in every nori
//! that runs API N (nori 0.8.0 changed `device-state` under API 3, and every plugin built before it
//! stopped starting, issue #143).

mod common;

use std::sync::Arc;

use nori_plugins::testing::FIXTURE;
use nori_plugins::wit::provider::{Page, Profile, Request};
use nori_plugins::{GuestStreams, Plugins, SignInCopy};
use sha2::{Digest, Sha256};

/// The API and its WIT's fingerprint.
const PINNED: (u32, &str) = (7, "ce654f06fc78f2098f076d02ce57f8291489315b6f8f82893ccb3a259122b92e");

/// `wit/`'s files as the interfaces they declare: comments and blank lines left out, each line trimmed.
fn fingerprint() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "wit")).collect();
    files.sort();
    let mut hash = Sha256::new();
    for f in files {
        hash.update(f.file_name().unwrap().as_encoded_bytes());
        for line in std::fs::read_to_string(&f).unwrap().lines() {
            let code = line.split("//").next().unwrap().trim();
            if !code.is_empty() {
                hash.update(code.as_bytes());
                hash.update(b"\n");
            }
        }
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Changing the WIT: a plugin already built for this API must still start. Only an addition keeps
/// that: a new host import, or a new extension point in the `current` world that the host looks up
/// by name (`runtime::Exports`), the package's version kept (nori stubs an import it doesn't have by
/// its exact name). A changed record, function or export doesn't, and takes a new API
/// (`section::API`, the SDK's `API`, and the plugins rebuilt). Either way the fingerprint here follows.
#[test]
fn the_wit_is_pinned_to_its_api() {
    let (api, pinned) = PINNED;
    assert_eq!(api, nori_plugins::API, "a new API pins its WIT here");
    assert_eq!(fingerprint(), pinned, "the WIT changed under API {api}: see this test's doc");
}

/// A plugin built by nori 1.1.0 (the real build, `testing::built_by_nori_1_1`) starts in this nori and
/// answers, and what was added since reads as its defaults: each device signs in itself, no account, a
/// jam only on the host's network. The plugin built now says its own.
#[test]
fn a_plugin_built_before_an_addition_runs_with_its_defaults() {
    let built = [
        (nori_plugins::testing::built_by_nori_1_1(), (SignInCopy::EachDevice, GuestStreams::SameNetwork, None)),
        (nori_plugins::testing::fixture(""), (SignInCopy::Copied, GuestStreams::GuestLookup, Some("ada-account".to_string()))),
    ];
    for (from, said) in built {
        let dir = nori_testdir::TempDir::new("plugins-addition");
        let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(common::Noted::default()));
        let manifest = plugins.install(&from).unwrap();
        plugins.turn(FIXTURE.into(), true);
        let worker = plugins.worker(FIXTURE).expect("it runs");
        let profile = Profile { id: "p".into(), name: "ended".into(), address: String::new(), user: String::new(), password: String::new(), api_key: String::new() };
        let account = nori_tasks::block_on(worker.call(move |s, x| {
            x.nori_plugin_provider().call_open(&mut *s, &profile)?;
            // Its session, ended at opening, is taken up again by a read.
            let page = x.nori_plugin_provider().call_read(&mut *s, &Request::Artists)?;
            assert!(matches!(page, Ok(Page::Artists(a)) if a.len() == 2), "it answers");
            x.call_account_id(s)
        }));
        let account = account.expect("its calls went through");
        assert_eq!((manifest.sign_in_copy, manifest.guest_streams, account), said, "{from:?}");
    }
}
