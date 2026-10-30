//! Classic Bluetooth through IOBluetooth, on a thread of its own ("nori-iobluetooth") whose run loop
//! IOBluetooth delivers to: made at the first need, ended once nothing is watched or open. Calls reach it
//! through a run loop source; what waits on the device (an SDP query, a channel opening) is waited for on
//! the caller's thread, so the run loop sleeps between events.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex, Weak};

use nori_plugins::{BtDevice, BtFailure, Plugins};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext};
use objc2_foundation::{NSArray, NSNumber, NSObjectProtocol, NSString};
use objc2_io_bluetooth::{
    IOBluetoothDevice, IOBluetoothRFCOMMChannel, IOBluetoothRFCOMMChannelDelegate, IOBluetoothSDPDataElement, IOBluetoothSDPServiceRecord, IOBluetoothSDPUUID,
    IOBluetoothUserNotification,
};

use super::{io, SLOWEST};
use crate::btids;

/// SDP's ServiceClassIDList: the UUIDs a service record is of.
const SERVICE_CLASSES: u16 = 0x0001;
/// The PnP Information service, with the device's vendor (0x0201) and product (0x0202).
const PNP: u16 = 0x1200;

type Job = Box<dyn FnOnce(&mut OnThread) + Send>;

#[derive(Default)]
struct Mailbox {
    jobs: Mutex<VecDeque<Job>>,
}

/// The thread's run loop and the source that brings it jobs.
struct Door {
    run_loop: CFRetained<CFRunLoop>,
    source: CFRetained<CFRunLoopSource>,
}

// SAFETY: a run loop and its source are only signaled, woken and released from other threads, which
// CFRunLoop allows from any thread.
unsafe impl Send for Door {}
// SAFETY: as above.
unsafe impl Sync for Door {}

struct Running {
    mailbox: Arc<Mailbox>,
    door: Door,
}

impl Running {
    fn start(plugins: Weak<Plugins>) -> Result<Running, BtFailure> {
        let mailbox = Arc::new(Mailbox::default());
        let (tx, rx) = mpsc::sync_channel(1);
        let m = mailbox.clone();
        std::thread::Builder::new().name("nori-iobluetooth".into()).spawn(move || run(m, plugins, tx)).map_err(|e| BtFailure::Failed { detail: e.to_string() })?;
        let door = rx.recv().map_err(|_| BtFailure::Failed { detail: "the Bluetooth thread did not start".into() })?;
        Ok(Running { mailbox, door })
    }

    fn post(&self, job: Job) {
        self.mailbox.jobs.lock().expect("not poisoned").push_back(job);
        self.door.source.signal();
        self.door.run_loop.wake_up();
    }
}

/// What the run loop's source reaches: the jobs and the thread's own state.
struct Here {
    mailbox: Arc<Mailbox>,
    on: RefCell<OnThread>,
}

unsafe extern "C-unwind" fn perform(info: *mut c_void) {
    // SAFETY: `info` is the `Here` that `run` keeps alive while the source is in its run loop.
    let here = unsafe { &*(info as *const Here) };
    loop {
        let job = here.mailbox.jobs.lock().expect("not poisoned").pop_front();
        let Some(job) = job else { break };
        // This run loop drains no autorelease pool of its own.
        objc2::rc::autoreleasepool(|_| job(&mut here.on.borrow_mut()));
    }
    if here.on.borrow().stop {
        if let Some(r) = CFRunLoop::current() {
            r.stop();
        }
    }
}

fn run(mailbox: Arc<Mailbox>, plugins: Weak<Plugins>, started: mpsc::SyncSender<Door>) {
    let here = Box::new(Here { mailbox, on: RefCell::new(OnThread { plugins, watcher: None, channels: HashMap::new(), queries: HashMap::new(), stop: false }) });
    let mut context = CFRunLoopSourceContext {
        version: 0,
        info: &*here as *const Here as *mut c_void,
        retain: None,
        release: None,
        copyDescription: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(perform),
    };
    // SAFETY: the context is copied by the call; its `info` outlives the source, invalidated below.
    let Some(source) = (unsafe { CFRunLoopSource::new(None, 0, &mut context) }) else { return };
    let Some(run_loop) = CFRunLoop::current() else { return };
    // SAFETY: a constant CoreFoundation defines.
    let mode = unsafe { kCFRunLoopDefaultMode };
    run_loop.add_source(Some(&source), mode);
    if started.send(Door { run_loop: run_loop.clone(), source: source.clone() }).is_err() {
        return source.invalidate();
    }
    CFRunLoop::run();
    source.invalidate();
    drop(here);
}

