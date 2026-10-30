//! Signs a `.noriplugin` for its repo's index: the author's ed25519 key's signature over the package's
//! sha256 (its 32 bytes), as nori checks it at install.
//!
//!   cargo run -p nori-plugin-sdk --example sign -- new KEYFILE      a new key (its seed, hex), never overwritten
//!   cargo run -p nori-plugin-sdk --example sign -- KEYFILE PACKAGE  the version's index fields, as JSON
//!
//! Keep KEYFILE secret and safe: every update of a plugin must be signed by the key its users first
//! installed it with.

use std::io::Read;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn fail(why: String) -> ! {
    eprintln!("sign: {why}");
    std::process::exit(1)
}

fn key(file: &str) -> SigningKey {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
    let text = text.trim();
    let seed: Option<Vec<u8>> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect();
    let seed: [u8; 32] = seed.and_then(|s| s.try_into().ok()).unwrap_or_else(|| fail(format!("{file} isn't a key this made")));
    SigningKey::from_bytes(&seed)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["new", file] => {
            let mut seed = [0u8; 32];
            std::fs::File::open("/dev/urandom").and_then(|mut r| r.read_exact(&mut seed)).unwrap_or_else(|e| fail(format!("no randomness: {e}")));
            let mut out = std::fs::File::options().write(true).create_new(true).open(file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
            std::io::Write::write_all(&mut out, hex(&seed).as_bytes()).unwrap_or_else(|e| fail(format!("{file}: {e}")));
            println!("{}", hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()));
        }
        [file, package] => {
            let key = key(file);
            let bytes = std::fs::read(package).unwrap_or_else(|e| fail(format!("{package}: {e}")));
            let digest = Sha256::digest(&bytes);
            println!(
                r#""sha256": "{}", "key": "{}", "signature": "{}""#,
                hex(&digest),
                hex(key.verifying_key().as_bytes()),
                hex(&key.sign(&digest).to_bytes())
            );
        }
        _ => fail("usage: sign new KEYFILE | sign KEYFILE PACKAGE".into()),
    }
}
