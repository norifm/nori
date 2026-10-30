//! TCP, TLS over it, and UDP for plugins: to the hosts their manifest names, and with `lan` to private
//! addresses (a speaker's protocol, SSDP's multicast). Blocking, on the plugin's own thread.

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use wasmtime::component::Resource;

use crate::lan::{host_of, private, private_addresses, zoned, Multicast};
use crate::runtime::nori::plugin::sockets;
use crate::runtime::{types, Host};

/// The most bytes one read returns.
const READ_MOST: u32 = 64 << 10;
/// How long a TLS handshake may take.
const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);

enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

pub struct Tcp {
    /// None only while TLS takes it over.
    stream: Option<Stream>,
    peer: SocketAddr,
}

impl Tcp {
    fn socket(&self) -> Option<&TcpStream> {
        match self.stream.as_ref()? {
            Stream::Plain(s) => Some(s),
            Stream::Tls(t) => Some(&t.sock),
        }
    }

    /// The plain socket, for the host to write into itself (`audio-out`); None over TLS.
    pub(crate) fn plain(&self) -> Option<&TcpStream> {
        match self.stream.as_ref()? {
            Stream::Plain(s) => Some(s),
            Stream::Tls(_) => None,
        }
    }
}

pub struct Udp {
    socket: UdpSocket,
    /// An IPv6 socket that takes IPv4 too, its addresses mapped; IPv4 alone where the system has no IPv6.
    dual: bool,
    /// Held while it is in a multicast group.
    _joined: Option<Multicast>,
}

pub(crate) fn error(kind: types::ErrorKind, detail: impl Into<String>) -> types::Error {
    types::Error { kind, detail: detail.into() }
}

fn failed(e: std::io::Error) -> types::Error {
    match e.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => error(types::ErrorKind::Timeout, "timed out"),
        _ => error(types::ErrorKind::Network, e.to_string()),
    }
}

fn wait(ms: u32) -> Option<Duration> {
    Some(Duration::from_millis(ms.max(1) as u64))
}

fn table(e: wasmtime::component::ResourceTableError) -> types::Error {
    error(types::ErrorKind::Other, e.to_string())
}

