//! A profile's proxy: the SOCKS5 server its connections go through, as its form reads and shows it, and
//! the local destinations reached around it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A SOCKS5 proxy, which looks up the names asked of it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SavedProxy {
    /// A name, an IPv4 address, or an IPv6 address without its brackets.
    pub host: String,
    pub port: u16,
    pub login: Option<ProxyLogin>,
}

/// What a proxy that asks for a login is given (RFC 1929).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProxyLogin {
    pub user: String,
    pub password: String,
}

/// Why the proxy's fields in a profile's form are not a proxy. Messages are for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum ProxyError {
    #[error("not a SOCKS5 proxy")]
    NotSocks5,
    #[error("no host, or not one")]
    Host,
    #[error("the port is not 1 to 65535")]
    Port,
    /// A login in the address, a password without a user, or a part longer than SOCKS5 carries.
    #[error("the login is not one")]
    Login,
}

/// The port a proxy typed without one listens on.
const DEFAULT_PORT: u16 = 1080;

/// The longest user, password or host name SOCKS5 carries: its length is one byte.
const MOST_BYTES: usize = 255;

/// The proxy a profile's proxy sheet gives: `address` as `[socks5://]host[:port]` (an IPv6 host in
/// brackets), signed in to with `user` and `password` when a user is given; None without an address,
/// which is how the sheet removes a proxy, whatever its login fields still hold.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn proxy_from_form(address: String, user: String, password: String) -> Result<Option<SavedProxy>, ProxyError> {
    let (address, user) = (address.trim(), user.trim());
    if address.is_empty() {
        return Ok(None);
    }
    let login = match (user, password.as_str()) {
        ("", "") => None,
        ("", _) => return Err(ProxyError::Login),
        (user, password) if user.len() > MOST_BYTES || password.len() > MOST_BYTES => return Err(ProxyError::Login),
        (user, password) => Some(ProxyLogin { user: user.to_string(), password: password.to_string() }),
    };
    let rest = match address.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("socks5") || scheme.eq_ignore_ascii_case("socks5h") => rest,
        Some(_) => return Err(ProxyError::NotSocks5),
        None => address,
    };
    if rest.contains('@') {
        return Err(ProxyError::Login);
    }
    let (host, port) = host_port(rest)?;
    let port = match port {
        Some(p) => p.parse::<u16>().ok().filter(|&p| p > 0).ok_or(ProxyError::Port)?,
        None => DEFAULT_PORT,
    };
    Ok(Some(SavedProxy { host: host.to_string(), port, login }))
}

/// `host[:port]`, or `[ipv6][:port]`: the host and the port as typed.
fn host_port(address: &str) -> Result<(&str, Option<&str>), ProxyError> {
    if let Some(v6) = address.strip_prefix('[') {
        let (host, after) = v6.split_once(']').ok_or(ProxyError::Host)?;
        host.parse::<Ipv6Addr>().map_err(|_| ProxyError::Host)?;
        return match after {
            "" => Ok((host, None)),
            _ => after.strip_prefix(':').map(|port| (host, Some(port))).ok_or(ProxyError::Host),
        };
    }
    let (host, port) = match address.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (address, None),
    };
    let named = !host.is_empty() && host.len() <= MOST_BYTES && host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b));
    if !named {
        return Err(ProxyError::Host);
    }
    Ok((host, port))
}

/// A profile's proxy's address as its form shows it, which [`proxy_from_form`] reads back: `host:port`,
/// empty without a proxy. Its login has fields of its own.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn proxy_address(proxy: Option<SavedProxy>) -> String {
    proxy.map_or_else(String::new, |p| format!("{}:{}", proxy_host(&p), p.port))
}

/// The proxy's host as an address writes it: an IPv6 one in brackets.
pub fn proxy_host(proxy: &SavedProxy) -> String {
    if proxy.host.contains(':') { format!("[{}]", proxy.host) } else { proxy.host.clone() }
}

