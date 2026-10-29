//! TCP for plugins, only to the hosts their manifest names: a protocol of a service's own that isn't
//! HTTP (Spotify's access point). Blocking, on the plugin's own thread.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use wasmtime::component::Resource;

use crate::runtime::nori::plugin::sockets;
use crate::runtime::{types, Host};

/// The most bytes one read returns.
const READ_MOST: u32 = 64 << 10;

pub struct Tcp(TcpStream);

/// Never made: UDP isn't linked yet.
pub enum Udp {}

fn error(kind: types::ErrorKind, detail: impl Into<String>) -> types::Error {
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

impl Host {
    fn tcp(&mut self, s: &Resource<Tcp>) -> Result<&mut TcpStream, types::Error> {
        self.table.get_mut(s).map(|t| &mut t.0).map_err(|e| error(types::ErrorKind::Other, e.to_string()))
    }
}

impl sockets::Host for Host {}

impl sockets::HostTcp for Host {
    fn connect(&mut self, host: String, port: u16, timeout_ms: u32) -> Result<Resource<Tcp>, types::Error> {
        if !self.manifest.reaches(&host) {
            return Err(error(types::ErrorKind::Refused, format!("{host} is not among its net hosts")));
        }
        let mut last = error(types::ErrorKind::Network, format!("{host} has no address"));
        for address in (host.as_str(), port).to_socket_addrs().map_err(failed)? {
            match TcpStream::connect_timeout(&address, wait(timeout_ms).expect("some")) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return self.table.push(Tcp(stream)).map_err(|e| error(types::ErrorKind::Other, e.to_string()));
                }
                Err(e) => last = failed(e),
            }
        }
        Err(last)
    }

    fn read(&mut self, s: Resource<Tcp>, max: u32, timeout_ms: u32) -> Result<Vec<u8>, types::Error> {
        let stream = self.tcp(&s)?;
        stream.set_read_timeout(wait(timeout_ms)).map_err(failed)?;
        let mut got = vec![0; max.min(READ_MOST) as usize];
        let n = stream.read(&mut got).map_err(failed)?;
        got.truncate(n);
        Ok(got)
    }

    fn write(&mut self, s: Resource<Tcp>, bytes: Vec<u8>) -> Result<(), types::Error> {
        self.tcp(&s)?.write_all(&bytes).map_err(failed)
    }

    fn tls(&mut self, _: Resource<Tcp>, _: String, _: sockets::TlsVerify) -> Result<(), types::Error> {
        Err(error(types::ErrorKind::Unsupported, "TLS over a plugin's socket isn't linked yet"))
    }

    fn drop(&mut self, s: Resource<Tcp>) -> wasmtime::Result<()> {
        self.table.delete(s)?;
        Ok(())
    }
}

impl sockets::HostUdp for Host {
    fn bind(&mut self, _: u16) -> Result<Resource<Udp>, types::Error> {
        Err(error(types::ErrorKind::Unsupported, "UDP for plugins isn't linked yet"))
    }

    fn join(&mut self, s: Resource<Udp>, _: String) -> Result<(), types::Error> {
        match *self.table.get(&s).map_err(|e| error(types::ErrorKind::Other, e.to_string()))? {}
    }

    fn send(&mut self, s: Resource<Udp>, _: String, _: u16, _: Vec<u8>) -> Result<(), types::Error> {
        match *self.table.get(&s).map_err(|e| error(types::ErrorKind::Other, e.to_string()))? {}
    }

    fn receive(&mut self, s: Resource<Udp>, _: u32) -> Result<Option<(String, u16, Vec<u8>)>, types::Error> {
        match *self.table.get(&s).map_err(|e| error(types::ErrorKind::Other, e.to_string()))? {}
    }

    fn drop(&mut self, s: Resource<Udp>) -> wasmtime::Result<()> {
        self.table.delete(s)?;
        Ok(())
    }
}
