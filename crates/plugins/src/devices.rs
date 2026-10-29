//! Device control: a connected Bluetooth device is offered to the enabled plugins whose filter
//! matches it, in their order, and the first that handles it controls it while it stays connected.
//! nori keeps what it last reported (battery, noise control) and asks again only when the device
//! said something or the user changed it: nothing polls. With no such plugin enabled the platform
//! watches nothing.

use std::sync::{Arc, Mutex};

use crate::bluetooth::{Bluetooth, BtDevice};
use crate::host::Side;
use crate::runtime::points::{device_control as dc, hooks};
use crate::view::PluginCard;
use crate::{Plugins, Store, Worker};

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
    /// The plugin controlling it: its name and its rows.
    pub card: PluginCard,
}

/// A device and the plugin controlling it.
pub(crate) struct Control {
    device: BtDevice,
    plugin: String,
    reported: Reported,
}

/// Which plugin controls which connected device.
#[derive(Default)]
pub(crate) struct Controls(Mutex<Vec<Control>>);

impl Controls {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Control>> {
        self.0.lock().expect("not poisoned")
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
    }
}

/// What a running plugin's device calls need beyond its instance.
#[derive(Clone)]
struct Around {
    controls: Arc<Controls>,
    side: Arc<dyn Side>,
}