/// The networks a profile reaches around its proxy, which elsewhere cannot reach the user's own: loopback,
/// private (RFC 1918), shared (RFC 6598: carrier NAT, Tailscale), link-local and unique-local.
const LOCAL_NETWORKS: [(IpAddr, u8); 9] = [
    (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8),
    (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8),
    (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)), 12),
    (IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)), 16),
    (IpAddr::V4(Ipv4Addr::new(100, 64, 0, 0)), 10),
    (IpAddr::V4(Ipv4Addr::new(169, 254, 0, 0)), 16),
    (IpAddr::V6(Ipv6Addr::LOCALHOST), 128),
    (IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0)), 7),
    (IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)), 10),
];

/// The name of this device, reached around the proxy.
const LOCALHOST: &str = "localhost";

/// The domain of the names mDNS answers on the user's own network, which a proxy elsewhere cannot look up.
const MDNS_DOMAIN: &str = ".local";

/// Whether a connection to `host` (a URL's host: a name, an IPv4 address, or an IPv6 one in brackets or
/// not) goes around the profile's proxy: an address in [`LOCAL_NETWORKS`], `localhost`, or a `.local`
/// name. Every other name goes to the proxy, which looks it up: looking it up here would tell the network
/// what is asked.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn skips_proxy(host: String) -> bool {
    match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(ip) => LOCAL_NETWORKS.iter().any(|&(network, bits)| within(ip.to_canonical(), network, bits)),
        Err(_) => {
            let name = host.trim_end_matches('.').to_ascii_lowercase();
            name == LOCALHOST || (name.ends_with(MDNS_DOMAIN) && name.len() > MDNS_DOMAIN.len())
        }
    }
}

/// What [`skips_proxy`] passes, as a web view's bypass rules take it: `localhost`, the `.local` names and
/// each network in CIDR notation.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn proxy_bypass_rules() -> Vec<String> {
    [LOCALHOST.to_string(), format!("*{MDNS_DOMAIN}")].into_iter().chain(LOCAL_NETWORKS.iter().map(|(network, bits)| format!("{network}/{bits}"))).collect()
}

