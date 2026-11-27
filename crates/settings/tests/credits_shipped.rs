//! Every crate a client's licenses page credits is in that client's dependency tree, as its release builds
//! it: `cargo tree` over its package, target and features.

use std::collections::BTreeSet;
use std::process::Command;

use nori_settings::credits::{credited_crates, CreditClient};

/// Each client's package, the target its tree is read for, and the features its builds turn on: Android's
/// beat model (android/core/build.gradle.kts), the web's codecs module (web/build.sh).
const BUILDS: [(CreditClient, &str, &str, &str); 5] = [
    (CreditClient::Android, "nori-android", "aarch64-linux-android", "neural-beats"),
    (CreditClient::Web, "nori-web", "wasm32-unknown-unknown", "rust-codecs"),
    (CreditClient::Desktop, "nori-desktop", "x86_64-unknown-linux-gnu", ""),
    (CreditClient::Terminal, "nori-cli", "x86_64-unknown-linux-gnu", ""),
    (CreditClient::Ipod, "nori-ios", "aarch64-apple-ios", ""),
];

/// The packages `package` links for `target`: normal dependencies, not build tools or proc macros.
fn tree(package: &str, target: &str, features: &str) -> BTreeSet<String> {
    let mut cargo = Command::new(env!("CARGO"));
    cargo.current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).args(["tree", "--frozen", "-p", package, "--target", target, "-e", "normal,no-proc-macro", "--prefix", "none", "-f", "{p}"]);
    if !features.is_empty() {
        cargo.args(["--features", features]);
    }
    let out = cargo.output().expect("cargo tree");
    assert!(out.status.success(), "cargo tree -p {package}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf-8").lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect()
}

#[test]
fn every_credited_crate_ships() {
    for (client, package, target, features) in BUILDS {
        let tree = tree(package, target, features);
        let missing: Vec<String> = credited_crates(client)
            .into_iter()
            .flat_map(|(name, crates)| crates.iter().filter(|c| !c.split('|').any(|c| tree.contains(c))).map(move |c| format!("{name}: {c}")))
            .collect();
        assert!(missing.is_empty(), "{client:?} credits crates {package} doesn't link: {missing:?}");
    }
}