impl Around {
    /// Asks the controlling plugin for `device`'s state and keeps it.
    fn refresh(&self, s: &mut Store, p: &crate::wit::Plugin, device: &str) -> wasmtime::Result<()> {
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
        let Some((id, w)) = rest.next() else { return };
        w.post(move |s, p| {
            if !p.nori_plugin_device_control().call_handles(&mut *s, &device.id)? {
                self.ask(rest, device);
                return Ok(());
            }
            if self.controls.take(&device, &id) {
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
        Some(o) => devices.iter().find(|d| d.name.trim().eq_ignore_ascii_case(o.trim()) || d.id.eq_ignore_ascii_case(o.trim())),
        None if devices.len() == 1 => devices.first(),
        None => None,
    }
}

/// [`output_device`] for a client over uniffi.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn controlled_output(devices: Vec<ControlledDevice>, output: Option<String>) -> Option<ControlledDevice> {
    output_device(&devices, output.as_deref()).cloned()
}

/// Whether a plugin's instance is there only for its devices, and so goes when the last one does.
fn only_for_devices(m: &crate::Manifest) -> bool {
    m.points.iter().all(|p| matches!(*p, "device-control" | "hooks"))
}

impl Plugins {
    fn around(&self) -> Around {
        Around { controls: self.controls.clone(), side: self.side.clone() }
    }

    /// The enabled plugins reaching `device` that extend `point`, in their order.
    fn reaching(&self, device: &BtDevice, point: &str) -> Vec<String> {
        self.installed().into_iter().filter(|i| i.enabled && i.manifest.extends(point) && i.manifest.reaches_device(device)).map(|i| i.manifest.id.clone()).collect()
    }

    fn running(&self, id: &str) -> Option<Worker> {
        self.workers.lock().expect("not poisoned").get(id).cloned()
    }

    /// Tells the hook plugins reaching `device` it came or went; one not running is started only for a
    /// device that came.
    fn tell(&self, device: &BtDevice, came: bool) {
        for id in self.reaching(device, "hooks") {
            let w = if came { self.worker(&id) } else { self.running(&id) };
            let event = if came { hooks::Event::DeviceConnected(device.id.clone()) } else { hooks::Event::DeviceDisconnected(device.id.clone()) };
            if let Some(w) = w {
                w.post(move |s, p| p.nori_plugin_hooks().call_on(s, &event));
            }
        }
    }

    /// Offers `device` to the plugins that may control it, unless one does.
    fn offer(&self, device: BtDevice) {
        if self.controls.of(&device.id).is_some() {
            return;
        }
        let askers: Vec<(String, Worker)> = self.reaching(&device, "device-control").into_iter().filter_map(|id| Some((id.clone(), self.worker(&id)?))).collect();
        self.around().ask(askers.into_iter(), device);
    }

    /// Watches Bluetooth while an enabled plugin reaches devices, and only then; called whenever the
    /// enabled plugins change. Devices no plugin controls are offered again, and a plugin turned off
    /// lets its devices go.
    pub(crate) fn rewatch(&self) {
        let Some(platform) = self.radio.platform() else { return };
        let enabled: Vec<String> = self.installed().into_iter().filter(|i| i.enabled && i.manifest.reaches_devices()).map(|i| i.manifest.id.clone()).collect();
        let want = !enabled.is_empty();
        let was = std::mem::replace(&mut *self.watching.lock().expect("not poisoned"), want);
        if want != was {
            platform.watch(want);
        }
        let let_go = {
            let mut c = self.controls.lock();
            let n = c.len();
            c.retain(|c| enabled.contains(&c.plugin));
            n != c.len()
        };
        if !want {
            self.radio.set_connected(Vec::new());
        } else if !was {
            self.radio.set_connected(Vec::new());
            for d in platform.connected() {
                self.bluetooth_connected(d);
            }
        } else {
            for d in self.radio.connected() {
                self.offer(d);
            }
        }
        if let_go {
            self.side.devices_changed();
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// The platform's Bluetooth, which plugins' devices are reached through. Set again (permission
    /// granted), it is asked for the connected devices anew.
    pub fn use_bluetooth(&self, platform: Arc<dyn Bluetooth>) {
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
        if !*self.watching.lock().expect("not poisoned") {
            return;
        }
        if self.radio.arrived(device.clone()) {
            self.tell(&device, true);
        }
        self.offer(device);
    }

    pub fn bluetooth_disconnected(&self, id: String) {
        let device = self.radio.connected().into_iter().find(|d| d.id == id);
        if !self.radio.went(&id) {
            return;
        }
        if let Some(d) = &device {
            self.tell(d, false);
        }
        let control = {
            let mut c = self.controls.lock();
            c.iter().position(|c| c.device.id == id).map(|at| c.remove(at))
        };
        if let Some(c) = control {
            if let Some(w) = self.running(&c.plugin) {
                let device = id.clone();
                w.post(move |s, p| p.nori_plugin_device_control().call_gone(s, &device));
            }
            let keeps = self.controls.lock().iter().any(|o| o.plugin == c.plugin);
            let only = self.installed().iter().any(|i| i.manifest.id == c.plugin && only_for_devices(&i.manifest));
            if !keeps && only {
                self.workers.lock().expect("not poisoned").remove(&c.plugin);
            }
            self.side.devices_changed();
        }
    }

    /// `bytes` came on `link`: a characteristic's notification (`characteristic`), or what an RFCOMM
    /// channel received (its `characteristic` empty). Its plugin hears it, then is asked for its
    /// device's state.
    pub fn bluetooth_heard(&self, link: u64, characteristic: String, bytes: Vec<u8>) {
        let Some(heard) = self.radio.heard(link, &bytes) else { return };
        let Some(w) = self.running(&heard.plugin) else { return };
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

    /// The platform lost `link`: reads end, writes fail.
    pub fn bluetooth_lost(&self, link: u64) {
        self.radio.lost(link);
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
                Some(ControlledDevice { id: d.id, name: d.name, level: r.battery.level(), battery: r.battery, noise: r.noise, noise_modes: r.modes, card })
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
        let r = Reported { battery: Battery { single: Some(5), ..Battery::default() }, noise: Some(Noise::Off), modes: vec![Noise::Off] };
        assert!(c.report("a", "one", r.clone()));
        assert!(!c.report("a", "one", r.clone()), "the same report again changes nothing");
        assert!(!c.report("a", "two", r), "only its controller reports");
    }
}
