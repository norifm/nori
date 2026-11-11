//! Authenticated LAN control and scoped jams. Devices pair using a random secret; Subsonic profiles
//! can also use their existing account credential. Discovery carries only a keyed tag. A jam uses its
//! own invite credential and derives a separate credential for every guest, without sharing an account.

use alloc::{format, string::{String, ToString}, vec::Vec};
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
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

/// What a device's pairing code (and QR code) carries: the invite its door pairs a new device with,
/// never an account's credential, and where that door is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCode {
    /// 256 bits, hex.
    pub invite: String,
    /// The device showing it.
    pub id: String,
    pub name: String,
    /// Its door at each of its addresses ([`door_addresses`]), at least one.
    pub doors: Vec<SocketAddr>,
}

const PAIR_LINK: &str = "nori://pair#";

/// Device pairing is separate from a jam invitation. Each door is an `a`; an older nori reads the first.
pub fn pairing_link(code: &PairCode) -> String {
    let PairCode { invite, id, name, doors } = code;
    let doors: String = doors.iter().map(|d| format!("&a={}", crate::encode(&d.to_string()))).collect();
    format!("{PAIR_LINK}k={invite}&d={}{doors}&n={}", crate::encode(id), crate::encode(name))
}

/// The code a pairing link carries; None for anything else, an older nori's code without its door among them.
pub fn parse_pairing(link: &str) -> Option<PairCode> {
    let fragment = link.trim().strip_prefix(PAIR_LINK)?;
    let all = |key: &'static str| fragment.split('&').filter_map(|kv| kv.split_once('=')).filter(move |(k, _)| *k == key).filter_map(|(_, v)| crate::decode(v));
    let get = |key| all(key).next();
    let invite = get("k").filter(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()))?.to_ascii_lowercase();
    let id = get("d").filter(|d| !d.is_empty())?;
    let doors: Vec<SocketAddr> = all("a").filter_map(|a| a.parse().ok()).collect();
    (!doors.is_empty()).then(|| PairCode { invite, id, name: get("n").unwrap_or_default(), doors })
}

/// One of a device's IPv4 addresses, and the length of its network's prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Net {
    pub ip: Ipv4Addr,
    pub prefix: u8,
}

impl Net {
    /// Whether `ip` is on this network.
    fn holds(&self, ip: IpAddr) -> bool {
        let IpAddr::V4(ip) = ip else { return false };
        let host = 32u32.saturating_sub(u32::from(self.prefix));
        host >= 32 || u32::from(self.ip) >> host == u32::from(ip) >> host
    }
}

/// Where a device's door can be reached from another device, `first` (the address its traffic to the
/// internet leaves from) before the others: every address of its networks `nets` but loopback. A device
/// on two networks (Ethernet and Wi-Fi) is reached from each.
pub fn door_addresses(nets: &[Net], first: Option<IpAddr>) -> Vec<IpAddr> {
    let first = first.and_then(|ip| match ip { IpAddr::V4(v4) => Some(v4), IpAddr::V6(_) => None });
    let mut out: Vec<IpAddr> = Vec::new();
    for ip in first.into_iter().chain(nets.iter().map(|n| n.ip)) {
        if !ip.is_loopback() && !ip.is_unspecified() && !out.contains(&IpAddr::V4(ip)) {
            out.push(IpAddr::V4(ip));
        }
    }
    out
}

/// The order a device on networks `nets` tries another's `doors` in: those on a network it is on first,
/// then the others, each in the order given. IPv4 ones only: a door listens on IPv4.
pub fn door_order(doors: impl IntoIterator<Item = SocketAddr>, nets: &[Net]) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for door in doors {
        if door.is_ipv4() && !out.contains(&door) {
            out.push(door);
        }
    }
    out.sort_by_key(|door| !nets.iter().any(|n| n.holds(door.ip())));
    out
}

/// The key a device paired with `invite` signs with: made on both devices from the pairing's nonce, never sent.
pub fn pair_key(invite: &str, nonce: &str) -> String {
    hex(&hmac(invite.as_bytes(), format!("nori-pair:{nonce}").as_bytes()))
}

/// Why a device did not pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum PairError {
    /// Not a pairing code.
    NotACode,
    /// This device's own code.
    Own,
    /// This profile does not pair: a server account's devices find each other, and a jam's guest controls none.
    Unavailable,
    /// Nothing answered where the code says the device is: it is on another network, or its remote control is off.
    Unreachable,
    /// It did not answer in time.
    TimedOut,
    /// It turned the code down: it shows another one now.
    Refused,
}

