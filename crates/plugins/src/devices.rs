//! Device control: a connected Bluetooth device is offered to the enabled plugins whose filter
//! matches it, in their order, and the first that handles it controls it while it stays connected.
//! nori keeps what it last reported (battery, noise control) and asks again only when the device
//! said something or the user changed it: nothing polls. With no such plugin enabled the platform
//! watches nothing.

use nori_time::Duration;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::bluetooth::{Bluetooth, BtDevice};
use crate::host::Side;
use crate::manifest::Hook;
use crate::timers::{Active, Gate};
use crate::runtime::points::{device_control as dc, hooks};
use crate::view::PluginCard;
use crate::{Plugins, Store, Worker};

/// The services of a device sound plays on: A2DP's audio sink and hands-free.
const AUDIO: [&str; 2] = ["0000110b-0000-1000-8000-00805f9b34fb", "0000111e-0000-1000-8000-00805f9b34fb"];

/// A device's noise control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Noise {
    Off,
    Canceling,
    Transparency,
    Adaptive,
}

/// Charge in percent, per part the device has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Battery {
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub single: Option<u8>,
}

impl Battery {
    /// The one level a short line shows: a single battery's, else the emptier bud's; never the case's.
    pub fn level(&self) -> Option<u8> {
        self.single.or(match (self.left, self.right) {
            (Some(l), Some(r)) => Some(l.min(r)),
            (l, r) => l.or(r),
        })
    }
}

/// What a plugin last reported of a device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reported {
    battery: Battery,
    noise: Option<Noise>,
    modes: Vec<Noise>,
    cycle: Vec<Noise>,
}

/// A connected device a plugin controls, as every client draws it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ControlledDevice {
    pub id: String,
    pub name: String,
    pub battery: Battery,
    /// The level the output row shows beside the device's name ([`Battery::level`]).
    pub level: Option<u8>,
    /// None when it hasn't said, or has no noise control.
    pub noise: Option<Noise>,
    /// The modes it offers, in its order.
    pub noise_modes: Vec<Noise>,
    /// Those its noise button moves through.
    pub noise_cycle: Vec<Noise>,
    /// The plugin controlling it: its name and its rows.
    pub card: PluginCard,
}

/// A device and the plugin controlling it.
pub(crate) struct Control {
    device: BtDevice,
    plugin: String,
    reported: Reported,
}

/// How long a device nobody took is left before it is offered again. Nothing says when one that just
/// connected, or whose channel reset, can be talked to.
const OFFER_AGAIN: [Duration; 4] = [Duration::from_millis(1_000), Duration::from_millis(3_000), Duration::from_millis(8_000), Duration::from_millis(20_000)];

/// Which plugin controls which connected device, the offers under way, and how often each device was
/// offered again.
#[derive(Default)]
pub(crate) struct Controls(Mutex<Vec<Control>>, Mutex<std::collections::BTreeSet<String>>, Mutex<std::collections::BTreeMap<String, usize>>);

impl Controls {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Control>> {
        self.0.lock().expect("not poisoned")
    }

    /// `plugin` no longer controls anything; whether it did.
    fn let_go(&self, plugin: &str) -> bool {
        let mut c = self.lock();
        let had = c.len();
        c.retain(|c| c.plugin != plugin);
        c.len() != had
    }

    /// `device` is being offered to the plugins from now on, unless it already is; whether it now is.
    fn offering(&self, device: &str) -> bool {
        self.1.lock().expect("not poisoned").insert(device.to_string())
    }

    /// The offer of `device` ended.
    fn offered(&self, device: &str) {
        self.1.lock().expect("not poisoned").remove(device);
    }

    /// How long to leave `device` before offering it again; None once it has had its tries.
    fn next_try(&self, device: &str) -> Option<Duration> {
        let mut tries = self.2.lock().expect("not poisoned");
        let n = tries.entry(device.to_string()).or_insert(0);
        let wait = OFFER_AGAIN.get(*n).copied();
        *n += 1;
        wait
    }

    /// `device` has all its tries again.
    fn fresh_tries(&self, device: &str) {
        self.2.lock().expect("not poisoned").remove(device);
    }