fn within(ip: IpAddr, network: IpAddr, bits: u8) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(n)) => u32::from(ip) >> (32 - bits) == u32::from(n) >> (32 - bits),
        (IpAddr::V6(ip), IpAddr::V6(n)) => u128::from(ip) >> (128 - bits) == u128::from(n) >> (128 - bits),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(address: &str, user: &str, password: &str) -> Result<Option<SavedProxy>, ProxyError> {
        proxy_from_form(address.into(), user.into(), password.into())
    }

    fn proxy(host: &str, port: u16, login: Option<(&str, &str)>) -> Option<SavedProxy> {
        Some(SavedProxy { host: host.into(), port, login: login.map(|(user, password)| ProxyLogin { user: user.into(), password: password.into() }) })
    }

    #[test]
    fn a_proxy_is_read_from_its_fields() {
        let cases = [
            (("", "", ""), Ok(None)),
            (("  ", " ", ""), Ok(None)),
            (("proxy.lan", "", ""), Ok(proxy("proxy.lan", 1080, None))),
            ((" socks5://10.0.0.1:9050 ", "", ""), Ok(proxy("10.0.0.1", 9050, None))),
            (("SOCKS5H://proxy.lan:1081", " me ", "p@ss:w0rd "), Ok(proxy("proxy.lan", 1081, Some(("me", "p@ss:w0rd "))))),
            (("proxy.lan", "me", ""), Ok(proxy("proxy.lan", 1080, Some(("me", ""))))),
            (("[::1]:1080", "", ""), Ok(proxy("::1", 1080, None))),
            (("[fe80::1]", "", ""), Ok(proxy("fe80::1", 1080, None))),
            (("http://proxy.lan:3128", "", ""), Err(ProxyError::NotSocks5)),
            (("socks4://proxy.lan", "", ""), Err(ProxyError::NotSocks5)),
            ((":1080", "", ""), Err(ProxyError::Host)),
            (("proxy lan", "", ""), Err(ProxyError::Host)),
            (("me:pw@proxy.lan", "", ""), Err(ProxyError::Login)),
            (("proxy.lan/path", "", ""), Err(ProxyError::Host)),
            (("::1", "", ""), Err(ProxyError::Host)),
            (("[::1", "", ""), Err(ProxyError::Host)),
            (("[nope]:1080", "", ""), Err(ProxyError::Host)),
            (("[::1]1080", "", ""), Err(ProxyError::Host)),
            (("", "me", "pw"), Ok(None)),
            ((" ", "", "pw"), Ok(None)),
            (("proxy.lan:0", "", ""), Err(ProxyError::Port)),
            (("proxy.lan:65536", "", ""), Err(ProxyError::Port)),
            (("proxy.lan:", "", ""), Err(ProxyError::Port)),
            (("proxy.lan", "", "pw"), Err(ProxyError::Login)),
        ];
        for ((address, user, password), want) in cases {
            assert_eq!(read(address, user, password), want, "{address:?} {user:?} {password:?}");
        }
        assert_eq!(read("proxy.lan", &"u".repeat(256), ""), Err(ProxyError::Login));
        assert_eq!(read("proxy.lan", "me", &"p".repeat(256)), Err(ProxyError::Login));
        assert_eq!(read(&format!("{}.lan", "h".repeat(252)), "", ""), Err(ProxyError::Host));
    }

    #[test]
    fn a_proxy_reads_back_as_it_is_shown() {
        for p in [proxy("proxy.lan", 1080, None), proxy("::1", 9050, Some(("me", ""))), proxy("10.0.0.1", 1, Some(("me", "p@ss:w0rd")))] {
            let login = p.as_ref().and_then(|p| p.login.clone()).map_or_else(Default::default, |l| (l.user, l.password));
            assert_eq!(read(&proxy_address(p.clone()), &login.0, &login.1), Ok(p.clone()), "{}", proxy_address(p));
        }
        assert_eq!(proxy_address(None), "");
    }

    /// Local addresses go around the proxy, as do `localhost` and `.local` names; every other address and
    /// name goes to it.
    #[test]
    fn local_destinations_skip_the_proxy() {
        let cases = [
            ("127.0.0.1", true),
            ("127.255.0.9", true),
            ("10.1.2.3", true),
            ("172.16.0.1", true),
            ("172.19.0.1", true),
            ("172.31.255.255", true),
            ("192.168.1.5", true),
            ("169.254.10.1", true),
            ("::1", true),
            ("[::1]", true),
            ("[fd12:3456::1]", true),
            ("fc00::1", true),
            ("[fe80::1]", true),
            ("[::ffff:192.168.1.5]", true),
            ("localhost", true),
            ("LocalHost", true),
            ("100.64.0.1", true),
            ("100.127.255.254", true),
            ("nas.local", true),
            ("NAS.Local.", true),
            ("172.15.255.255", false),
            ("172.32.0.1", false),
            ("192.169.0.1", false),
            ("11.0.0.1", false),
            ("100.63.255.255", false),
            ("100.128.0.1", false),
            (".local", false),
            ("nas.local.example", false),
            ("notlocal", false),
            ("8.8.8.8", false),
            ("0.0.0.0", false),
            ("[2001:db8::1]", false),
            ("[::ffff:8.8.8.8]", false),
            ("music.example", false),
            ("localhost.example", false),
            ("10.0.0.1.example", false),
        ];
        for (host, direct) in cases {
            assert_eq!(skips_proxy(host.into()), direct, "{host}");
        }
    }

    #[test]
    fn bypass_rules_name_every_local_network() {
        assert_eq!(
            proxy_bypass_rules(),
            ["localhost", "*.local", "127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10", "169.254.0.0/16", "::1/128", "fc00::/7", "fe80::/10"]
        );
    }
}
