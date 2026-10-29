//! Plugins' Bluetooth on Linux: BlueZ over the system bus (zbus, as the media controls are), made at the
//! first watch and let go once nothing is watched. RFCOMM channels come from a client profile BlueZ
//! connects (`Profile1.NewConnection` hands over the socket), read on a thread of their own; GATT
//! notifications are the characteristic's `PropertiesChanged`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::Duration;

use futures_lite::future::block_on;
use futures_lite::StreamExt;
use nori_plugins::{Bluetooth, BtDevice, BtFailure, Plugins};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const BLUEZ: &str = "org.bluez";
const DEVICE: &str = "org.bluez.Device1";
const CHARACTERISTIC: &str = "org.bluez.GattCharacteristic1";
const CHANGED: &str = "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'";
/// The longest a profile or a device's services take to come.
const SLOWEST: Duration = Duration::from_secs(10);

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

fn text(p: &Props, key: &str) -> Option<String> {
    match p.get(key).map(|v| &**v) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn flag(p: &Props, key: &str) -> bool {
    matches!(p.get(key).map(|v| &**v), Some(Value::Bool(true)))
}

fn path(p: &Props, key: &str) -> Option<String> {
    match p.get(key).map(|v| &**v) {
        Some(Value::ObjectPath(o)) => Some(o.to_string()),
        _ => None,
    }
}

fn strings(p: &Props, key: &str) -> Vec<String> {
    match p.get(key).map(|v| &**v) {
        Some(Value::Array(a)) => a.inner().iter().filter_map(|v| if let Value::Str(s) = v { Some(s.to_lowercase()) } else { None }).collect(),
        _ => Vec::new(),
    }
}

fn bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Array(a) => a.inner().iter().filter_map(|v| if let Value::U8(b) = v { Some(*b) } else { None }).collect(),
        _ => Vec::new(),
    }
}

/// The vendor and product in a modalias (`usb:v054Cp0CD3d0100`, `bluetooth:v004Cp200Ad...`).
fn modalias(m: &str) -> (Option<u16>, Option<u16>) {
    let (_, ids) = m.split_once(':').unwrap_or_default();
    let hex = |tag: char| ids.find(tag).and_then(|at| ids.get(at + 1..at + 5)).and_then(|h| u16::from_str_radix(h, 16).ok());
    (hex('v'), hex('p'))
}

fn device(p: &Props) -> Option<BtDevice> {
    let (vendor, product) = text(p, "Modalias").map_or((None, None), |m| modalias(&m));
    Some(BtDevice { id: text(p, "Address")?, name: text(p, "Alias").or_else(|| text(p, "Name")).unwrap_or_default(), services: strings(p, "UUIDs"), vendor, product })
}

fn failed(e: zbus::Error) -> BtFailure {
    let name = match &e {
        zbus::Error::MethodError(name, _, _) => name.as_str().to_string(),
        _ => String::new(),
    };
    match name.rsplit('.').next().unwrap_or_default() {
        "NotSupported" | "NotAvailable" | "ServiceUnknown" => BtFailure::Unsupported,
        "DoesNotExist" | "UnknownObject" | "NotConnected" => BtFailure::NotFound,
        "NotAuthorized" | "NotPermitted" | "AccessDenied" => BtFailure::Refused,
        "Timeout" | "NoReply" => BtFailure::Timeout,
        _ => BtFailure::Failed { detail: e.to_string() },
    }
}

