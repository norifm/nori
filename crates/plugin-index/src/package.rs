//! Whether a fetched package may install, for both plugin hosts (crates/plugins and the web's): it is the
//! one its repo lists, signed as listed, and by the key its plugin pinned.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a repo lists of one version's package, hex as in `index.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub sha256: String,
    /// Its author's ed25519 public key, and the key's signature over the package's sha256 (its 32 bytes):
    /// both, or neither for an unsigned version.
    pub key: Option<String>,
    pub signature: Option<String>,
}

/// Why a fetched package doesn't install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PackageRefusal {
    /// Its sha256 isn't the one listed.
    NotListed,
    /// A key without a signature or the reverse, or a signature that doesn't verify.
    Signature,
}

/// `body`'s sha256 (hex) when it is the package `listed` names, signed as listed.
pub fn checked(body: &[u8], listed: &Listed) -> Result<String, PackageRefusal> {
    let digest = Sha256::digest(body);
    let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if !hash.eq_ignore_ascii_case(&listed.sha256) {
        return Err(PackageRefusal::NotListed);
    }
    match (&listed.key, &listed.signature) {
        (None, None) => Ok(hash),
        (Some(key), Some(signature)) if signed(key, signature, &digest) => Ok(hash),
        _ => Err(PackageRefusal::Signature),
    }
}

/// Whether a version signed by `key` (None: unsigned, or a file) may replace a plugin that pinned
/// `pinned`: any may while none is pinned; once one is, only that key's.
pub fn admits(pinned: Option<&str>, key: Option<&str>) -> bool {
    pinned.is_none_or(|p| key.is_some_and(|k| k.eq_ignore_ascii_case(p)))
}

/// The bytes hex `text` spells; None when it isn't `N` of them.
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let mut out = [0; N];
    if text.len() != 2 * N {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Whether `signature` (hex) is ed25519 key `key`'s (hex) over `digest`.
fn signed(key: &str, signature: &str, digest: &[u8]) -> bool {
    let (Some(key), Some(signature)) = (unhex::<32>(key), unhex::<64>(signature)) else { return false };
    ed25519_dalek::VerifyingKey::from_bytes(&key).is_ok_and(|k| k.verify_strict(digest, &ed25519_dalek::Signature::from_bytes(&signature)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `body` listed as signed by the key made of `seed`.
    fn listed(body: &[u8], seed: u8) -> Listed {
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        let digest = Sha256::digest(body);
        Listed { sha256: hex(&digest), key: Some(hex(key.verifying_key().as_bytes())), signature: Some(hex(&key.sign(&digest).to_bytes())) }
    }

    #[test]
    fn a_package_installs_only_as_its_repo_lists_it() {
        let body = b"package";
        let signed = listed(body, 1);
        let unsigned = Listed { key: None, signature: None, ..signed.clone() };
        let other = listed(body, 2);
        let upper = Listed { sha256: signed.sha256.to_ascii_uppercase(), ..signed.clone() };
        for (what, listed, body, want) in [
            ("signed", &signed, &body[..], Ok(())),
            ("unsigned", &unsigned, body, Ok(())),
            ("listed in capitals", &upper, body, Ok(())),
            ("another package", &signed, b"changed", Err(PackageRefusal::NotListed)),
            ("a key without its signature", &Listed { signature: None, ..signed.clone() }, body, Err(PackageRefusal::Signature)),
            ("a signature without its key", &Listed { key: None, ..signed.clone() }, body, Err(PackageRefusal::Signature)),
            ("another key's signature", &Listed { signature: other.signature.clone(), ..signed.clone() }, body, Err(PackageRefusal::Signature)),
            ("a signature that isn't hex", &Listed { signature: Some("zz".repeat(64)), ..signed.clone() }, body, Err(PackageRefusal::Signature)),
        ] {
            assert_eq!(checked(body, listed).map(drop), want, "{what}");
        }
        assert_eq!(checked(body, &signed).unwrap(), signed.sha256);
    }

    #[test]
    fn a_pinned_key_admits_only_itself() {
        for (pinned, key, want) in [
            (None, None, true),
            (None, Some("ab"), true),
            (Some("ab"), Some("AB"), true),
            (Some("ab"), Some("cd"), false),
            (Some("ab"), None, false),
        ] {
            assert_eq!(admits(pinned, key), want, "{pinned:?} {key:?}");
        }
    }
}
