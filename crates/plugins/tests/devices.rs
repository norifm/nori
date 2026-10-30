//! Device control over a fake platform Bluetooth, with a real plugin (fixture/src/headphones.rs):
//! watching only while a plugin reaches devices, offering a device to the plugins its filter lets
//! reach it, RFCOMM and GATT both ways, and what nori shows of it.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

use nori_plugins::testing::fixture;
use nori_plugins::{output_device, Battery, Bluetooth, BtDevice, BtFailure, Noise, Plugins, Side, Value};

const ID: &str = "fm.nori.test.headphones";
const SPP: &str = "00001101-0000-1000-8000-00805f9b34fb";
const LEVEL: &str = "00002a19-0000-1000-8000-00805f9b34fb";

fn device(id: &str, name: &str) -> BtDevice {
    BtDevice { id: id.into(), name: name.into(), services: vec![SPP.into()], vendor: Some(0x0a12), product: None }
}

/// Headphones answering the fixture's protocol, and what the host asked of it.
#[derive(Default)]
struct Fake {
    plugins: OnceLock<Weak<Plugins>>,
    devices: Vec<BtDevice>,
    /// Those connecting after the host asked.
    later: Mutex<Vec<BtDevice>>,
    watching: Mutex<Vec<bool>>,
    /// Links opened: (link, device, what).
    opened: Mutex<Vec<(u64, String, String)>>,
    closed: (Mutex<Vec<u64>>, Condvar),
    /// The device's state: left, right, noise.
    buds: Mutex<[u8; 3]>,
}

impl Fake {
    fn host(&self) -> Arc<Plugins> {
        self.plugins.get().and_then(Weak::upgrade).unwrap()
    }

    fn link(&self, what: &str) -> u64 {
        self.opened.lock().unwrap().iter().find(|(_, _, w)| w == what).map(|o| o.0).unwrap()
    }
}

impl Bluetooth for Fake {
    fn watch(&self, on: bool) {
        self.watching.lock().unwrap().push(on);
    }
    fn connected(&self) -> Vec<BtDevice> {
        self.devices.clone()
    }
    fn rfcomm(&self, link: u64, device: String, service: String) -> Result<(), BtFailure> {
        let known = |list: &[BtDevice]| list.iter().any(|d| d.id == device && d.services.contains(&service));
        if !known(&self.devices) && !known(&self.later.lock().unwrap()) {
            return Err(BtFailure::NotFound);
        }
        self.opened.lock().unwrap().push((link, device, format!("rfcomm {service}")));
        Ok(())
    }
    fn rfcomm_write(&self, link: u64, bytes: Vec<u8>) -> Result<(), BtFailure> {
        let frame = {
            let mut b = self.buds.lock().unwrap();
            if let [b'N', n] = bytes[..] {
                b[2] = n;
            }
            b.to_vec()
        };
        // The answer comes as the write returns, before the plugin reads.
        self.host().bluetooth_heard(link, String::new(), frame);
        Ok(())
    }
    fn gatt(&self, link: u64, device: String) -> Result<(), BtFailure> {
        self.opened.lock().unwrap().push((link, device, "gatt".into()));
        Ok(())
    }
    fn gatt_mtu(&self, _: u64) -> u16 {
        185
    }
    fn gatt_services(&self, _: u64) -> Vec<String> {
        vec!["0000180f-0000-1000-8000-00805f9b34fb".into()]
    }
    fn gatt_read(&self, _: u64, _: String, characteristic: String) -> Result<Vec<u8>, BtFailure> {
        if characteristic == LEVEL { Ok(vec![90]) } else { Err(BtFailure::NotFound) }
    }
    fn gatt_write(&self, _: u64, _: String, _: String, _: Vec<u8>, _: bool) -> Result<(), BtFailure> {
        Ok(())
    }
    fn gatt_notify(&self, link: u64, _: String, characteristic: String, on: bool) -> Result<(), BtFailure> {
        self.opened.lock().unwrap().push((link, String::new(), format!("notify {characteristic} {on}")));
        Ok(())
    }
    fn close(&self, link: u64) {
        self.closed.0.lock().unwrap().push(link);
        self.closed.1.notify_all();
    }
}

/// Counts what the host says changed, and the plugin's notices.
#[derive(Default)]
struct Session {
    changes: (Mutex<u32>, Condvar),
    notices: Mutex<HashMap<String, Vec<String>>>,
    /// Each notice's words, in order.
    said: Mutex<Vec<String>>,
}

impl Session {
    /// Waits, change by change, until `f` holds.
    fn until(&self, what: &str, f: impl Fn() -> bool) {
        let (n, came) = &self.changes;
        let mut seen = n.lock().unwrap();
        while !f() {
            let (s, t) = came.wait_timeout(seen, Duration::from_secs(20)).unwrap();
            seen = s;
            assert!(!t.timed_out() || f(), "{what}");
        }
    }

    fn notice(&self, words: &str) -> Option<Vec<String>> {
        self.notices.lock().unwrap().get(words).cloned()
    }

    fn poke(&self) {
        *self.changes.0.lock().unwrap() += 1;
        self.changes.1.notify_all();
    }
}