struct Channel {
    channel: Option<Retained<IOBluetoothRFCOMMChannel>>,
    /// Kept here: the channel does not retain its delegate.
    _delegate: Retained<RfcommDelegate>,
}

/// What lives on the thread.
struct OnThread {
    plugins: Weak<Plugins>,
    /// Hears devices connect, and its notification.
    watcher: Option<(Retained<Watcher>, Retained<IOBluetoothUserNotification>)>,
    channels: HashMap<u64, Channel>,
    /// A link's SDP query under way, kept until it ends.
    queries: HashMap<u64, Retained<SdpDone>>,
    stop: bool,
}

/// Where an RFCOMM channel opening is.
enum Opening {
    /// The device's services were not known: their query, then the opening again.
    Query(mpsc::Receiver<i32>),
    /// The channel's opening.
    Channel(mpsc::Receiver<i32>),
}

fn paired() -> Vec<Retained<IOBluetoothDevice>> {
    // SAFETY: IOBluetooth's own list, on its thread.
    let all: Option<Retained<NSArray>> = unsafe { IOBluetoothDevice::pairedDevices() };
    all.map(|a| a.iter().filter_map(|d| d.downcast::<IOBluetoothDevice>().ok()).collect()).unwrap_or_default()
}

fn address_of(d: &IOBluetoothDevice) -> Option<String> {
    // SAFETY: a property read.
    unsafe { d.addressString() }.map(|a| btids::address(&a.to_string()))
}

fn name_of(d: &IOBluetoothDevice) -> String {
    // SAFETY: `name` is nil until the device's name was asked once.
    let name: Option<Retained<NSString>> = unsafe { msg_send![d, name] };
    name.map(|n| n.to_string()).unwrap_or_default()
}

fn connected_device(address: &str) -> Option<Retained<IOBluetoothDevice>> {
    // SAFETY: a property read.
    paired().into_iter().find(|d| unsafe { d.isConnected() } && address_of(d).as_deref() == Some(address))
}

/// The service records SDP gave of `d`, kept by IOBluetooth.
fn records(d: &IOBluetoothDevice) -> Vec<Retained<IOBluetoothSDPServiceRecord>> {
    // SAFETY: a property read.
    let all: Option<Retained<NSArray>> = unsafe { d.services() };
    all.map(|a| a.iter().filter_map(|r| r.downcast::<IOBluetoothSDPServiceRecord>().ok()).collect()).unwrap_or_default()
}

fn attribute(r: &IOBluetoothSDPServiceRecord, id: u16) -> Option<Retained<IOBluetoothSDPDataElement>> {
    // SAFETY: a lookup in the record.
    unsafe { r.getAttributeDataElement(id) }
}

fn number(r: &IOBluetoothSDPServiceRecord, id: u16) -> Option<u16> {
    // SAFETY: a read of the element's value.
    let n: Option<Retained<NSNumber>> = unsafe { attribute(r, id)?.getNumberValue() };
    n.map(|n| n.unsignedShortValue())
}

fn device(d: &IOBluetoothDevice) -> Option<BtDevice> {
    let mut services = Vec::new();
    let mut ids = (None, None);
    for r in records(d) {
        // SAFETY: reads of the record's elements.
        let classes: Vec<Retained<AnyObject>> = unsafe { attribute(&r, SERVICE_CLASSES).and_then(|e| e.getArrayValue()) }.map(|a| a.to_vec()).unwrap_or_default();
        for class in classes {
            let Ok(element) = class.downcast::<IOBluetoothSDPDataElement>() else { continue };
            // SAFETY: as above.
            let Some(uuid) = (unsafe { element.getUUIDValue() }) else { continue };
            if let Some(text) = btids::text(&uuid.to_vec()) {
                if !services.contains(&text) {
                    services.push(text);
                }
            }
        }
        // SAFETY: a comparison within the record.
        if unsafe { r.matchesUUID16(PNP) } {
            ids = (number(&r, 0x0201), number(&r, 0x0202));
        }
    }
    Some(BtDevice { id: address_of(d)?, name: name_of(d), services, vendor: ids.0, product: ids.1 })
}

