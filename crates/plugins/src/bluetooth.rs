//! Bluetooth for plugins, only to the devices their manifest's filter matches, over the platform's own
//! ([`Bluetooth`]: Android's `BluetoothSocket` and `BluetoothGatt`, BlueZ on Linux, IOBluetooth and
//! CoreBluetooth on macOS). Calls block on the plugin's own thread; what a device sends comes back
//! through [`crate::Plugins::bluetooth_heard`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use wasmtime::component::Resource;

use crate::runtime::nori::plugin::bluetooth as w;
use crate::runtime::{types, Host};

/// A Bluetooth device the platform has connected.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BtDevice {
    /// The platform's id of it (its address).
    pub id: String,
    pub name: String,
    /// The service UUIDs it offers, lowercase.
    pub services: Vec<String>,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
}

/// Why a Bluetooth call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum BtFailure {
    /// This platform, or this device, has no such thing.
    Unsupported,
    NotFound,
    /// Not allowed (Android's permission not granted).
    Refused,
    Timeout,
    Failed { detail: String },
}

impl std::fmt::Display for BtFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BtFailure {}

/// The platform's Bluetooth. A link is a channel the host numbered; what comes on it the platform
/// hands to [`crate::Plugins::bluetooth_heard`], and a link it loses to [`crate::Plugins::bluetooth_lost`].
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait Bluetooth: Send + Sync {
    /// Reports devices connecting and going ([`crate::Plugins::bluetooth_connected`],
    /// [`crate::Plugins::bluetooth_disconnected`]) while on; off, it holds no listener.
    fn watch(&self, on: bool);
    fn connected(&self) -> Vec<BtDevice>;
    /// Opens `link`, an RFCOMM channel to `device`'s `service`.
    fn rfcomm(&self, link: u64, device: String, service: String) -> Result<(), BtFailure>;
    fn rfcomm_write(&self, link: u64, bytes: Vec<u8>) -> Result<(), BtFailure>;
    /// Opens `link`, a GATT connection to `device` with its services discovered.
    fn gatt(&self, link: u64, device: String) -> Result<(), BtFailure>;
    fn gatt_mtu(&self, link: u64) -> u16;
    fn gatt_services(&self, link: u64) -> Vec<String>;
    fn gatt_read(&self, link: u64, service: String, characteristic: String) -> Result<Vec<u8>, BtFailure>;
    fn gatt_write(&self, link: u64, service: String, characteristic: String, bytes: Vec<u8>, with_response: bool) -> Result<(), BtFailure>;
    fn gatt_notify(&self, link: u64, service: String, characteristic: String, on: bool) -> Result<(), BtFailure>;
    fn close(&self, link: u64);
}

/// The most bytes an RFCOMM channel keeps for the plugin; older ones go first.
const KEPT_MOST: usize = 64 << 10;
/// The longest one RFCOMM read waits.
const READ_LONGEST: Duration = Duration::from_secs(30);

pub(crate) enum LinkKind {
    Rfcomm { service: String },
    Gatt,
}

struct LinkState {
    plugin: String,
    device: String,
    kind: LinkKind,
    /// What came on an RFCOMM channel that no read has taken yet.
    waiting: VecDeque<u8>,
    lost: bool,
}

#[derive(Default)]
struct Links {
    next: u64,
    open: HashMap<u64, LinkState>,
}

/// What came on a link, for its plugin.
pub(crate) struct Heard {
    pub plugin: String,
    pub device: String,
    /// An RFCOMM channel's service, whose bytes are taken when the plugin hears them.
    pub rfcomm: Option<String>,
}

/// What the host knows of Bluetooth: the platform, the devices connected and the links plugins hold.
#[derive(Default)]
pub(crate) struct Radio {
    platform: Mutex<Option<Arc<dyn Bluetooth>>>,
    connected: Mutex<Vec<BtDevice>>,
    links: Mutex<Links>,
    came: Condvar,
}

impl Radio {
    pub(crate) fn platform(&self) -> Option<Arc<dyn Bluetooth>> {
        self.platform.lock().expect("not poisoned").clone()
    }

    pub(crate) fn set_platform(&self, p: Arc<dyn Bluetooth>) {
        *self.platform.lock().expect("not poisoned") = Some(p);
    }

    pub(crate) fn connected(&self) -> Vec<BtDevice> {
        self.connected.lock().expect("not poisoned").clone()
    }

    pub(crate) fn set_connected(&self, devices: Vec<BtDevice>) {
        *self.connected.lock().expect("not poisoned") = devices;
    }

    /// Adds or replaces `d`; whether it is new.
    pub(crate) fn arrived(&self, d: BtDevice) -> bool {
        let mut c = self.connected.lock().expect("not poisoned");
        match c.iter_mut().find(|x| x.id == d.id) {
            Some(x) => {
                *x = d;
                false
            }
            None => {
                c.push(d);
                true
            }
        }
    }

