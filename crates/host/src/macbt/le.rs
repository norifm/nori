//! Bluetooth LE through CoreBluetooth: a central manager whose delegate runs on a serial dispatch queue
//! ("nori-corebluetooth"), made at the first need and let go once nothing is watched or open. Every call
//! into CoreBluetooth runs on that queue; what a call waits for (a connection, services, a value) the
//! delegate hands over, and the caller waits for it on its own thread.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Instant;

use dispatch2::{DispatchQueue, DispatchRetained};
use nori_plugins::{BtDevice, BtFailure, Plugins};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_core_bluetooth::{
    CBAdvertisementDataLocalNameKey, CBAttribute, CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType, CBManagerState, CBPeripheral,
    CBPeripheralDelegate, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSObjectProtocol, NSString, NSUUID};

use super::SLOWEST;
use crate::btids;

pub(super) use crate::btids::is_peripheral_id as is_identifier;

/// The services a peripheral connected to the system is looked for by: Generic Access, Device
/// Information and Battery.
const PROBED: [u16; 3] = [0x1800, 0x180a, 0x180f];

/// A CoreBluetooth object, held anywhere but messaged only on the manager's queue.
struct OnQueue<T>(Retained<T>);

// SAFETY: the object is only retained and released off the queue, which Objective-C allows from any
// thread; it is messaged only inside `Central::on`, on the manager's serial queue.
unsafe impl<T> Send for OnQueue<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for OnQueue<T> {}

impl<T: Message> Clone for OnQueue<T> {
    fn clone(&self) -> Self {
        OnQueue(self.0.clone())
    }
}

impl<T> OnQueue<T> {
    /// The object, borrowed whole so a closure captures the wrapper (which is `Send`), not the object.
    fn get(&self) -> &T {
        &self.0
    }
}

/// A service's and a characteristic's UUIDs.
type Key = (String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// A scan found the peripheral it seeks.
    Found,
    Connect,
    Services,
    /// The characteristics of the service of this UUID.
    Characteristics(String),
    Read(Key),
    Write(Key),
    Notify(Key),
}

/// What the delegate heard that a call waits for.
struct Done {
    peripheral: String,
    step: Step,
    result: Result<Vec<u8>, String>,
}

struct GattLink {
    peripheral: OnQueue<CBPeripheral>,
    /// The peripheral's identifier.
    id: String,
    services: Vec<String>,
    characteristics: HashMap<Key, OnQueue<CBCharacteristic>>,
    mtu: u16,
    notifying: HashSet<Key>,
    /// The characteristics a read waits on; any other value is a notification.
    reading: HashSet<Key>,
}

#[derive(Default)]
struct Book {
    power: Option<CBManagerState>,
    links: HashMap<u64, GattLink>,
    done: Vec<Done>,
    /// The peripherals that disconnected since a link connected to them.
    gone: HashSet<String>,
    /// The name a scan looks for, and the peripheral it found.
    seeking: Option<String>,
    found: Option<OnQueue<CBPeripheral>>,
}

#[derive(Default)]
struct Shared {
    book: Mutex<Book>,
    came: Condvar,
}

impl Shared {
    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().expect("not poisoned")
    }

    fn push(&self, peripheral: String, step: Step, result: Result<Vec<u8>, String>) {
        self.book().done.push(Done { peripheral, step, result });
        self.came.notify_all();
    }

    fn forget(&self, peripheral: &str, step: &Step) {
        self.book().done.retain(|d| d.peripheral != peripheral || d.step != *step);
    }

    /// What came for `step` of `peripheral`, waiting at most [`SLOWEST`].
    fn wait(&self, peripheral: &str, step: &Step) -> Result<Vec<u8>, BtFailure> {
        let until = Instant::now() + SLOWEST;
        let mut b = self.book();
        loop {
            if let Some(at) = b.done.iter().position(|d| d.peripheral == peripheral && d.step == *step) {
                return b.done.remove(at).result.map_err(|detail| BtFailure::Failed { detail });
            }
            if !matches!(step, Step::Found | Step::Connect) && b.gone.contains(peripheral) {
                return Err(BtFailure::Failed { detail: "disconnected".into() });
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(BtFailure::Timeout);
            }
            b = self.came.wait_timeout(b, left).expect("not poisoned").0;
        }
    }
}