impl OnThread {
    fn watch(&mut self, on: bool) {
        if !on {
            if let Some((watcher, note)) = self.watcher.take() {
                // SAFETY: notifications IOBluetooth gave, ended once.
                unsafe { note.unregister() };
                for (_, n) in watcher.ivars().gone.borrow_mut().drain() {
                    // SAFETY: as above.
                    unsafe { n.unregister() };
                }
            }
            return;
        }
        if self.watcher.is_some() {
            return;
        }
        let watcher = Watcher::new(self.plugins.clone());
        // SAFETY: the watcher answers `connected:device:` and outlives the notification, which is unregistered before it goes.
        let note = unsafe { IOBluetoothDevice::registerForConnectNotifications_selector(Some(&watcher), Some(sel!(connected:device:))) };
        if let Some(note) = note {
            self.watcher = Some((watcher, note));
        }
    }

    fn connected(&mut self) -> Vec<BtDevice> {
        // SAFETY: a property read.
        let all: Vec<_> = paired().into_iter().filter(|d| unsafe { d.isConnected() }).collect();
        if let Some((w, _)) = &self.watcher {
            all.iter().for_each(|d| w.follow(d));
        }
        all.iter().filter_map(|d| device(d)).collect()
    }

    fn rfcomm(&mut self, link: u64, address: &str, uuid: [u8; 16], may_query: bool) -> Result<Opening, BtFailure> {
        let d = connected_device(address).ok_or(BtFailure::NotFound)?;
        // SAFETY: 16 bytes, copied by the call.
        let sdp = unsafe { IOBluetoothSDPUUID::uuidWithBytes_length(uuid.as_ptr().cast(), 16) }.ok_or(BtFailure::Unsupported)?;
        // SAFETY: a lookup among the records IOBluetooth keeps, then a read of one.
        let channel_id = unsafe { d.getServiceRecordForUUID(Some(&sdp)) }.and_then(|r| {
            let mut id = 0;
            (unsafe { r.getRFCOMMChannelID(&mut id) } == 0).then_some(id)
        });
        let Some(channel_id) = channel_id else {
            if !may_query {
                return Err(BtFailure::Unsupported);
            }
            let (tx, rx) = mpsc::channel();
            let done = SdpDone::new(tx);
            let target: &AnyObject = &done;
            // SAFETY: the target answers `sdpQueryComplete:status:` and is kept until the link opens or closes.
            io(unsafe { d.performSDPQuery(Some(target)) })?;
            self.queries.insert(link, done);
            return Ok(Opening::Query(rx));
        };
        self.queries.remove(&link);
        let (tx, rx) = mpsc::channel();
        let delegate = RfcommDelegate::new(link, self.plugins.clone(), tx);
        let mut channel = None;
        let target: &AnyObject = &delegate;
        // SAFETY: the delegate answers the RFCOMM delegate's methods and is kept with the channel.
        io(unsafe { d.openRFCOMMChannelAsync_withChannelID_delegate(Some(&mut channel), channel_id, Some(target)) })?;
        self.channels.insert(link, Channel { channel, _delegate: delegate });
        Ok(Opening::Channel(rx))
    }

    fn write(&mut self, link: u64, bytes: &[u8]) -> Result<(), BtFailure> {
        let channel = self.channels.get(&link).and_then(|c| c.channel.clone()).ok_or(BtFailure::NotFound)?;
        // SAFETY: a property read.
        let mtu = usize::from(unsafe { channel.getMTU() }).max(1);
        for chunk in bytes.chunks(mtu) {
            // SAFETY: the chunk is read, not written, for as long as the call takes.
            io(unsafe { channel.writeSync_length(chunk.as_ptr().cast_mut().cast(), chunk.len() as u16) })?;
        }
        Ok(())
    }

    fn close(&mut self, link: u64) {
        self.queries.remove(&link);
        if let Some(Channel { channel: Some(channel), .. }) = self.channels.remove(&link) {
            // SAFETY: the channel this link opened, let go of its delegate before it is closed.
            unsafe {
                channel.setDelegate(None);
                channel.closeChannel();
            }
        }
    }
}