/// The door listens on a socket, which a browser cannot.
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod door;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use door::{Access, Ask, Asker, Door, Handler, PairChange, Pairs, Told, Unanswered};

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

    #[test]
    fn pairing_codes_read_back() {
        let invite = "ab".repeat(32);
        let doors = vec!["192.168.196.241:41234".parse().unwrap(), "192.168.0.105:41234".parse().unwrap()];
        let code = PairCode { invite: invite.clone(), id: "d3sk".into(), name: "Ann's Mac & co".into(), doors };
        assert_eq!(parse_pairing(&format!(" {} ", pairing_link(&code))), Some(code.clone()));
        let link = |fragment: &str| format!("nori://pair#{fragment}");
        let one = |door: &str| PairCode { name: String::new(), doors: vec![door.parse().unwrap()], ..code.clone() };
        let cases = [
            (link(&format!("k={}&d=d3sk&a=10.0.0.2%3A1", invite.to_uppercase())), Some(one("10.0.0.2:1"))),
            (link(&format!("k={invite}&d=d3sk&a=%5Bfe80%3A%3A1%5D%3A41234")), Some(one("[fe80::1]:41234"))),
            (link(&format!("k={invite}&d=d3sk&a=nori.local%3A1&a=10.0.0.2%3A1")), Some(one("10.0.0.2:1"))),
            (link(&format!("k={invite}")), None),
            (link(&format!("k={invite}&d=d3sk")), None),
            (link(&format!("k={invite}&d=d3sk&a=")), None),
            (link(&format!("k={invite}&a=10.0.0.2%3A1")), None),
            (link(&format!("k={}&d=d3sk&a=10.0.0.2%3A1", &invite[1..])), None),
            (link(&format!("k={}&d=d3sk&a=10.0.0.2%3A1", "zz".repeat(32))), None),
            (format!("nori://jam?k={invite}&d=d3sk&a=10.0.0.2%3A1"), None),
            (String::new(), None),
        ];
        for (link, code) in cases {
            assert_eq!(parse_pairing(&link), code, "{link}");
        }
        assert_ne!(pair_key(&invite, "n1"), pair_key(&invite, "n2"));
        assert_ne!(pair_key(&invite, "n1"), pair_key(&"cd".repeat(32), "n1"));
    }

    /// A Mac on Ethernet (192.168.192.0/20, its route to the internet) and on the phone's Wi-Fi
    /// (192.168.0.0/24): its code names both, and the phone tries the Wi-Fi one first.
    #[test]
    fn a_device_on_two_networks_is_reached_on_each() {
        let net = |ip: &str, prefix| Net { ip: ip.parse().unwrap(), prefix };
        let mac = [net("127.0.0.1", 8), net("192.168.0.105", 24), net("192.168.196.241", 20)];
        let ethernet: IpAddr = "192.168.196.241".parse().unwrap();
        let addresses = door_addresses(&mac, Some(ethernet));
        assert_eq!(addresses, ["192.168.196.241", "192.168.0.105"].map(|a| a.parse::<IpAddr>().unwrap()), "the route's first, an older nori reading only it");
        assert_eq!(door_addresses(&mac, Some("fd00::1".parse().unwrap())), ["192.168.0.105", "192.168.196.241"].map(|a| a.parse::<IpAddr>().unwrap()));
        assert!(door_addresses(&mac[..1], None).is_empty());

        let doors: Vec<SocketAddr> = addresses.iter().map(|ip| SocketAddr::new(*ip, 41234)).collect();
        let order = |nets: &[Net], doors: Vec<SocketAddr>| door_order(doors, nets).iter().map(|d| d.to_string()).collect::<Vec<_>>();
        let phone = [net("192.168.0.42", 24)];
        assert_eq!(order(&phone, doors.clone()), ["192.168.0.105:41234", "192.168.196.241:41234"]);
        assert_eq!(order(&[net("192.168.200.7", 20)], doors.clone()), ["192.168.196.241:41234", "192.168.0.105:41234"]);
        assert_eq!(order(&[net("10.0.0.2", 8)], doors.clone()), ["192.168.196.241:41234", "192.168.0.105:41234"], "on neither: as given");
        let v6: SocketAddr = "[fe80::1]:41234".parse().unwrap();
        assert_eq!(order(&phone, vec![v6, doors[1], doors[1], doors[0]]), ["192.168.0.105:41234", "192.168.196.241:41234"], "IPv4 once each");
        assert_eq!(order(&[net("0.0.0.0", 0)], vec![doors[1], doors[0]]), ["192.168.0.105:41234", "192.168.196.241:41234"], "a /0 holds both: as given");
    }

}