fn uuid_of(a: &CBAttribute) -> Option<String> {
    // SAFETY: property reads, on the queue.
    btids::text(&unsafe { a.UUID().data() }.to_vec())
}

fn id_of(p: &CBPeripheral) -> String {
    // SAFETY: a property read.
    unsafe { p.identifier() }.UUIDString().to_string()
}

fn name_of(p: &CBPeripheral) -> String {
    // SAFETY: a property read.
    unsafe { p.name() }.map(|n| n.to_string()).unwrap_or_default()
}

fn failure(e: Option<&NSError>) -> Result<Vec<u8>, String> {
    e.map_or(Ok(Vec::new()), |e| Err(e.localizedDescription().to_string()))
}

/// A characteristic's service and its own UUID.
fn key_of(c: &CBCharacteristic) -> Option<Key> {
    // SAFETY: a property read.
    let service: Retained<CBService> = unsafe { c.service() }?;
    Some((uuid_of(&service)?, uuid_of(c)?))
}

fn key(service: &str, characteristic: &str) -> Result<Key, BtFailure> {
    let full = |u: &str| btids::bytes(u).and_then(|b| btids::text(&b)).ok_or(BtFailure::NotFound);
    Ok((full(service)?, full(characteristic)?))
}

struct DelegateIvars {
    shared: Arc<Shared>,
    plugins: Weak<Plugins>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "NoriBluetoothCentral"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let shared = &self.ivars().shared;
            // SAFETY: a property read, on the queue.
            shared.book().power = Some(unsafe { central.state() });
            shared.came.notify_all();
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(&self, _central: &CBCentralManager, peripheral: &CBPeripheral, advertised: &NSDictionary<NSString, AnyObject>, _rssi: &NSNumber) {
            // SAFETY: a constant CoreBluetooth defines.
            let local = advertised.objectForKey(unsafe { CBAdvertisementDataLocalNameKey }).and_then(|n| n.downcast::<NSString>().ok()).map(|n| n.to_string());
            let shared = &self.ivars().shared;
            {
                let mut b = shared.book();
                let name = name_of(peripheral);
                if b.found.is_some() || !b.seeking.as_ref().is_some_and(|s| *s == name || Some(s) == local.as_ref()) {
                    return;
                }
                b.found = Some(OnQueue(peripheral.retain()));
            }
            shared.push(String::new(), Step::Found, Ok(Vec::new()));
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            self.ivars().shared.push(id_of(peripheral), Step::Connect, Ok(Vec::new()));
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_to_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral, error: Option<&NSError>) {
            let result = failure(error).and_then(|_| Err("connection failed".into()));
            self.ivars().shared.push(id_of(peripheral), Step::Connect, result);
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral, _error: Option<&NSError>) {
            let id = id_of(peripheral);
            let shared = &self.ivars().shared;
            let lost: Vec<u64> = {
                let mut b = shared.book();
                b.gone.insert(id.clone());
                b.links.iter().filter(|(_, g)| g.id == id).map(|(l, _)| *l).collect()
            };
            shared.came.notify_all();
            if let Some(plugins) = self.ivars().plugins.upgrade() {
                lost.into_iter().for_each(|l| plugins.bluetooth_lost(l));
            }
        }
    }

    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, peripheral: &CBPeripheral, error: Option<&NSError>) {
            self.ivars().shared.push(id_of(peripheral), Step::Services, failure(error));
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_characteristics(&self, peripheral: &CBPeripheral, service: &CBService, error: Option<&NSError>) {
            let Some(uuid) = uuid_of(service) else { return };
            self.ivars().shared.push(id_of(peripheral), Step::Characteristics(uuid), failure(error));
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(&self, peripheral: &CBPeripheral, characteristic: &CBCharacteristic, error: Option<&NSError>) {
            let Some(k) = key_of(characteristic) else { return };
            let id = id_of(peripheral);
            // SAFETY: a property read, on the queue.
            let value = unsafe { characteristic.value() }.map(|v| v.to_vec()).unwrap_or_default();
            let shared = &self.ivars().shared;
            let (read, heard) = {
                let mut b = shared.book();
                let read = b.links.values_mut().any(|g| g.id == id && g.reading.remove(&k));
                let heard: Vec<u64> = b.links.iter().filter(|(_, g)| g.id == id && g.notifying.contains(&k)).map(|(l, _)| *l).collect();
                (read, heard)
            };
            if read {
                return shared.push(id, Step::Read(k), failure(error).map(|_| value));
            }
            if let (None, Some(plugins)) = (error, self.ivars().plugins.upgrade()) {
                heard.into_iter().for_each(|l| plugins.bluetooth_heard(l, k.1.clone(), value.clone()));
            }
        }

        #[unsafe(method(peripheral:didWriteValueForCharacteristic:error:))]
        fn did_write_value(&self, peripheral: &CBPeripheral, characteristic: &CBCharacteristic, error: Option<&NSError>) {
            let Some(k) = key_of(characteristic) else { return };
            self.ivars().shared.push(id_of(peripheral), Step::Write(k), failure(error));
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn did_update_notification_state(&self, peripheral: &CBPeripheral, characteristic: &CBCharacteristic, error: Option<&NSError>) {
            let Some(k) = key_of(characteristic) else { return };
            self.ivars().shared.push(id_of(peripheral), Step::Notify(k), failure(error));
        }
    }
);

impl Delegate {
    fn new(shared: Arc<Shared>, plugins: Weak<Plugins>) -> Retained<Delegate> {
        let this = Self::alloc().set_ivars(DelegateIvars { shared, plugins });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }
}

/// The manager, its delegate (which it does not retain) and its queue.
struct Central {
    manager: OnQueue<CBCentralManager>,
    delegate: OnQueue<Delegate>,
    queue: DispatchRetained<DispatchQueue>,
}

impl Central {
    /// Runs `f` on the manager's queue. Never called from the queue, nor with the book locked, which the
    /// delegate takes there.
    fn on<R: Send>(&self, f: impl FnOnce(&CBCentralManager) -> R + Send) -> R {
        let mut out = None;
        let manager = &self.manager;
        self.queue.exec_sync(|| out = Some(f(manager.get())));
        out.expect("ran on the queue")
    }

    /// The peripherals connected to the system offering service `short`, with their ids.
    fn retrieve(&self, short: u16) -> Vec<(String, OnQueue<CBPeripheral>)> {
        self.on(|m| {
            // SAFETY: CoreBluetooth calls on its queue.
            let uuid = unsafe { CBUUID::UUIDWithString(&NSString::from_str(&format!("{short:04X}"))) };
            let found = unsafe { m.retrieveConnectedPeripheralsWithServices(&NSArray::from_retained_slice(&[uuid])) };
            found.iter().map(|p| (id_of(&p), OnQueue(p))).collect()
        })
    }
}

/// A characteristic of a link: the manager, the peripheral and its identifier, the characteristic.
struct Target {
    c: Arc<Central>,
    p: OnQueue<CBPeripheral>,
    id: String,
    ch: OnQueue<CBCharacteristic>,
}

#[derive(Default)]
struct State {
    central: Option<Arc<Central>>,
    watching: bool,
}

/// CoreBluetooth's peripherals and GATT links.
pub(super) struct Le {
    plugins: Weak<Plugins>,
    shared: Arc<Shared>,
    state: Mutex<State>,
}

impl Le {
    pub(super) fn new(plugins: Weak<Plugins>) -> Le {
        Le { plugins, shared: Arc::default(), state: Mutex::default() }
    }

    /// The manager, made at first need, once Bluetooth is on.
    fn central(&self) -> Result<Arc<Central>, BtFailure> {
        let made = {
            let mut s = self.state.lock().expect("not poisoned");
            match &s.central {
                Some(c) => c.clone(),
                None => {
                    self.shared.book().power = None;
                    let queue = DispatchQueue::new("nori-corebluetooth", None);
                    let delegate = Delegate::new(self.shared.clone(), self.plugins.clone());
                    // SAFETY: the delegate is kept beside the manager; events come on `queue`.
                    let manager = unsafe { CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(ProtocolObject::from_ref(&*delegate)), Some(&queue)) };
                    let c = Arc::new(Central { manager: OnQueue(manager), delegate: OnQueue(delegate), queue });
                    s.central = Some(c.clone());
                    c
                }
            }
        };
        let until = Instant::now() + SLOWEST;
        let mut b = self.shared.book();
        loop {
            match b.power {
                Some(CBManagerState::PoweredOn) => return Ok(made),
                Some(CBManagerState::Unauthorized) => return Err(BtFailure::Refused),
                Some(CBManagerState::Unsupported) => return Err(BtFailure::Unsupported),
                Some(CBManagerState::PoweredOff) => return Err(BtFailure::Failed { detail: "Bluetooth is off".into() }),
                _ => {}
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(BtFailure::Timeout);
            }
            b = self.shared.came.wait_timeout(b, left).expect("not poisoned").0;
        }
    }

    /// Lets the manager go once nothing is watched or open.
    fn settle(&self) {
        let mut s = self.state.lock().expect("not poisoned");
        if !s.watching && self.shared.book().links.is_empty() {
            s.central = None;
        }
    }

    /// Starts `step` of `peripheral` with `start` and waits for it.
    fn ask(&self, c: &Central, peripheral: &str, step: Step, start: impl FnOnce(&CBCentralManager) + Send) -> Result<Vec<u8>, BtFailure> {
        self.shared.forget(peripheral, &step);
        c.on(start);
        self.shared.wait(peripheral, &step)
    }

    pub(super) fn watch(&self, on: bool) {
        self.state.lock().expect("not poisoned").watching = on;
        if !on {
            self.settle();
        }
    }

    /// The peripherals connected to the system over LE, by the services probed.
    pub(super) fn connected(&self) -> Vec<BtDevice> {
        let Ok(c) = self.central() else {
            self.settle();
            return Vec::new();
        };
        let mut all: Vec<BtDevice> = Vec::new();
        for short in PROBED {
            let service = btids::text(&short.to_be_bytes()).expect("two bytes");
            for (id, p) in c.retrieve(short) {
                let name = c.on(|_| name_of(p.get()));
                let at = match all.iter().position(|d| d.id == id) {
                    Some(at) => at,
                    None => {
                        all.push(BtDevice { id, name, services: Vec::new(), vendor: None, product: None });
                        all.len() - 1
                    }
                };
                all[at].services.push(service.clone());
            }
        }
        self.settle();
        all
    }

    pub(super) fn gatt(&self, link: u64, device: String, name: Option<String>) -> Result<(), BtFailure> {
        let opened = self.central().and_then(|c| self.open(&c, link, &device, name));
        if opened.is_err() {
            self.close(link);
        }
        opened
    }

    /// The peripheral `device` is: its identifier, or the one of classic device's `name`, connected to the
    /// system or else found by a scan.
    fn find(&self, c: &Central, device: &str, name: Option<String>) -> Result<OnQueue<CBPeripheral>, BtFailure> {
        let Some(name) = name else {
            let id = device.to_string();
            return c
                .on(move |m| {
                    let uuid = NSUUID::from_string(&NSString::from_str(&id))?;
                    // SAFETY: a CoreBluetooth call on its queue.
                    unsafe { m.retrievePeripheralsWithIdentifiers(&NSArray::from_retained_slice(&[uuid])) }.firstObject().map(OnQueue)
                })
                .ok_or(BtFailure::NotFound);
        };
        if name.is_empty() {
            return Err(BtFailure::NotFound);
        }
        for short in PROBED {
            let named = c.retrieve(short).into_iter().map(|(_, p)| p).find(|p| c.on(|_| name_of(p.get())) == name);
            if let Some(p) = named {
                return Ok(p);
            }
        }
        {
            let mut b = self.shared.book();
            b.seeking = Some(name);
            b.found = None;
        }
        // SAFETY: CoreBluetooth calls on its queue.
        let scanned = self.ask(c, "", Step::Found, |m| unsafe { m.scanForPeripheralsWithServices_options(None, None) });
        c.on(|m| unsafe { m.stopScan() });
        let found = {
            let mut b = self.shared.book();
            b.seeking = None;
            b.found.take()
        };
        scanned?;
        found.ok_or(BtFailure::NotFound)
    }

    fn open(&self, c: &Central, link: u64, device: &str, name: Option<String>) -> Result<(), BtFailure> {
        let p = self.find(c, device, name)?;
        let id = c.on(|_| id_of(p.get()));
        {
            let mut b = self.shared.book();
            b.gone.remove(&id);
            let link_state = GattLink { peripheral: p.clone(), id: id.clone(), services: Vec::new(), characteristics: HashMap::new(), mtu: 23, notifying: HashSet::new(), reading: HashSet::new() };
            b.links.insert(link, link_state);
        }
        let delegate = &c.delegate;
        // SAFETY: CoreBluetooth calls on its queue; the delegate is kept beside the manager.
        self.ask(c, &id, Step::Connect, |m| unsafe {
            p.get().setDelegate(Some(ProtocolObject::from_ref(delegate.get())));
            m.connectPeripheral_options(p.get(), None);
        })?;
        // SAFETY: as above.
        self.ask(c, &id, Step::Services, |_| unsafe { p.get().discoverServices(None) })?;
        // SAFETY: a property read on the queue.
        let services: Vec<(String, OnQueue<CBService>)> = c.on(|_| unsafe { p.get().services() }.map(|a| a.iter().filter_map(|s| Some((uuid_of(&s)?, OnQueue(s)))).collect()).unwrap_or_default());
        for (uuid, s) in &services {
            // SAFETY: as above.
            self.ask(c, &id, Step::Characteristics(uuid.clone()), |_| unsafe { p.get().discoverCharacteristics_forService(None, s.get()) })?;
        }
        let (characteristics, mtu) = c.on(|_| {
            let mut all = HashMap::new();
            for (uuid, s) in &services {
                // SAFETY: a property read on the queue.
                for ch in unsafe { s.get().characteristics() }.map(|a| a.to_vec()).unwrap_or_default() {
                    if let Some(u) = uuid_of(&ch) {
                        all.insert((uuid.clone(), u), OnQueue(ch));
                    }
                }
            }
            // SAFETY: as above.
            let most = unsafe { p.get().maximumWriteValueLengthForType(CBCharacteristicWriteType::WithoutResponse) };
            (all, u16::try_from(most + 3).unwrap_or(u16::MAX))
        });
        let mut uuids: Vec<String> = services.into_iter().map(|(u, _)| u).collect();
        uuids.sort();
        uuids.dedup();
        if let Some(g) = self.shared.book().links.get_mut(&link) {
            (g.services, g.characteristics, g.mtu) = (uuids, characteristics, mtu);
        }
        Ok(())
    }

    pub(super) fn mtu(&self, link: u64) -> u16 {
        self.shared.book().links.get(&link).map_or(23, |g| g.mtu)
    }

    pub(super) fn services(&self, link: u64) -> Vec<String> {
        self.shared.book().links.get(&link).map(|g| g.services.clone()).unwrap_or_default()
    }

    /// The characteristic `k` of `link`, with its peripheral and manager.
    fn characteristic(&self, link: u64, k: &Key) -> Result<Target, BtFailure> {
        let c = self.state.lock().expect("not poisoned").central.clone().ok_or(BtFailure::NotFound)?;
        let b = self.shared.book();
        let g = b.links.get(&link).ok_or(BtFailure::NotFound)?;
        let ch = g.characteristics.get(k).ok_or(BtFailure::NotFound)?.clone();
        Ok(Target { c, p: g.peripheral.clone(), id: g.id.clone(), ch })
    }

    pub(super) fn read(&self, link: u64, service: &str, characteristic: &str) -> Result<Vec<u8>, BtFailure> {
        let k = key(service, characteristic)?;
        let Target { c, p, id, ch } = self.characteristic(link, &k)?;
        if let Some(g) = self.shared.book().links.get_mut(&link) {
            g.reading.insert(k.clone());
        }
        // SAFETY: a CoreBluetooth call on its queue.
        let value = self.ask(&c, &id, Step::Read(k.clone()), |_| unsafe { p.get().readValueForCharacteristic(ch.get()) });
        if let Some(g) = self.shared.book().links.get_mut(&link) {
            g.reading.remove(&k);
        }
        value
    }

    pub(super) fn write(&self, link: u64, service: &str, characteristic: &str, bytes: Vec<u8>, with_response: bool) -> Result<(), BtFailure> {
        let k = key(service, characteristic)?;
        let Target { c, p, id, ch } = self.characteristic(link, &k)?;
        let kind = if with_response { CBCharacteristicWriteType::WithResponse } else { CBCharacteristicWriteType::WithoutResponse };
        // SAFETY: a CoreBluetooth call on its queue.
        let start = move |_: &CBCentralManager| unsafe { p.get().writeValue_forCharacteristic_type(&NSData::with_bytes(&bytes), ch.get(), kind) };
        if with_response {
            self.ask(&c, &id, Step::Write(k), start).map(drop)
        } else {
            c.on(start);
            Ok(())
        }
    }

    pub(super) fn notify(&self, link: u64, service: &str, characteristic: &str, on: bool) -> Result<(), BtFailure> {
        let k = key(service, characteristic)?;
        let Target { c, p, id, ch } = self.characteristic(link, &k)?;
        // SAFETY: a CoreBluetooth call on its queue.
        self.ask(&c, &id, Step::Notify(k.clone()), |_| unsafe { p.get().setNotifyValue_forCharacteristic(on, ch.get()) })?;
        if let Some(g) = self.shared.book().links.get_mut(&link) {
            if on {
                g.notifying.insert(k);
            } else {
                g.notifying.remove(&k);
            }
        }
        Ok(())
    }

    pub(super) fn close(&self, link: u64) {
        let (g, shared_peripheral) = {
            let mut b = self.shared.book();
            let Some(g) = b.links.remove(&link) else { return };
            let shared_peripheral = b.links.values().any(|o| o.id == g.id);
            (g, shared_peripheral)
        };
        let central = self.state.lock().expect("not poisoned").central.clone();
        if let Some(c) = central {
            c.on(|m| {
                let notifying: Vec<&OnQueue<CBCharacteristic>> = g.notifying.iter().filter_map(|k| g.characteristics.get(k)).collect();
                // SAFETY: CoreBluetooth calls on its queue.
                unsafe {
                    for ch in notifying {
                        g.peripheral.get().setNotifyValue_forCharacteristic(false, ch.get());
                    }
                    if !shared_peripheral {
                        m.cancelPeripheralConnection(g.peripheral.get());
                    }
                }
            });
        }
        self.settle();
    }
}