struct WatcherIvars {
    plugins: Weak<Plugins>,
    /// Each connected device's disconnect notification, by address.
    gone: RefCell<HashMap<String, Retained<IOBluetoothUserNotification>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "NoriBluetoothWatcher"]
    #[ivars = WatcherIvars]
    struct Watcher;

    impl Watcher {
        #[unsafe(method(connected:device:))]
        fn connected(&self, _note: Option<&IOBluetoothUserNotification>, d: Option<&IOBluetoothDevice>) {
            let Some(d) = d else { return };
            self.follow(d);
            if let (Some(device), Some(plugins)) = (device(d), self.ivars().plugins.upgrade()) {
                plugins.bluetooth_connected(device);
            }
        }

        #[unsafe(method(disconnected:device:))]
        fn disconnected(&self, _note: Option<&IOBluetoothUserNotification>, d: Option<&IOBluetoothDevice>) {
            let Some(address) = d.and_then(address_of) else { return };
            if let Some(n) = self.ivars().gone.borrow_mut().remove(&address) {
                // SAFETY: the notification this device was followed with, ended once.
                unsafe { n.unregister() };
            }
            if let Some(plugins) = self.ivars().plugins.upgrade() {
                plugins.bluetooth_disconnected(address);
            }
        }
    }

    unsafe impl NSObjectProtocol for Watcher {}
);

impl Watcher {
    fn new(plugins: Weak<Plugins>) -> Retained<Watcher> {
        let this = Self::alloc().set_ivars(WatcherIvars { plugins, gone: RefCell::default() });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }

    /// Hears `d` disconnect, once.
    fn follow(&self, d: &IOBluetoothDevice) {
        let Some(address) = address_of(d) else { return };
        let mut gone = self.ivars().gone.borrow_mut();
        if gone.contains_key(&address) {
            return;
        }
        // SAFETY: the watcher answers `disconnected:device:` and unregisters the notification before it goes.
        if let Some(n) = unsafe { d.registerForDisconnectNotification_selector(Some(self), Some(sel!(disconnected:device:))) } {
            gone.insert(address, n);
        }
    }
}

struct SdpIvars {
    done: RefCell<Option<mpsc::Sender<i32>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "NoriBluetoothSdpDone"]
    #[ivars = SdpIvars]
    struct SdpDone;

    impl SdpDone {
        #[unsafe(method(sdpQueryComplete:status:))]
        fn complete(&self, _device: Option<&IOBluetoothDevice>, status: i32) {
            if let Some(to) = self.ivars().done.borrow_mut().take() {
                let _ = to.send(status);
            }
        }
    }

    unsafe impl NSObjectProtocol for SdpDone {}
);

impl SdpDone {
    fn new(done: mpsc::Sender<i32>) -> Retained<SdpDone> {
        let this = Self::alloc().set_ivars(SdpIvars { done: RefCell::new(Some(done)) });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }
}

struct RfcommIvars {
    link: u64,
    plugins: Weak<Plugins>,
    opened: RefCell<Option<mpsc::Sender<i32>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "NoriBluetoothRfcomm"]
    #[ivars = RfcommIvars]
    struct RfcommDelegate;

    unsafe impl NSObjectProtocol for RfcommDelegate {}

    unsafe impl IOBluetoothRFCOMMChannelDelegate for RfcommDelegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn data(&self, _channel: Option<&IOBluetoothRFCOMMChannel>, data: *mut c_void, length: usize) {
            if data.is_null() {
                return;
            }
            // SAFETY: IOBluetooth's buffer of `length` bytes, valid during the call.
            let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, length) }.to_vec();
            if let Some(plugins) = self.ivars().plugins.upgrade() {
                plugins.bluetooth_heard(self.ivars().link, String::new(), bytes);
            }
        }

        #[unsafe(method(rfcommChannelOpenComplete:status:))]
        fn open_complete(&self, _channel: Option<&IOBluetoothRFCOMMChannel>, status: i32) {
            if let Some(to) = self.ivars().opened.borrow_mut().take() {
                let _ = to.send(status);
            }
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn closed(&self, _channel: Option<&IOBluetoothRFCOMMChannel>) {
            if let Some(plugins) = self.ivars().plugins.upgrade() {
                plugins.bluetooth_lost(self.ivars().link);
            }
        }
    }
);