    /// The devices `plugin` controls.
    pub(crate) fn of_plugin(&self, plugin: &str) -> Vec<String> {
        self.lock().iter().filter(|c| c.plugin == plugin).map(|c| c.device.id.clone()).collect()
    }

    fn of(&self, device: &str) -> Option<String> {
        self.lock().iter().find(|c| c.device.id == device).map(|c| c.plugin.clone())
    }

    /// `plugin` controls `device`, unless another already does; whether it does now.
    fn take(&self, device: &BtDevice, plugin: &str) -> bool {
        let mut c = self.lock();
        match c.iter().find(|c| c.device.id == device.id) {
            Some(c) => c.plugin == plugin,
            None => {
                c.push(Control { device: device.clone(), plugin: plugin.into(), reported: Reported::default() });
                true
            }
        }
    }

    /// What `plugin` reported of `device`; whether that changed what is shown.
    fn report(&self, device: &str, plugin: &str, reported: Reported) -> bool {
        let mut c = self.lock();
        match c.iter_mut().find(|c| c.device.id == device && c.plugin == plugin) {
            Some(c) if c.reported != reported => {
                c.reported = reported;
                true
            }
            _ => false,
        }
    }
}

fn noise_in(n: dc::Noise) -> Noise {
    match n {
        dc::Noise::Off => Noise::Off,
        dc::Noise::Canceling => Noise::Canceling,
        dc::Noise::Transparency => Noise::Transparency,
        dc::Noise::Adaptive => Noise::Adaptive,
    }
}

fn noise_out(n: Noise) -> dc::Noise {
    match n {
        Noise::Off => dc::Noise::Off,
        Noise::Canceling => dc::Noise::Canceling,
        Noise::Transparency => dc::Noise::Transparency,
        Noise::Adaptive => dc::Noise::Adaptive,
    }
}

fn reported(s: Option<dc::DeviceState>) -> Reported {
    let Some(s) = s else { return Reported::default() };
    let percent = |v: Option<u8>| v.map(|v| v.min(100));
    let b = s.battery;
    Reported {
        battery: Battery { left: percent(b.left), right: percent(b.right), case: percent(b.case), single: percent(b.single) },
        noise: s.noise.map(noise_in),
        modes: s.noise_modes.into_iter().map(noise_in).collect(),
        cycle: s.noise_cycle.into_iter().map(noise_in).collect(),
    }
}

/// What a running plugin's device calls need beyond its instance.
#[derive(Clone)]
pub(crate) struct Around {
    controls: Arc<Controls>,
    side: Arc<dyn Side>,
    gate: Arc<Gate>,
    me: std::sync::Weak<Plugins>,
}

impl Around {
    /// Asks the controlling plugin for `device`'s state and keeps it.
    pub(crate) fn refresh(&self, s: &mut Store, p: &crate::wit::Plugin, device: &str) -> wasmtime::Result<()> {
        let plugin = s.data().id.clone();
        if self.controls.of(device).as_deref() != Some(&plugin) {
            return Ok(());
        }
        let state = p.nori_plugin_device_control().call_state(&mut *s, device)?;
        if self.controls.report(device, &plugin, reported(state)) {
            self.side.devices_changed();
        }
        Ok(())
    }

    /// Asks the plugins in `rest`, in turn, whether one handles `device`; the first that does controls it.
    fn ask(self, mut rest: std::vec::IntoIter<(String, Worker)>, device: BtDevice) {
        let Some((id, w)) = rest.next() else {
            self.controls.offered(&device.id);
            if let Some(p) = self.me.upgrade() {
                p.offer_again(device);
            }
            return;
        };
        w.post(move |s, p| {
            let handles = p.nori_plugin_device_control().call_handles(&mut *s, &device.id);
            if !matches!(handles, Ok(true)) {
                self.ask(rest, device);
                return handles.map(|_| ());
            }
            self.controls.offered(&device.id);
            self.controls.fresh_tries(&device.id);
            // It went, or its plugin was turned off, while the plugin was answering.
            if !self.me.upgrade().is_some_and(|me| me.may_control(&id, &device.id)) {
                return p.nori_plugin_device_control().call_gone(s, &device.id);
            }
            if self.controls.take(&device, &id) {
                self.gate.active(&id, Active::Device, true);
                self.side.devices_changed();
                self.refresh(s, p, &device.id)?;
            }
            Ok(())
        });
    }
}