impl Host {
    /// The addresses of `host`:`port` this plugin may reach: any of a host its manifest names, else with
    /// `lan` those of a host on the LAN.
    fn reachable(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, types::Error> {
        if self.manifest.reaches(host) {
            let all: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(failed)?.collect();
            return if all.is_empty() { Err(error(types::ErrorKind::Network, format!("{host} has no address"))) } else { Ok(all) };
        }
        if self.manifest.lan() {
            if let Some(all) = private_addresses(host, port) {
                return Ok(all);
            }
        }
        Err(error(types::ErrorKind::Refused, format!("{host} is neither among its net hosts nor on the LAN it may reach")))
    }

    fn tcp(&mut self, s: &Resource<Tcp>) -> Result<&mut Tcp, types::Error> {
        self.table.get_mut(s).map_err(table)
    }
}

impl sockets::Host for Host {}

impl sockets::HostTcp for Host {
    fn connect(&mut self, host: String, port: u16, timeout_ms: u32) -> Result<Resource<Tcp>, types::Error> {
        let mut last = error(types::ErrorKind::Network, format!("{host} has no address"));
        for address in self.reachable(&host, port)? {
            match self.mark.waiting(|| TcpStream::connect_timeout(&address, wait(timeout_ms).expect("some"))) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return self.table.push(Tcp { stream: Some(Stream::Plain(stream)), peer: address }).map_err(table);
                }
                Err(e) => last = failed(e),
            }
        }
        Err(last)
    }

    fn read(&mut self, s: Resource<Tcp>, max: u32, timeout_ms: u32) -> Result<Vec<u8>, types::Error> {
        let mark = self.mark.clone();
        let tcp = self.tcp(&s)?;
        tcp.socket().ok_or_else(|| error(types::ErrorKind::Network, "closed"))?.set_read_timeout(wait(timeout_ms)).map_err(failed)?;
        let mut got = vec![0; max.min(READ_MOST) as usize];
        let n = mark
            .waiting(|| match tcp.stream.as_mut() {
                Some(Stream::Plain(s)) => s.read(&mut got),
                Some(Stream::Tls(t)) => t.read(&mut got),
                None => Ok(0),
            })
            .map_err(failed)?;
        got.truncate(n);
        Ok(got)
    }

    fn write(&mut self, s: Resource<Tcp>, bytes: Vec<u8>) -> Result<(), types::Error> {
        match self.tcp(&s)?.stream.as_mut() {
            Some(Stream::Plain(s)) => s.write_all(&bytes),
            Some(Stream::Tls(t)) => t.write_all(&bytes).and_then(|_| t.flush()),
            None => Err(ErrorKind::NotConnected.into()),
        }
        .map_err(failed)
    }

    fn tls(&mut self, s: Resource<Tcp>, server_name: String, verify: sockets::TlsVerify) -> Result<(), types::Error> {
        let tcp = self.tcp(&s)?;
        let verifier: Arc<dyn ServerCertVerifier> = match verify {
            sockets::TlsVerify::System => {
                let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
                rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider()).build().map_err(|e| error(types::ErrorKind::Other, e.to_string()))?
            }
            sockets::TlsVerify::Pinned(der) => Arc::new(Accepts { only: Some(der) }),
            sockets::TlsVerify::None if private(tcp.peer.ip()) => Arc::new(Accepts { only: None }),
            sockets::TlsVerify::None => return Err(error(types::ErrorKind::Refused, "TLS without verification only on the LAN")),
        };
        let name = ServerName::try_from(server_name).map_err(|e| error(types::ErrorKind::Refused, e.to_string()))?;
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| error(types::ErrorKind::Other, e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let Some(Stream::Plain(mut sock)) = tcp.stream.take() else { return Err(error(types::ErrorKind::Refused, "already TLS")) };
        let mut conn = ClientConnection::new(Arc::new(config), name).map_err(|e| error(types::ErrorKind::Other, e.to_string()))?;
        let _ = sock.set_read_timeout(Some(HANDSHAKE_WAIT));
        while conn.is_handshaking() {
            if let Err(e) = conn.complete_io(&mut sock) {
                // The socket is no use after a failed handshake: it stays closed.
                return Err(match e.kind() {
                    ErrorKind::InvalidData => error(types::ErrorKind::Refused, e.to_string()),
                    _ => failed(e),
                });
            }
        }
        tcp.stream = Some(Stream::Tls(Box::new(StreamOwned::new(conn, sock))));
        Ok(())
    }

    fn drop(&mut self, s: Resource<Tcp>) -> wasmtime::Result<()> {
        self.table.delete(s)?;
        Ok(())
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A server whose certificate is the one pinned, or any (`none`, on the LAN only). Its handshake's
/// signatures are still checked against that certificate.
#[derive(Debug)]
struct Accepts {
    only: Option<Vec<u8>>,
}

impl ServerCertVerifier for Accepts {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        match &self.only {
            Some(der) if der.as_slice() != end_entity.as_ref() => Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &provider().signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &provider().signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider().signature_verification_algorithms.supported_schemes()
    }
}

/// A UDP socket on `port`, for IPv6 and IPv4 both where the system has IPv6; whether it does.
fn udp_socket(port: u16) -> std::io::Result<(UdpSocket, bool)> {
    use socket2::{Domain, Protocol, Socket, Type};
    let dual = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).and_then(|s| s.set_only_v6(false).map(|()| s));
    let (socket, any) = match dual {
        Ok(s) => (s, IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        Err(_) => (Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?, IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
    };
    // A well-known port (SSDP's, mDNS's) is shared with the system's own listeners.
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::new(any, port).into())?;
    Ok((socket.into(), any.is_ipv6()))
}

impl sockets::HostUdp for Host {
    fn bind(&mut self, port: u16) -> Result<Resource<Udp>, types::Error> {
        let (socket, dual) = udp_socket(port).map_err(failed)?;
        self.table.push(Udp { socket, dual, _joined: None }).map_err(table)
    }

    fn join(&mut self, s: Resource<Udp>, group: String) -> Result<(), types::Error> {
        if !self.manifest.lan() {
            return Err(error(types::ErrorKind::Refused, "multicast needs lan"));
        }
        // An IPv6 group may name the interface it is joined on (`ff02::c%3`); else the system picks.
        let (ip, interface) = match zoned(&group) {
            Some((v6, interface)) => (IpAddr::V6(v6), interface),
            None => (group.parse::<IpAddr>().map_err(|_| error(types::ErrorKind::Refused, format!("{group} is not a group")))?, 0),
        };
        if !ip.is_multicast() || !private(ip) {
            return Err(error(types::ErrorKind::Refused, format!("{group} is not a group on the LAN")));
        }
        let lan = self.lan.clone();
        let udp = self.table.get_mut(&s).map_err(table)?;
        match ip {
            IpAddr::V4(g) => udp.socket.join_multicast_v4(&g, &Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(g) => udp.socket.join_multicast_v6(&g, interface),
        }
        .map_err(failed)?;
        if udp._joined.is_none() {
            udp._joined = Some(lan.multicast());
        }
        Ok(())
    }

    fn send(&mut self, s: Resource<Udp>, host: String, port: u16, bytes: Vec<u8>) -> Result<(), types::Error> {
        let to = *self.reachable(&host, port)?.first().expect("not empty");
        let udp = self.table.get(&s).map_err(table)?;
        let to = match to {
            SocketAddr::V4(v4) if udp.dual => SocketAddr::V6(SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0)),
            to => to,
        };
        udp.socket.send_to(&bytes, to).map(|_| ()).map_err(failed)
    }

    fn receive(&mut self, s: Resource<Udp>, timeout_ms: u32) -> Result<Option<(String, u16, Vec<u8>)>, types::Error> {
        let udp = self.table.get(&s).map_err(table)?;
        udp.socket.set_read_timeout(wait(timeout_ms)).map_err(failed)?;
        let mut got = vec![0; READ_MOST as usize];
        match self.mark.waiting(|| udp.socket.recv_from(&mut got)) {
            Ok((n, from)) => {
                got.truncate(n);
                Ok(Some((host_of(from), from.port(), got)))
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    fn drop(&mut self, s: Resource<Udp>) -> wasmtime::Result<()> {
        self.table.delete(s)?;
        Ok(())
    }
}