impl Side for Session {
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, words: &str, args: &[String]) {
        self.notices.lock().unwrap().insert(words.into(), args.to_vec());
        self.said.lock().unwrap().push(words.into());
        self.poke();
    }
    fn devices_changed(&self) {
        self.poke();
    }
}

fn open(name: &str, devices: Vec<BtDevice>) -> (nori_testdir::TempDir, Arc<Session>, Arc<Fake>, Arc<Plugins>) {
    open_hearing(name, devices, false)
}

/// With `outputs`, the fake is how the client hears output devices connect, as BlueZ is on Linux.
fn open_hearing(name: &str, devices: Vec<BtDevice>, outputs: bool) -> (nori_testdir::TempDir, Arc<Session>, Arc<Fake>, Arc<Plugins>) {
    let dir = nori_testdir::TempDir::new(name);
    let session = Arc::new(Session::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), session.clone());
    let fake = Arc::new(Fake { devices, buds: Mutex::new([80, 70, 1]), ..Fake::default() });
    let _ = fake.plugins.set(Arc::downgrade(&plugins));
    plugins.use_bluetooth(fake.clone(), outputs);
    (dir, session, fake, plugins)
}

#[test]
fn headphones_are_controlled_while_connected_and_heard_without_polling() {
    let (_dir, session, fake, plugins) = open("plugins-devices", vec![device("AA", "Test Buds Pro"), device("CAR", "Car")]);
    plugins.install(&fixture("headphones")).unwrap();
    assert!(fake.watching.lock().unwrap().is_empty(), "nothing is watched before a plugin reaching devices is on");
    assert!(plugins.reaches_bluetooth(), "the permission is for it");

    plugins.turn(ID.into(), true);
    assert_eq!(*fake.watching.lock().unwrap(), vec![true]);
    let shown = |p: &Plugins| p.controlled("en".into()).into_iter().next();
    session.until("the buds are controlled with their state", || shown(&plugins).is_some_and(|d| d.battery.case.is_some()));
    let d = shown(&plugins).unwrap();
    assert_eq!((d.id.as_str(), d.name.as_str(), d.card.id.as_str()), ("AA", "Test Buds Pro", ID));
    assert_eq!(d.battery, Battery { left: Some(80), right: Some(70), case: Some(90), single: None });
    assert_eq!((d.level, d.noise, d.noise_modes.clone()), (Some(70), Some(Noise::Canceling), vec![Noise::Off, Noise::Canceling, Noise::Transparency]));
    assert_eq!(plugins.controlled("en".into()).len(), 1, "the car is not among the devices it reaches");
    assert_eq!(session.notice("reach"), Some(vec!["AA".into(), "ErrorKind::NotFound".into()]), "it sees and opens only what its filter matches");
    assert_eq!(session.notice("gatt"), Some(vec!["185".into(), "0000180f-0000-1000-8000-00805f9b34fb".into()]));
    assert_eq!(session.notice("connected"), Some(vec!["AA".into()]));
    assert!(fake.opened.lock().unwrap().iter().any(|o| o.2 == format!("notify {LEVEL} true")));
    let all = plugins.controlled("en".into());
    let on = |output: Option<&str>| output_device(&all, output).map(|d| d.id.as_str());
    assert_eq!((on(Some("test buds pro")), on(Some("AA")), on(Some("Speaker"))), (Some("AA"), Some("AA"), None), "by its name or id");
    assert_eq!(on(None), Some("AA"), "the default output is the one device");
    let two = [all.clone(), all.clone()].concat();
    assert_eq!(output_device(&two, None), None, "of two, neither is the default's");

    // What the device sends on its own, and a characteristic's notification.
    fake.host().bluetooth_heard(fake.link(&format!("rfcomm {SPP}")), String::new(), vec![20, 60, 0]);
    session.until("a frame of its own is heard", || shown(&plugins).is_some_and(|d| d.noise == Some(Noise::Off)));
    fake.host().bluetooth_heard(fake.link("gatt"), LEVEL.into(), vec![45]);
    session.until("a notification is heard", || shown(&plugins).is_some_and(|d| d.battery.case == Some(45)));
    assert_eq!(shown(&plugins).unwrap().level, Some(20));

    plugins.set_noise("AA".into(), Noise::Transparency);
    session.until("the noise control is set", || shown(&plugins).is_some_and(|d| d.noise == Some(Noise::Transparency)));

    plugins.bluetooth_disconnected("AA".into());
    assert!(plugins.controlled("en".into()).is_empty(), "shown only while connected");
    let closed = fake.closed.1.wait_timeout_while(fake.closed.0.lock().unwrap(), Duration::from_secs(20), |c| c.len() < 2).unwrap().0;
    assert_eq!(closed.len(), 2, "its channels close as it lets the device go");
    drop(closed);
    session.until("its hooks hear it went", || session.notice("disconnected").is_some());

    plugins.turn(ID.into(), false);
    assert_eq!(*fake.watching.lock().unwrap(), vec![true, false], "off, nothing is watched");
}