/// The controlled device the music plays on: the one named `output` (by its name or id), or for the
/// system's default output (None) the only one connected.
pub fn output_device<'a>(devices: &'a [ControlledDevice], output: Option<&str>) -> Option<&'a ControlledDevice> {
    match output {
        Some(o) => {
            let o = nori_player::outputs::parts(o).1.unwrap_or(o).trim();
            devices.iter().find(|d| d.name.trim().eq_ignore_ascii_case(o) || d.id.eq_ignore_ascii_case(o))
        }
        None if devices.len() == 1 => devices.first(),
        None => None,
    }
}

/// The mode after `now` among the `modes` a device offers, the first after the last or when it hasn't said.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn next_noise(modes: Vec<Noise>, now: Option<Noise>) -> Option<Noise> {
    let next = now.and_then(|n| modes.iter().position(|m| *m == n)).map_or(0, |at| at + 1);
    modes.get(next % modes.len().max(1)).copied()
}

/// The controlled device on this phone's output `output` (its key): none while the music plays on the
/// phone's own speaker or wired headphones, whatever else is connected.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn controlled_on_output(devices: Vec<ControlledDevice>, output: String) -> Option<ControlledDevice> {
    let (port, name) = nori_player::outputs::parts(&output);
    if matches!(port, nori_player::outputs::OutputPort::Speaker | nori_player::outputs::OutputPort::Wired) {
        return None;
    }
    output_device(&devices, name).cloned()
}

/// Whether a plugin's instance is there only for its devices, and so goes when the last one does.
fn only_for_devices(m: &crate::Manifest) -> bool {
    m.points.iter().all(|p| matches!(*p, "device-control" | "hooks" | "lifecycle"))
}

impl Plugins {
    pub(crate) fn around(&self) -> Around {
        Around { controls: self.controls.clone(), side: self.side.clone(), gate: self.gate.clone(), me: self.me.clone() }
    }

    /// The enabled plugins reaching `device` that extend `point`, in their order.
    fn reaching(&self, device: &BtDevice, point: &str) -> Vec<String> {
        self.running().into_iter().filter(|i| i.manifest.extends(point) && i.manifest.reaches_device(device)).map(|i| i.manifest.id.clone()).collect()
    }

    fn live_worker(&self, id: &str) -> Option<Worker> {
        self.workers.lock().expect("not poisoned").get(id).cloned()
    }

    /// Tells the hook plugins reaching `device` it came or went; one not running is started only for a
    /// device that came.
    fn tell_device(&self, device: &BtDevice, came: bool) {
        for id in self.reaching(device, "hooks") {
            let w = if came { self.worker(&id) } else { self.live_worker(&id) };
            let event = if came { hooks::Event::DeviceConnected(device.id.clone()) } else { hooks::Event::DeviceDisconnected(device.id.clone()) };
            if let Some(w) = w {
                w.post(move |s, p| p.nori_plugin_hooks().call_on(s, &event));
            }
        }
    }

    /// Offers `device` to the plugins that may control it, unless one does.
    fn offer(&self, device: BtDevice) {
        if self.controls.of(&device.id).is_some() || !self.controls.offering(&device.id) {
            return;
        }
        let askers: Vec<(String, Worker)> = self.reaching(&device, "device-control").into_iter().filter_map(|id| Some((id.clone(), self.worker(&id)?))).collect();
        if askers.is_empty() {
            self.controls.offered(&device.id);
            return;
        }
        self.around().ask(askers.into_iter(), device);
    }

    /// Whether plugin `id` is running and `device` still connected.
    fn may_control(&self, id: &str, device: &str) -> bool {
        self.running().iter().any(|i| i.manifest.id == id) && self.radio.connected().iter().any(|d| d.id == device)
    }