/// The client profile BlueZ hands an RFCOMM channel's socket to.
struct Profile(Mutex<Option<mpsc::Sender<OwnedFd>>>);

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    fn release(&self) {}

    fn new_connection(&self, _device: OwnedObjectPath, fd: zbus::zvariant::OwnedFd, _props: Props) {
        if let Some(to) = self.0.lock().expect("not poisoned").take() {
            let _ = to.send(fd.into());
        }
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

enum Link {
    Rfcomm { socket: std::fs::File, profile: String },
    Gatt {
        services: Vec<String>,
        /// By service and characteristic UUID.
        characteristics: HashMap<(String, String), String>,
        mtu: u16,
        notifying: Vec<String>,
    },
}

/// The system bus, while it is needed: its executor runs on the "nori-bluez" thread.
struct Bus {
    conn: Connection,
    /// Reports devices connecting and going, and characteristics' notifications.
    watch: Option<zbus::Task<()>>,
}

#[derive(Default)]
struct State {
    bus: Option<Bus>,
    links: HashMap<u64, Link>,
    /// Device object paths by address, for those reported connected.
    devices: HashMap<String, String>,
}

/// BlueZ as the plugin host's Bluetooth.
pub struct Bluez {
    plugins: Weak<Plugins>,
    state: Arc<Mutex<State>>,
}

/// The system bus's socket, connected here (as the media controls connect the session bus's) so no pool
/// thread of zbus's stays.
fn connect() -> zbus::Result<Connection> {
    let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").ok();
    let socket = address.as_deref().and_then(|a| a.strip_prefix("unix:path=")).map(|p| p.split(',').next().unwrap_or(p)).unwrap_or("/run/dbus/system_bus_socket").to_string();
    let stream = std::os::unix::net::UnixStream::connect(socket)?;
    let builder = zbus::connection::Builder::async_io_unix_stream(stream).internal_executor(false);
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("nori-bluez".into())
        .spawn(move || {
            block_on(async move {
                let conn = match builder.build().await {
                    Ok(c) => c,
                    Err(e) => return drop(tx.send(Err(e))),
                };
                let executor = conn.executor().clone();
                let _ = tx.send(Ok(conn));
                while !executor.is_empty() {
                    executor.tick().await;
                }
            })
        })
        ?;
    rx.recv().map_err(|_| zbus::Error::Failure("the bus thread ended".into()))?
}

async fn call<R: serde::de::DeserializeOwned + zbus::zvariant::Type>(conn: &Connection, path: &str, iface: &str, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<R> {
    conn.call_method(Some(BLUEZ), path, Some(iface), method, body).await?.body().deserialize()
}

async fn objects(conn: &Connection) -> zbus::Result<Objects> {
    call(conn, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &()).await
}

impl Bluez {
    pub fn new(plugins: Weak<Plugins>) -> Bluez {
        Bluez { plugins, state: Arc::default() }
    }

    /// The bus, connected at first need.
    fn conn(&self) -> Result<Connection, BtFailure> {
        let mut s = self.state.lock().expect("not poisoned");
        if let Some(b) = &s.bus {
            return Ok(b.conn.clone());
        }
        let conn = connect().map_err(failed)?;
        s.bus = Some(Bus { conn: conn.clone(), watch: None });
        Ok(conn)
    }

    /// The object path of connected device `address`.
    fn device_path(&self, conn: &Connection, address: &str) -> Result<String, BtFailure> {
        let all = block_on(objects(conn)).map_err(failed)?;
        all.into_iter()
            .find(|(_, i)| i.get(DEVICE).is_some_and(|p| flag(p, "Connected") && text(p, "Address").as_deref() == Some(address)))
            .map(|(p, _)| p.to_string())
            .ok_or(BtFailure::NotFound)
    }

    fn characteristic(&self, link: u64, service: &str, characteristic: &str) -> Result<String, BtFailure> {
        match self.state.lock().expect("not poisoned").links.get(&link) {
            Some(Link::Gatt { characteristics, .. }) => characteristics.get(&(service.to_lowercase(), characteristic.to_lowercase())).cloned().ok_or(BtFailure::NotFound),
            _ => Err(BtFailure::NotFound),
        }
    }
}

/// What the watch does with one `PropertiesChanged`.
async fn changed(conn: &Connection, state: &Mutex<State>, plugins: &Plugins, msg: zbus::Message) {
    let Some(at) = msg.header().path().map(|p| p.to_string()) else { return };
    let Ok((iface, props, _)) = msg.body().deserialize::<(String, Props, Vec<String>)>() else { return };
    match iface.as_str() {
        DEVICE if props.contains_key("Connected") || props.contains_key("UUIDs") || props.contains_key("Alias") => {
            let Ok(all) = call::<Props>(conn, &at, "org.freedesktop.DBus.Properties", "GetAll", &(DEVICE,)).await else { return };
            let Some(d) = device(&all) else { return };
            if flag(&all, "Connected") {
                state.lock().expect("not poisoned").devices.insert(d.id.clone(), at);
                plugins.bluetooth_connected(d);
            } else if state.lock().expect("not poisoned").devices.remove(&d.id).is_some() {
                plugins.bluetooth_disconnected(d.id);
            }
        }
        CHARACTERISTIC => {
            let Some(value) = props.get("Value") else { return };
            let heard = {
                let s = state.lock().expect("not poisoned");
                s.links.iter().find_map(|(id, l)| match l {
                    Link::Gatt { characteristics, notifying, .. } if notifying.contains(&at) => characteristics.iter().find(|(_, p)| **p == at).map(|((_, c), _)| (*id, c.clone())),
                    _ => None,
                })
            };
            if let Some((link, uuid)) = heard {
                plugins.bluetooth_heard(link, uuid, bytes(value));
            }
        }
        _ => {}
    }
}

impl Bluetooth for Bluez {
    fn watch(&self, on: bool) {
        if !on {
            let mut s = self.state.lock().expect("not poisoned");
            s.devices.clear();
            if s.links.is_empty() {
                s.bus = None;
            } else if let Some(b) = s.bus.as_mut() {
                b.watch = None;
            }
            return;
        }
        let Ok(conn) = self.conn() else { return };
        let (state, plugins, c) = (self.state.clone(), self.plugins.clone(), conn.clone());
        let task = conn.executor().spawn(
            async move {
                let Ok(mut stream) = zbus::MessageStream::for_match_rule(CHANGED, &c, None).await else { return };
                while let Some(Ok(msg)) = stream.next().await {
                    let Some(plugins) = plugins.upgrade() else { return };
                    changed(&c, &state, &plugins, msg).await;
                }
            },
            "nori-bluez-watch",
        );
        if let Some(b) = self.state.lock().expect("not poisoned").bus.as_mut() {
            b.watch = Some(task);
        }
    }

    fn connected(&self) -> Vec<BtDevice> {
        let Ok(conn) = self.conn() else { return Vec::new() };
        let Ok(all) = block_on(objects(&conn)) else { return Vec::new() };
        let mut s = self.state.lock().expect("not poisoned");
        all.into_iter()
            .filter_map(|(path, i)| {
                let p = i.get(DEVICE).filter(|p| flag(p, "Connected"))?;
                let d = device(p)?;
                s.devices.insert(d.id.clone(), path.to_string());
                Some(d)
            })
            .collect()
    }

    fn rfcomm(&self, link: u64, device: String, service: String) -> Result<(), BtFailure> {
        let conn = self.conn()?;
        let at = self.device_path(&conn, &device)?;
        let profile = format!("/fm/nori/bluez/profile{link}");
        let (tx, rx) = mpsc::channel();
        block_on(conn.object_server().at(profile.as_str(), Profile(Mutex::new(Some(tx))))).map_err(failed)?;
        let options: HashMap<&str, Value> = HashMap::from([("Role", Value::from("client")), ("Name", Value::from("nori"))]);
        let socket = block_on(async {
            let path = zbus::zvariant::ObjectPath::try_from(profile.as_str())?;
            call::<()>(&conn, "/org/bluez", "org.bluez.ProfileManager1", "RegisterProfile", &(path, service.as_str(), options)).await?;
            call::<()>(&conn, &at, DEVICE, "ConnectProfile", &(service.as_str(),)).await
        })
        .map_err(failed)
        .and_then(|_| rx.recv_timeout(SLOWEST).map_err(|_| BtFailure::Timeout));
        let socket = match socket {
            Ok(fd) => std::fs::File::from(fd),
            Err(e) => {
                self.unregister(&conn, &profile);
                return Err(e);
            }
        };
        let mut reading = socket.try_clone().map_err(|e| BtFailure::Failed { detail: e.to_string() })?;
        let plugins = self.plugins.clone();
        std::thread::Builder::new()
            .name(format!("nori-rfcomm-{link}"))
            .spawn(move || {
                let mut buf = vec![0u8; 4096];
                loop {
                    let n = reading.read(&mut buf).unwrap_or(0);
                    let Some(p) = plugins.upgrade() else { return };
                    if n == 0 {
                        return p.bluetooth_lost(link);
                    }
                    p.bluetooth_heard(link, String::new(), buf[..n].to_vec());
                }
            })
            .map_err(|e| BtFailure::Failed { detail: e.to_string() })?;
        self.state.lock().expect("not poisoned").links.insert(link, Link::Rfcomm { socket, profile });
        Ok(())
    }

    fn rfcomm_write(&self, link: u64, bytes: Vec<u8>) -> Result<(), BtFailure> {
        let socket = match self.state.lock().expect("not poisoned").links.get(&link) {
            Some(Link::Rfcomm { socket, .. }) => socket.try_clone().map_err(|e| BtFailure::Failed { detail: e.to_string() })?,
            _ => return Err(BtFailure::NotFound),
        };
        (&socket).write_all(&bytes).map_err(|e| BtFailure::Failed { detail: e.to_string() })
    }

    fn gatt(&self, link: u64, device: String) -> Result<(), BtFailure> {
        let conn = self.conn()?;
        let at = self.device_path(&conn, &device)?;
        block_on(async {
            let resolved = |p: &Props| flag(p, "ServicesResolved");
            let all: Props = call(&conn, &at, "org.freedesktop.DBus.Properties", "GetAll", &(DEVICE,)).await.map_err(failed)?;
            if !resolved(&all) {
                // Its services are still being discovered: their `ServicesResolved`, or a timeout.
                let rule = format!("{CHANGED},path='{at}'");
                let mut stream = zbus::MessageStream::for_match_rule(rule.as_str(), &conn, None).await.map_err(failed)?;
                let came = async {
                    while let Some(Ok(m)) = stream.next().await {
                        if m.body().deserialize::<(String, Props, Vec<String>)>().is_ok_and(|(_, p, _)| resolved(&p)) {
                            return true;
                        }
                    }
                    false
                };
                let late = async {
                    async_io::Timer::after(SLOWEST).await;
                    false
                };
                if !futures_lite::future::or(came, late).await {
                    return Err(BtFailure::Timeout);
                }
            }
            Ok(())
        })?;
        let all = block_on(objects(&conn)).map_err(failed)?;
        let under = format!("{at}/");
        let services: HashMap<String, String> = all.iter().filter(|(p, _)| p.starts_with(&under)).filter_map(|(p, i)| Some((p.to_string(), text(i.get("org.bluez.GattService1")?, "UUID")?.to_lowercase()))).collect();
        let mut characteristics = HashMap::new();
        let mut mtu = 23;
        for (p, i) in &all {
            let Some(c) = i.get(CHARACTERISTIC) else { continue };
            let (Some(uuid), Some(service)) = (text(c, "UUID"), path(c, "Service").and_then(|s| services.get(&s))) else { continue };
            if let Some(Value::U16(m)) = c.get("MTU").map(|v| &**v) {
                mtu = mtu.max(*m);
            }
            characteristics.insert((service.clone(), uuid.to_lowercase()), p.to_string());
        }
        let mut uuids: Vec<String> = services.into_values().collect();
        uuids.sort();
        uuids.dedup();
        self.state.lock().expect("not poisoned").links.insert(link, Link::Gatt { services: uuids, characteristics, mtu, notifying: Vec::new() });
        Ok(())
    }

    fn gatt_mtu(&self, link: u64) -> u16 {
        match self.state.lock().expect("not poisoned").links.get(&link) {
            Some(Link::Gatt { mtu, .. }) => *mtu,
            _ => 23,
        }
    }

    fn gatt_services(&self, link: u64) -> Vec<String> {
        match self.state.lock().expect("not poisoned").links.get(&link) {
            Some(Link::Gatt { services, .. }) => services.clone(),
            _ => Vec::new(),
        }
    }

    fn gatt_read(&self, link: u64, service: String, characteristic: String) -> Result<Vec<u8>, BtFailure> {
        let at = self.characteristic(link, &service, &characteristic)?;
        let conn = self.conn()?;
        block_on(call::<Vec<u8>>(&conn, &at, CHARACTERISTIC, "ReadValue", &(HashMap::<&str, Value>::new(),))).map_err(failed)
    }

    fn gatt_write(&self, link: u64, service: String, characteristic: String, bytes: Vec<u8>, with_response: bool) -> Result<(), BtFailure> {
        let at = self.characteristic(link, &service, &characteristic)?;
        let conn = self.conn()?;
        let options = HashMap::from([("type", Value::from(if with_response { "request" } else { "command" }))]);
        block_on(call::<()>(&conn, &at, CHARACTERISTIC, "WriteValue", &(bytes, options))).map_err(failed)
    }

    fn gatt_notify(&self, link: u64, service: String, characteristic: String, on: bool) -> Result<(), BtFailure> {
        let at = self.characteristic(link, &service, &characteristic)?;
        let conn = self.conn()?;
        block_on(call::<()>(&conn, &at, CHARACTERISTIC, if on { "StartNotify" } else { "StopNotify" }, &())).map_err(failed)?;
        if let Some(Link::Gatt { notifying, .. }) = self.state.lock().expect("not poisoned").links.get_mut(&link) {
            notifying.retain(|p| *p != at);
            if on {
                notifying.push(at);
            }
        }
        Ok(())
    }

    fn close(&self, link: u64) {
        let Some(l) = self.state.lock().expect("not poisoned").links.remove(&link) else { return };
        let Ok(conn) = self.conn() else { return };
        match l {
            Link::Rfcomm { socket, profile } => {
                // Ends its reading thread's read.
                // SAFETY: the socket's own descriptor, open for as long as `socket` is.
                unsafe { libc::shutdown(socket.as_raw_fd(), libc::SHUT_RDWR) };
                self.unregister(&conn, &profile);
            }
            Link::Gatt { notifying, .. } => {
                for at in notifying {
                    let _ = block_on(call::<()>(&conn, &at, CHARACTERISTIC, "StopNotify", &()));
                }
            }
        }
        let mut s = self.state.lock().expect("not poisoned");
        if s.links.is_empty() && s.bus.as_ref().is_some_and(|b| b.watch.is_none()) {
            s.bus = None;
        }
    }
}

impl Bluez {
    fn unregister(&self, conn: &Connection, profile: &str) {
        let _ = block_on(async {
            let path = zbus::zvariant::ObjectPath::try_from(profile)?;
            let _ = call::<()>(conn, "/org/bluez", "org.bluez.ProfileManager1", "UnregisterProfile", &(path,)).await;
            conn.object_server().remove::<Profile, _>(profile).await
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modalias_gives_its_vendor_and_product() {
        assert_eq!(modalias("usb:v054Cp0F8Ad0315"), (Some(0x054c), Some(0x0f8a)));
        assert_eq!(modalias("bluetooth:v004Cp200Ad1A00"), (Some(0x004c), Some(0x200a)));
        assert_eq!(modalias("nonsense"), (None, None));
    }

    #[test]
    fn a_device_reads_from_its_properties() {
        let props: Props = HashMap::from([
            ("Address".to_string(), OwnedValue::try_from(Value::from("58:18:62:25:85:CB")).unwrap()),
            ("Name".to_string(), OwnedValue::try_from(Value::from("WH-1000XM6")).unwrap()),
            ("Modalias".to_string(), OwnedValue::try_from(Value::from("usb:v054Cp0F8Ad0315")).unwrap()),
            ("UUIDs".to_string(), OwnedValue::try_from(Value::from(vec!["0000FE2C-0000-1000-8000-00805F9B34FB"])).unwrap()),
        ]);
        let d = device(&props).unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str(), d.vendor, d.product), ("58:18:62:25:85:CB", "WH-1000XM6", Some(0x054c), Some(0x0f8a)));
        assert_eq!(d.services, vec!["0000fe2c-0000-1000-8000-00805f9b34fb"]);
    }
}