    /// Device `id` went: its links are lost. Whether it was connected.
    pub(crate) fn went(&self, id: &str) -> bool {
        let was = {
            let mut c = self.connected.lock().expect("not poisoned");
            let n = c.len();
            c.retain(|d| d.id != id);
            c.len() != n
        };
        let mut links = self.links.lock().expect("not poisoned");
        links.open.values_mut().filter(|l| l.device == id).for_each(|l| l.lost = true);
        self.came.notify_all();
        was
    }

    fn open(&self, plugin: &str, device: &str, kind: LinkKind) -> u64 {
        let mut links = self.links.lock().expect("not poisoned");
        links.next += 1;
        let id = links.next;
        links.open.insert(id, LinkState { plugin: plugin.into(), device: device.into(), kind, waiting: VecDeque::new(), lost: false });
        id
    }

    fn close(&self, link: u64) {
        let had = self.links.lock().expect("not poisoned").open.remove(&link).is_some();
        if let (true, Some(p)) = (had, self.platform()) {
            p.close(link);
        }
    }

    /// `bytes` came on `link`: an RFCOMM channel keeps them for a read. Whose link it is.
    pub(crate) fn heard(&self, link: u64, bytes: &[u8]) -> Option<Heard> {
        let mut links = self.links.lock().expect("not poisoned");
        let l = links.open.get_mut(&link)?;
        let rfcomm = match &l.kind {
            LinkKind::Rfcomm { service } => {
                l.waiting.extend(bytes);
                let over = l.waiting.len().saturating_sub(KEPT_MOST);
                l.waiting.drain(..over);
                self.came.notify_all();
                Some(service.clone())
            }
            LinkKind::Gatt => None,
        };
        Some(Heard { plugin: l.plugin.clone(), device: l.device.clone(), rfcomm })
    }

    /// What `link` has that no read took.
    pub(crate) fn take(&self, link: u64) -> Vec<u8> {
        let mut links = self.links.lock().expect("not poisoned");
        links.open.get_mut(&link).map(|l| l.waiting.drain(..).collect()).unwrap_or_default()
    }

    pub(crate) fn lost(&self, link: u64) {
        if let Some(l) = self.links.lock().expect("not poisoned").open.get_mut(&link) {
            l.lost = true;
        }
        self.came.notify_all();
    }

    /// Up to `max` bytes of `link`, waiting at most `wait` for any; empty once it is lost.
    fn read(&self, link: u64, max: usize, wait: Duration) -> Result<Vec<u8>, types::Error> {
        let until = Instant::now() + wait.min(READ_LONGEST);
        let mut links = self.links.lock().expect("not poisoned");
        loop {
            let l = links.open.get_mut(&link).ok_or_else(|| error(types::ErrorKind::Other, "no such channel"))?;
            if !l.waiting.is_empty() {
                let n = max.min(l.waiting.len());
                return Ok(l.waiting.drain(..n).collect());
            }
            if l.lost {
                return Ok(Vec::new());
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(error(types::ErrorKind::Timeout, "timed out"));
            }
            links = self.came.wait_timeout(links, left).expect("not poisoned").0;
        }
    }
}

/// A channel a plugin holds; it closes when the plugin drops it or its instance goes.
pub struct Link {
    id: u64,
    radio: Arc<Radio>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.radio.close(self.id);
    }
}

pub struct Rfcomm(Link);
pub struct Gatt(Link);

fn error(kind: types::ErrorKind, detail: impl Into<String>) -> types::Error {
    types::Error { kind, detail: detail.into() }
}

fn failed(f: BtFailure) -> types::Error {
    let kind = match f {
        BtFailure::Unsupported => types::ErrorKind::Unsupported,
        BtFailure::NotFound => types::ErrorKind::NotFound,
        BtFailure::Refused => types::ErrorKind::Refused,
        BtFailure::Timeout => types::ErrorKind::Timeout,
        BtFailure::Failed { .. } => types::ErrorKind::Network,
    };
    error(kind, f.to_string())
}

fn device_out(d: BtDevice) -> w::Device {
    w::Device { id: d.id, name: d.name, services: d.services, vendor: d.vendor, product: d.product }
}

impl Host {
    /// The platform, once `device` is connected and among those the plugin reaches.
    fn reach(&self, device: &str) -> Result<Arc<dyn Bluetooth>, types::Error> {
        let platform = self.radio.platform().ok_or_else(|| error(types::ErrorKind::Unsupported, "no Bluetooth here"))?;
        if !self.radio.connected().iter().any(|d| d.id == device && self.manifest.reaches_device(d)) {
            return Err(error(types::ErrorKind::NotFound, format!("{device} is not a connected device it reaches")));
        }
        Ok(platform)
    }

    fn link(&mut self, device: String, kind: LinkKind, open: impl FnOnce(&dyn Bluetooth, u64) -> Result<(), BtFailure>) -> Result<Link, types::Error> {
        let platform = self.reach(&device)?;
        let link = Link { id: self.radio.open(&self.id, &device, kind), radio: self.radio.clone() };
        open(&*platform, link.id).map_err(failed)?;
        Ok(link)
    }