    /// Offers `device` again in a while, if it has tries left and is still connected by then; out of tries,
    /// plugins there only for devices and controlling none stop running.
    fn offer_again(&self, device: BtDevice) {
        let Some(wait) = self.controls.next_try(&device.id) else {
            let idle = |id: &String| self.controls.of_plugin(id).is_empty() && self.installed().iter().any(|i| i.manifest.id == *id && only_for_devices(&i.manifest));
            let ids = self.reaching(&device, "device-control");
            self.workers.lock().expect("not poisoned").retain(|id, _| !(ids.contains(id) && idle(id)));
            return;
        };
        let (tasks, me) = (self.tasks(), self.me.clone());
        let task = async move {
            tasks.sleep(wait).await;
            let Some(p) = me.upgrade() else { return };
            if let Some(d) = p.radio.connected().into_iter().find(|d| d.id == device.id) {
                p.offer(d);
            }
        };
        self.tasks().spawn(Box::pin(task));
    }

    /// Whoever controls `id` no longer does and hears `gone`; which plugin that was.
    fn release(&self, id: &str) -> Option<String> {
        let control = {
            let mut c = self.controls.lock();
            c.iter().position(|c| c.device.id == id).map(|at| c.remove(at))
        }?;
        if let Some(w) = self.live_worker(&control.plugin) {
            let device = id.to_string();
            w.post(move |s, p| p.nori_plugin_device_control().call_gone(s, &device));
        }
        if !self.controls.lock().iter().any(|o| o.plugin == control.plugin) {
            self.gate.active(&control.plugin, Active::Device, false);
        }
        self.side.devices_changed();
        Some(control.plugin)
    }

    /// `device` is connected: the plugins reaching it hear so and it is offered to them; with `came`
    /// (not one already there when the watch began) it is also an output connecting.
    fn arrived(&self, device: BtDevice, came: bool) {
        if self.radio.arrived(device.clone()) {
            self.controls.fresh_tries(&device.id);
            self.tell_device(&device, true);
            if came {
                self.output_device(&device, true);
            }
        }
        self.offer(device);
    }

    /// An audio device connected or went, as the `device-connected` hook hears outputs, where the
    /// platform's Bluetooth is how this client hears them; not to the plugins reaching it, which heard of
    /// it by its id ([`Plugins::tell_device`]).
    fn output_device(&self, device: &BtDevice, connected: bool) {
        let audio = device.services.iter().any(|s| AUDIO.iter().any(|a| s.eq_ignore_ascii_case(a)));
        if audio && self.bluetooth_outputs.load(Ordering::Relaxed) {
            let told = self.reaching(device, "hooks");
            let name = device.name.clone();
            self.tell_but(Hook::DeviceConnected, &told, || if connected { hooks::Event::DeviceConnected(name) } else { hooks::Event::DeviceDisconnected(name) });
        }
    }

    /// `plugin`'s instance is a new one, which knows nothing of the devices the last controlled: they are
    /// offered to it again.
    pub(crate) fn reoffer(&self, plugin: &str) {
        if self.controls.let_go(plugin) {
            self.gate.active(plugin, Active::Device, false);
            self.side.devices_changed();
            self.rewatch();
        }
    }

