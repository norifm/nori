//! The SOCKS5 handshake (RFC 1928, its login RFC 1929) on a connection to the proxy: the proxy then
//! carries the connection to a host it looks up itself.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::IpAddr;

use nori_core::transport::FailureKind;

const VERSION: u8 = 5;
const NO_LOGIN: u8 = 0;
const LOGIN: u8 = 2;
const LOGIN_VERSION: u8 = 1;
const CONNECT: u8 = 1;
const IPV4: u8 = 1;
const NAME: u8 = 3;
const IPV6: u8 = 4;

/// Asks the proxy at the other end of `s` to connect it to `host` (a name, or an IP address, an IPv6
/// one in brackets or not) at `port`, signing in with `login` (user, password) when the proxy asks.
pub(crate) fn connect(s: &mut (impl Read + Write), host: &str, port: u16, login: Option<(&[u8], &[u8])>) -> io::Result<()> {
    let methods: &[u8] = if login.is_some() { &[NO_LOGIN, LOGIN] } else { &[NO_LOGIN] };
    s.write_all(&[&[VERSION, methods.len() as u8][..], methods].concat())?;
    let [version, method] = read(s)?;
    socks5(version)?;
    match (method, login) {
        (NO_LOGIN, _) => {}
        (LOGIN, Some((user, password))) => sign_in(s, user, password)?,
        _ => return Err(failed(FailureKind::ProxyLogin, "the proxy asks for a login it was not given")),
    }
    let mut ask = vec![VERSION, CONNECT, 0];
    match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ask.extend([&[IPV4][..], &ip.octets()].concat()),
        Ok(IpAddr::V6(ip)) => ask.extend([&[IPV6][..], &ip.octets()].concat()),
        Err(_) => ask.extend([&[NAME, length(host.as_bytes())?][..], host.as_bytes()].concat()),
    }
    ask.extend(port.to_be_bytes());
    s.write_all(&ask)?;
    let [version, reply, _, bound] = read(s)?;
    socks5(version)?;
    if reply != 0 {
        return Err(failed(FailureKind::ProxyNoRoute, format!("the proxy did not connect (SOCKS5 reply {reply})")));
    }
    // The address the proxy connected from, and its port: not needed.
    let bound = match bound {
        IPV4 => 4,
        IPV6 => 16,
        NAME => read::<1>(s)?[0] as u64,
        _ => return Err(not_socks5()),
    };
    io::copy(&mut Read::by_ref(s).take(bound + 2), &mut io::sink())?;
    Ok(())
}

fn sign_in(s: &mut (impl Read + Write), user: &[u8], password: &[u8]) -> io::Result<()> {
    s.write_all(&[&[LOGIN_VERSION, length(user)?][..], user, &[length(password)?], password].concat())?;
    // The first byte is the login's version: 1, or 5 from some proxies.
    let [_, status] = read(s)?;
    if status != 0 {
        return Err(failed(FailureKind::ProxyLogin, "the proxy refused the login"));
    }
    Ok(())
}

fn read<const N: usize>(s: &mut impl Read) -> io::Result<[u8; N]> {
    let mut b = [0; N];
    s.read_exact(&mut b)?;
    Ok(b)
}

fn socks5(version: u8) -> io::Result<()> {
    if version != VERSION {
        return Err(not_socks5());
    }
    Ok(())
}

fn not_socks5() -> io::Error {
    failed(FailureKind::ProxyUnreachable, "the proxy's answer is not SOCKS5")
}

/// A name's, user's or password's length as SOCKS5 sends it: one byte.
fn length(part: &[u8]) -> io::Result<u8> {
    u8::try_from(part.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "longer than SOCKS5 carries"))
}

/// A failure of the proxy, as its kind, inside the I/O failure the connector returns ([`crate::failure`]
/// reads it). The detail is for the log.
#[derive(Debug)]
pub(crate) struct Failed {
    pub(crate) kind: FailureKind,
    detail: String,
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for Failed {}

pub(crate) fn failed(kind: FailureKind, detail: impl fmt::Display) -> io::Error {
    io::Error::other(Failed { kind, detail: detail.to_string() })
}
