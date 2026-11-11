//! The plugin API's interfaces pinned to its number: a plugin built for API N must start in every nori
//! that runs API N (nori 0.8.0 changed `device-state` under API 3, and every plugin built before it
//! stopped starting, issue #143).

use sha2::{Digest, Sha256};

/// The API and its WIT's fingerprint.
const PINNED: (u32, &str) = (4, "5c72db31a63594ba7d64f9175876ae147df0a8e027b7a0a42a4e6376d7b9a70b");

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

/// Changing the WIT: a plugin already built for this API must still start. Only a new host import
/// keeps that; a changed record, function or export doesn't, and takes a new API (`section::API`,
/// the SDK's `API`, and the plugins rebuilt). Either way the fingerprint here follows.
#[test]
fn the_wit_is_pinned_to_its_api() {
    let (api, pinned) = PINNED;
    assert_eq!(api, nori_plugins::API, "a new API pins its WIT here");
    assert_eq!(fingerprint(), pinned, "the WIT changed under API {api}: see this test's doc");
}
