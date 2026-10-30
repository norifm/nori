//! Authenticated LAN control and scoped jams. Devices pair using a random secret; Subsonic profiles
//! can also use their existing account credential. Discovery carries only a keyed tag. A jam uses its
//! own invite credential and derives a separate credential for every guest, without sharing an account.

use alloc::{format, string::{String, ToString}, vec::Vec};
use sha2::{Digest, Sha256};

/// A 256-bit pairing or invite credential from the operating system.
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub fn new_secret() -> std::io::Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(hex(&bytes))
}

/// Query parameters encoded for the LAN protocol.
pub fn target(endpoint: &str, params: &[(String, String)]) -> String {
    format!("/rest/{endpoint}?{}", params.iter().map(|(k,v)| format!("{}={}", crate::encode(k), crate::encode(v))).collect::<Vec<_>>().join("&"))
}

/// A member's credential is derived on both devices and never sent over the LAN.
pub fn member_secret(invite: &str, nonce: &str) -> String {
    hex(&hmac(invite.as_bytes(), format!("nori-jam-member:{nonce}").as_bytes()))
}

/// The secret in a temporary YouTube LAN jam profile.
pub fn guest_secret(api_key: &str) -> Option<&str> {
    api_key.strip_prefix("nori-jam-lan-")
}

/// Device pairing is separate from a jam invitation.
pub fn pairing_link(secret: &str) -> String {
    format!("nori://pair#k={secret}")
}

/// A pairing link contains a 256-bit secret, never an account identifier.
pub fn parse_pairing(link: &str) -> Option<String> {
    let secret = link.trim().strip_prefix("nori://pair#k=")?;
    (secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit())).then(|| secret.to_ascii_lowercase())
}


/// The door listens on a socket, which a browser cannot.
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod door;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use door::{Ask, Asker, Door, Handler, Told, Unanswered};

/// The mDNS service type a door is announced as.
pub const SERVICE: &str = "_nori._tcp";

/// The room a door's answers name.
pub const ROOM: &str = "lan";

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a door announces so the account's devices recognise it: a keyed hash of the user name, so
/// another account's devices on the network neither match it nor learn the name.
pub fn account_tag(user: &str, secret: &str) -> String {
    hex(&hmac(secret.as_bytes(), format!("nori-account:{user}").as_bytes()))[..16].to_string()
}

fn signature(secret: &str, method: &str, target: &str, body: &[u8]) -> String {
    let mut msg = format!("{method} {target}\n").into_bytes();
    msg.extend_from_slice(body);
    hex(&hmac(secret.as_bytes(), &msg))
}

/// How far a request's time may be from the door's: a captured request cannot be sent again later.
const SKEW_MS: i64 = 60_000;

/// Whether `target` (path and query, `sig` last) was signed with `secret` within [`SKEW_MS`] of `now_ms`.
pub fn verified(secret: &str, method: &str, target: &str, body: &[u8], now_ms: i64) -> bool {
    let Some((signed, sig)) = target.rsplit_once("&sig=") else { return false };
    let fresh = query(signed, "ts").and_then(|t| t.parse::<i64>().ok()).is_some_and(|ts| now_ms.abs_diff(ts) <= SKEW_MS as u64);
    let expected = signature(secret, method, signed, body);
    fresh && expected.len() == sig.len() && expected.bytes().zip(sig.bytes()).fold(0, |difference, (a, b)| difference | (a ^ b)) == 0
}

/// Parameter `key` of a request target, undecoded (doors are only sent unreserved characters).
pub fn query<'a>(target: &'a str, key: &str) -> Option<&'a str> {
    target.split_once('?')?.1.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// `path?query` signed for a door: `ts` and `sig` appended.
pub fn signed(secret: &str, method: &str, path_query: &str, body: &[u8], now_ms: i64) -> String {
    let target = format!("{path_query}{}ts={now_ms}", if path_query.contains('?') { '&' } else { '?' });
    let sig = signature(secret, method, &target, body);
    format!("{target}&sig={sig}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(hex(&hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let long_key = [0xaa; 131];
        assert_eq!(hex(&hmac(&long_key, b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn requests_need_the_account_secret_and_a_fresh_time() {
        let t = signed("sesame", "POST", "/rest/noriRemote.send?dev=a", b"{}", 1_000_000);
        assert!(verified("sesame", "POST", &t, b"{}", 1_000_000 + SKEW_MS));
        assert!(!verified("other", "POST", &t, b"{}", 1_000_000), "another secret");
        assert!(!verified("sesame", "POST", &t, b"{\"x\":1}", 1_000_000), "another body");
        assert!(!verified("sesame", "GET", &t, b"{}", 1_000_000), "another method");
        assert!(!verified("sesame", "POST", &t.replace("dev=a", "dev=b"), b"{}", 1_000_000), "another device");
        assert!(!verified("sesame", "POST", &t, b"{}", 1_000_001 + SKEW_MS), "too old");
        assert_eq!(account_tag("ann", "pw"), account_tag("ann", "pw"));
        assert_ne!(account_tag("ann", "pw"), account_tag("ann", "pw2"));
        assert_ne!(account_tag("ann", "pw"), account_tag("bob", "pw"));
    }

}