#[test]
fn a_device_connecting_later_is_offered_and_one_it_declines_is_not_shown() {
    let (_dir, session, fake, plugins) = open("plugins-devices-later", Vec::new());
    plugins.install(&fixture("headphones")).unwrap();
    plugins.turn(ID.into(), true);
    let buds = BtDevice { services: Vec::new(), ..device("BB", "Test Buds") };
    fake.later.lock().unwrap().push(buds.clone());
    plugins.bluetooth_connected(device("CAR", "Car"));
    plugins.bluetooth_connected(buds);
    // Calls run in order: once this one answers, it has been asked.
    assert_eq!(nori_tasks::block_on(plugins.worker(ID).unwrap().call(|_, _| Ok(1))), Some(1));
    assert_eq!(session.notice("connected"), Some(vec!["BB".into()]), "its hooks hear only of what it reaches");
    // Without the channel the fixture needs, it declines: told of the device, never controlling it.
    assert_eq!(session.notice("reach"), Some(vec!["BB".into(), "ErrorKind::NotFound".into()]));
    assert!(plugins.controlled("en".into()).is_empty());
    let opened = fake.opened.lock().unwrap().iter().map(|o| o.1.clone()).collect::<Vec<_>>();
    assert!(opened.iter().all(|d| d == "BB"), "nothing was opened to the car: {opened:?}");
}

#[test]
fn its_timers_fire_as_asked_only_while_it_controls_a_device() {
    let (_dir, session, fake, plugins) = open("plugins-devices-timers", vec![device("AA", "Test Buds Pro")]);
    let clock = Arc::new(nori_tasks::Native::simulated("plugin-clock", 0).unwrap());
    plugins.use_tasks(clock.clone());
    plugins.install(&fixture("headphones")).unwrap();
    plugins.turn(ID.into(), true);
    session.until("the buds are controlled", || !plugins.controlled("en".into()).is_empty());
    let settle = || {
        clock.settle();
        assert_eq!(nori_tasks::block_on(plugins.worker(ID).unwrap().call(|_, _| Ok(1))), Some(1));
    };
    settle();
    clock.pass(nori_time::Duration::from_millis(3500));
    settle();
    assert_eq!(session.notice("tick"), Some(vec!["7".into()]), "a controlled device makes its session active");

    session.notices.lock().unwrap().remove("tick");
    fake.host().bluetooth_disconnected("AA".into());
    settle();
    clock.pass(nori_time::Duration::from_millis(3500));
    settle();
    assert_eq!(session.notice("tick"), None, "without the device and without the schedule permission");
    assert!(clock.waits().is_empty(), "its timer waits on no clock");
}

#[test]
fn an_audio_device_connecting_is_an_output_where_bluetooth_tells_of_them() {
    const A2DP: &str = "0000110b-0000-1000-8000-00805f9b34fb";
    let speaker = |id: &str, name: &str| BtDevice { id: id.into(), name: name.into(), services: vec![A2DP.into()], vendor: None, product: None };
    for outputs in [false, true] {
        let (_dir, session, fake, plugins) = open_hearing("plugins-devices-outputs", vec![speaker("OLD", "Kitchen")], outputs);
        // The test plugin hears devices connect and reaches none.
        plugins.install(&fixture("")).unwrap();
        plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
        assert_eq!(*fake.watching.lock().unwrap(), if outputs { vec![true] } else { Vec::new() }, "watched only where it tells of outputs");
        plugins.bluetooth_connected(BtDevice { services: vec![SPP.into()], ..device("BUDS", "Not audio") });
        plugins.bluetooth_connected(speaker("CAR", "Car"));
        let settle = || assert_eq!(nori_tasks::block_on(plugins.worker(nori_plugins::testing::FIXTURE).unwrap().call(|_, _| Ok(1))), Some(1));
        settle();
        assert_eq!(session.notice("heard"), outputs.then(|| vec!["device Car".to_string()]), "only an audio device that came, not one already there");
        plugins.bluetooth_disconnected("CAR".into());
        settle();
        assert_eq!(session.notice("heard"), outputs.then(|| vec!["Event::DeviceDisconnected(\"Car\")".to_string()]));
    }
}

#[test]
fn a_plugin_reaching_an_audio_device_it_hears_connect_is_told_once() {
    const A2DP: &str = "0000110b-0000-1000-8000-00805f9b34fb";
    let (_dir, session, _fake, plugins) = open_hearing("plugins-devices-once", Vec::new(), true);
    plugins.install(&fixture("headphones")).unwrap();
    plugins.turn(ID.into(), true);
    plugins.bluetooth_connected(BtDevice { services: vec![A2DP.into()], ..device("AA", "Test Buds Pro") });
    plugins.bluetooth_disconnected("AA".into());
    plugins.bluetooth_connected(BtDevice { services: vec![A2DP.into()], ..device("AA", "Test Buds Pro") });
    assert_eq!(nori_tasks::block_on(plugins.worker(ID).unwrap().call(|_, _| Ok(1))), Some(1));
    let said = session.said.lock().unwrap().clone();
    let count = |w: &str| said.iter().filter(|s| *s == w).count();
    assert_eq!((count("connected"), count("disconnected")), (2, 1), "by its id, not again as an output: {said:?}");
}