    fn platform(&self) -> Result<Arc<dyn Bluetooth>, types::Error> {
        self.radio.platform().ok_or_else(|| error(types::ErrorKind::Unsupported, "no Bluetooth here"))
    }

    fn rfcomm_id(&self, r: &Resource<Rfcomm>) -> Result<u64, types::Error> {
        self.table.get(r).map(|r| r.0.id).map_err(|e| error(types::ErrorKind::Other, e.to_string()))
    }

    fn gatt_id(&self, g: &Resource<Gatt>) -> Result<u64, types::Error> {
        self.table.get(g).map(|g| g.0.id).map_err(|e| error(types::ErrorKind::Other, e.to_string()))
    }
}

impl w::Host for Host {
    fn connected(&mut self) -> Vec<w::Device> {
        self.radio.connected().into_iter().filter(|d| self.manifest.reaches_device(d)).map(device_out).collect()
    }
}

impl w::HostRfcomm for Host {
    fn open(&mut self, device: String, service: String) -> Result<Resource<Rfcomm>, types::Error> {
        let to = (device.clone(), service.to_ascii_lowercase());
        let link = self.link(device, LinkKind::Rfcomm { service: to.1.clone() }, |p, id| p.rfcomm(id, to.0, to.1))?;
        self.table.push(Rfcomm(link)).map_err(|e| error(types::ErrorKind::Other, e.to_string()))
    }

    fn read(&mut self, r: Resource<Rfcomm>, max: u32, timeout_ms: u32) -> Result<Vec<u8>, types::Error> {
        let id = self.rfcomm_id(&r)?;
        self.radio.read(id, max as usize, Duration::from_millis(timeout_ms as u64))
    }

    fn write(&mut self, r: Resource<Rfcomm>, bytes: Vec<u8>) -> Result<(), types::Error> {
        let id = self.rfcomm_id(&r)?;
        self.platform()?.rfcomm_write(id, bytes).map_err(failed)
    }

    fn drop(&mut self, r: Resource<Rfcomm>) -> wasmtime::Result<()> {
        self.table.delete(r)?;
        Ok(())
    }
}

impl w::HostGatt for Host {
    fn open(&mut self, device: String) -> Result<Resource<Gatt>, types::Error> {
        let to = device.clone();
        let link = self.link(device, LinkKind::Gatt, |p, id| p.gatt(id, to))?;
        self.table.push(Gatt(link)).map_err(|e| error(types::ErrorKind::Other, e.to_string()))
    }

    fn mtu(&mut self, g: Resource<Gatt>) -> u16 {
        match (self.gatt_id(&g), self.platform()) {
            (Ok(id), Ok(p)) => p.gatt_mtu(id),
            _ => 23,
        }
    }

    fn services(&mut self, g: Resource<Gatt>) -> Vec<String> {
        match (self.gatt_id(&g), self.platform()) {
            (Ok(id), Ok(p)) => p.gatt_services(id),
            _ => Vec::new(),
        }
    }

    fn read(&mut self, g: Resource<Gatt>, service: String, characteristic: String) -> Result<Vec<u8>, types::Error> {
        let id = self.gatt_id(&g)?;
        self.platform()?.gatt_read(id, service, characteristic).map_err(failed)
    }

    fn write(&mut self, g: Resource<Gatt>, service: String, characteristic: String, bytes: Vec<u8>, with_response: bool) -> Result<(), types::Error> {
        let id = self.gatt_id(&g)?;
        self.platform()?.gatt_write(id, service, characteristic, bytes, with_response).map_err(failed)
    }

    fn notify(&mut self, g: Resource<Gatt>, service: String, characteristic: String, on: bool) -> Result<(), types::Error> {
        let id = self.gatt_id(&g)?;
        self.platform()?.gatt_notify(id, service, characteristic, on).map_err(failed)
    }

    fn drop(&mut self, g: Resource<Gatt>) -> wasmtime::Result<()> {
        self.table.delete(g)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rfcomm_read_takes_what_came_and_ends_once_lost() {
        let radio = Radio::default();
        let link = radio.open("p", "d", LinkKind::Rfcomm { service: "s".into() });
        assert_eq!(radio.read(link, 8, Duration::ZERO).map_err(|e| e.kind), Err(types::ErrorKind::Timeout));
        radio.heard(link, b"hello");
        assert_eq!(radio.read(link, 3, Duration::ZERO).unwrap(), b"hel");
        assert_eq!(radio.take(link), b"lo");
        radio.heard(link, &vec![1; KEPT_MOST + 10]);
        assert_eq!(radio.take(link).len(), KEPT_MOST, "only the newest are kept");
        radio.lost(link);
        assert_eq!(radio.read(link, 8, Duration::from_secs(1)).unwrap(), b"", "the end of the channel");
    }
}
