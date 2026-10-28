//! Authenticated LAN control and scoped jams. Devices pair using a random secret; Subsonic profiles
//! can also use their existing account credential. Discovery carries only a keyed tag. A jam uses its
//! own invite credential and derives a separate credential for every guest, without sharing an account.

use sha2::{Digest, Sha256};

/// A 256-bit pairing or invite credential from the operating system.
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
mod door;
#[cfg(not(target_arch = "wasm32"))]
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

}