impl RfcommDelegate {
    fn new(link: u64, plugins: Weak<Plugins>, opened: mpsc::Sender<i32>) -> Retained<RfcommDelegate> {
        let this = Self::alloc().set_ivars(RfcommIvars { link, plugins, opened: RefCell::new(Some(opened)) });
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    watching: bool,
    /// The RFCOMM links open or opening.
    links: HashSet<u64>,
}

/// IOBluetooth's devices and RFCOMM channels.
pub(super) struct Classic {
    plugins: Weak<Plugins>,
    state: Mutex<State>,
}

impl Classic {
    pub(super) fn new(plugins: Weak<Plugins>) -> Classic {
        Classic { plugins, state: Mutex::default() }
    }

    /// Runs `job` on the thread, made if it is not there, and waits for it.
    fn call<R: Send + 'static>(&self, job: impl FnOnce(&mut OnThread) -> R + Send + 'static) -> Result<R, BtFailure> {
        let came = {
            let mut s = self.state.lock().expect("not poisoned");
            if s.running.is_none() {
                s.running = Some(Running::start(self.plugins.clone())?);
            }
            let (tx, rx) = mpsc::channel();
            s.running.as_ref().expect("just made").post(Box::new(move |t| drop(tx.send(job(t)))));
            rx
        };
        let r = came.recv().map_err(|_| BtFailure::Failed { detail: "the Bluetooth thread ended".into() });
        self.settle();
        r
    }

    /// Ends the thread once nothing is watched or open.
    fn settle(&self) {
        let mut s = self.state.lock().expect("not poisoned");
        if !s.watching && s.links.is_empty() {
            if let Some(r) = s.running.take() {
                r.post(Box::new(|t| t.stop = true));
            }
        }
    }

    pub(super) fn watch(&self, on: bool) {
        let mut s = self.state.lock().expect("not poisoned");
        s.watching = on;
        if !on && s.running.is_none() {
            return;
        }
        drop(s);
        let _ = self.call(move |t| t.watch(on));
    }

    pub(super) fn connected(&self) -> Vec<BtDevice> {
        self.call(|t| t.connected()).unwrap_or_default()
    }

    /// The name of connected device `address`.
    pub(super) fn name(&self, address: String) -> Option<String> {
        self.call(move |_| connected_device(&address).map(|d| name_of(&d))).ok().flatten()
    }

    pub(super) fn rfcomm(&self, link: u64, device: String, service: String) -> Result<(), BtFailure> {
        let uuid = btids::bytes(&service).ok_or(BtFailure::NotFound)?;
        self.state.lock().expect("not poisoned").links.insert(link);
        let opened = self.open(link, device, uuid);
        if opened.is_err() {
            self.close(link);
        }
        opened
    }

    fn open(&self, link: u64, device: String, uuid: [u8; 16]) -> Result<(), BtFailure> {
        let first = device.clone();
        let opening = match self.call(move |t| t.rfcomm(link, &first, uuid, true))?? {
            Opening::Channel(rx) => rx,
            Opening::Query(rx) => {
                io(rx.recv_timeout(SLOWEST).map_err(|_| BtFailure::Timeout)?)?;
                match self.call(move |t| t.rfcomm(link, &device, uuid, false))?? {
                    Opening::Channel(rx) => rx,
                    Opening::Query(_) => return Err(BtFailure::Unsupported),
                }
            }
        };
        io(opening.recv_timeout(SLOWEST).map_err(|_| BtFailure::Timeout)?)
    }

    pub(super) fn write(&self, link: u64, bytes: Vec<u8>) -> Result<(), BtFailure> {
        if !self.state.lock().expect("not poisoned").links.contains(&link) {
            return Err(BtFailure::NotFound);
        }
        self.call(move |t| t.write(link, &bytes))?
    }

    /// Closes `link` if it is an RFCOMM channel; whether it was.
    pub(super) fn close(&self, link: u64) -> bool {
        if !self.state.lock().expect("not poisoned").links.remove(&link) {
            return false;
        }
        let _ = self.call(move |t| t.close(link));
        true
    }
}