    /// Watches Bluetooth while an enabled plugin reaches devices, or hears outputs connect where
    /// Bluetooth tells of them, and only then; called whenever the enabled plugins change. Devices no
    /// plugin controls are offered again, and a plugin turned off lets its devices go.
    pub(crate) fn rewatch(&self) {
        let Some(platform) = self.radio.platform() else { return };
        let enabled: Vec<String> = self.running().into_iter().filter(|i| i.manifest.reaches_devices()).map(|i| i.manifest.id.clone()).collect();
        let want = !enabled.is_empty() || (self.bluetooth_outputs.load(Ordering::Relaxed) && self.hears(Hook::DeviceConnected));
        let was = std::mem::replace(&mut *self.watching.lock().expect("not poisoned"), want);
        // Before watching: a platform may tell of connected devices as soon as it is watched.
        if want && !was {
            self.radio.set_connected(Vec::new());
        }
        if want != was {
            platform.watch(want);
        }
        let let_go: Vec<String> = {
            let mut c = self.controls.lock();
            let gone = c.iter().filter(|c| !enabled.contains(&c.plugin)).map(|c| c.plugin.clone()).collect();
            c.retain(|c| enabled.contains(&c.plugin));
            gone
        };
        for plugin in &let_go {
            self.gate.active(plugin, Active::Device, false);
        }
        if !want {
            self.radio.set_connected(Vec::new());
        } else if !was {
            for d in platform.connected() {
                self.arrived(d, false);
            }
        } else {
            for d in self.radio.connected() {
                self.offer(d);
            }
        }
        if !let_go.is_empty() {
            self.side.devices_changed();
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// The platform's Bluetooth, which plugins' devices are reached through; with `outputs`, it is also
    /// how this client hears output devices connect (the `device-connected` hook). Set again
    /// (permission granted), it is asked for the connected devices anew.
    pub fn use_bluetooth(&self, platform: Arc<dyn Bluetooth>, outputs: bool) {
        self.bluetooth_outputs.store(outputs, Ordering::Relaxed);
        self.radio.set_platform(platform);
        *self.watching.lock().expect("not poisoned") = false;
        self.rewatch();
    }

    /// Whether an installed plugin reaches Bluetooth devices: what the platform's permission is for.
    pub fn reaches_bluetooth(&self) -> bool {
        self.installed().iter().any(|i| i.manifest.reaches_devices())
    }

    /// A device connected (or what it says of itself changed).
    pub fn bluetooth_connected(&self, device: BtDevice) {
        if *self.watching.lock().expect("not poisoned") {
            self.arrived(device, true);
        }
    }

    pub fn bluetooth_disconnected(&self, id: String) {
        self.controls.offered(&id);
        self.controls.fresh_tries(&id);
        let device = self.radio.connected().into_iter().find(|d| d.id == id);
        if !self.radio.went(&id) {
            return;
        }
        if let Some(d) = &device {
            self.tell_device(d, false);
            self.output_device(d, false);
        }
        if let Some(plugin) = self.release(&id) {
            let keeps = self.controls.lock().iter().any(|o| o.plugin == plugin);
            let only = self.installed().iter().any(|i| i.manifest.id == plugin && only_for_devices(&i.manifest));
            if !keeps && only {
                self.workers.lock().expect("not poisoned").remove(&plugin);
            }
        }
    }

    /// `bytes` came on `link`: a characteristic's notification (`characteristic`), or what an RFCOMM
    /// channel received (its `characteristic` empty). Its plugin hears it, then is asked for its
    /// device's state.
    pub fn bluetooth_heard(&self, link: u64, characteristic: String, bytes: Vec<u8>) {
        let Some(heard) = self.radio.heard(link, &bytes) else { return };
        let Some(w) = self.live_worker(&heard.plugin) else { return };
        let (around, radio) = (self.around(), self.radio.clone());
        w.post(move |s, p| {
            // An RFCOMM channel's bytes a read took already were the answer it waited for.
            let (said, bytes) = match heard.rfcomm {
                Some(service) => (service, radio.take(link)),
                None => (characteristic, bytes),
            };
            if bytes.is_empty() {
                return Ok(());
            }
            if s.data().manifest.extends("hooks") {
                p.nori_plugin_hooks().call_on(&mut *s, &hooks::Event::Notified((heard.device.clone(), said, bytes)))?;
            }
            around.refresh(s, p, &heard.device)
        });
    }

    /// Offers the connected devices nobody controls again: something else may have let go of one (another
    /// app on its channel) with nothing to say so.
    pub fn look_for_devices(&self) {
        for d in self.radio.connected() {
            self.controls.fresh_tries(&d.id);
            self.offer(d);
        }
    }

    /// The platform lost `link`: reads end, writes fail. A device can lose its channel and stay connected
    /// (headphones changing codec): whoever controlled it lets go and it is offered again.
    pub fn bluetooth_lost(&self, link: u64) {
        let Some(device) = self.radio.lost(link) else { return };
        if self.release(&device).is_some() {
            self.controls.fresh_tries(&device);
            if let Some(d) = self.radio.connected().into_iter().find(|d| d.id == device) {
                self.offer_again(d);
            }
        }
    }

    /// The connected devices plugins control, in the order they connected, in `lang`.
    pub fn controlled(&self, lang: String) -> Vec<ControlledDevice> {
        let shown: Vec<(BtDevice, String, Reported)> = self.controls.lock().iter().map(|c| (c.device.clone(), c.plugin.clone(), c.reported.clone())).collect();
        if shown.is_empty() {
            return Vec::new();
        }
        let cards = self.cards(lang);
        shown
            .into_iter()
            .filter_map(|(d, plugin, r)| {
                let card = cards.iter().find(|c| c.id == plugin)?.clone();
                Some(ControlledDevice { id: d.id, name: d.name, level: r.battery.level(), battery: r.battery, noise: r.noise, noise_modes: r.modes, noise_cycle: r.cycle, card })
            })
            .collect()
    }

    /// Sets `device`'s noise control through the plugin controlling it.
    pub fn set_noise(&self, device: String, noise: Noise) {
        let Some(plugin) = self.controls.of(&device) else { return };
        let Some(w) = self.worker(&plugin) else { return };
        let around = self.around();
        w.post(move |s, p| {
            p.nori_plugin_device_control().call_set_noise(&mut *s, &device, noise_out(noise))?;
            around.refresh(s, p, &device)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_on_the_output_only_when_the_output_is_not_the_phones_own() {
        let device = |id: &str, name: &str| ControlledDevice {
            id: id.into(),
            name: name.into(),
            battery: Battery::default(),
            level: None,
            noise: None,
            noise_modes: vec![], noise_cycle: vec![],
            card: PluginCard {
                id: "p".into(),
                version: "1".into(),
                name: "p".into(),
                about: None,
                enabled: true,
                repo: None,
                held: false,
                points: vec![],
                overrides: vec![],
                net: vec![],
                bluetooth: true,
                lan: false,
                permissions: vec![],
                hooks: vec![],
                wakes: vec![],
                commands: vec![],
                cache_bytes: 0,
                asset_bytes: 0,
                rows: vec![],
            },
        };
        let one = || vec![device("AA", "soundcore Liberty 4 NC")];
        for (output, on) in [
            ("Bluetooth: soundcore Liberty 4 NC", true),
            ("Bluetooth: Other Buds", false),
            ("Phone speaker", false),
            ("Wired headphones", false),
        ] {
            assert_eq!(controlled_on_output(one(), output.to_string()).is_some(), on, "{output}");
        }
    }

    #[test]
    fn the_next_noise_mode_goes_round_those_the_device_offers() {
        use Noise::{Adaptive, Canceling, Off, Transparency};
        let three = || vec![Canceling, Off, Transparency];
        for (modes, now, next) in [
            (three(), Some(Canceling), Some(Off)),
            (three(), Some(Transparency), Some(Canceling)),
            (three(), None, Some(Canceling)),
            (three(), Some(Adaptive), Some(Canceling)),
            (vec![Off], Some(Off), Some(Off)),
            (vec![], Some(Off), None),
        ] {
            assert_eq!(next_noise(modes.clone(), now), next, "{modes:?} {now:?}");
        }
    }

    #[test]
    fn the_shown_level_is_the_single_battery_else_the_emptier_bud() {
        let b = |left, right, case, single| Battery { left, right, case, single };
        for (battery, level) in [
            (b(None, None, None, Some(80)), Some(80)),
            (b(Some(60), Some(40), Some(100), None), Some(40)),
            (b(Some(60), None, Some(100), None), Some(60)),
            (b(None, None, Some(100), None), None),
        ] {
            assert_eq!(battery.level(), level, "{battery:?}");
        }
    }

    #[test]
    fn a_device_is_the_first_handlers_and_reports_change_once() {
        let c = Controls::default();
        let d = BtDevice { id: "a".into(), name: "WH".into(), services: Vec::new(), vendor: None, product: None };
        assert!(c.take(&d, "one") && !c.take(&d, "two") && c.take(&d, "one"));
        assert_eq!(c.of("a").as_deref(), Some("one"));
        let r = Reported { battery: Battery { single: Some(5), ..Battery::default() }, noise: Some(Noise::Off), modes: vec![Noise::Off], cycle: vec![Noise::Off] };
        assert!(c.report("a", "one", r.clone()));
        assert!(!c.report("a", "one", r.clone()), "the same report again changes nothing");
        assert!(!c.report("a", "two", r), "only its controller reports");
    }
}
