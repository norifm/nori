//! Headphones over a made-up protocol, for the device control tests (`--features headphones`). Over
//! RFCOMM, `?` asks for a frame `[left, right, noise]` and `[N, noise]` sets the noise control; the
//! device sends frames of its own too. Over GATT, the battery service's level is the case's. Once it
//! controls a device it runs an `always` timer, heard as `tick`.

use nori_plugin_sdk::bluetooth::{connected, Gatt, Rfcomm};
use nori_plugin_sdk::device_control::{Battery, DeviceState, Noise};
use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::timers::{self, When};
use nori_plugin_sdk::{DeviceControl, DeviceFilter, Hooks, Manifest};

const SPP: &str = "00001101-0000-1000-8000-00805f9b34fb";
const BATTERY: &str = "0000180f-0000-1000-8000-00805f9b34fb";
const LEVEL: &str = "00002a19-0000-1000-8000-00805f9b34fb";
const TICK: u32 = 7;
const MODES: [Noise; 3] = [Noise::Off, Noise::Canceling, Noise::Transparency];

struct Buds {
    id: String,
    channel: Rfcomm,
    _gatt: Gatt,
    state: DeviceState,
}

#[derive(Default)]
pub struct Headphones {
    buds: Vec<Buds>,
}

fn said(words: &str, args: &[String]) {
    show(Tone::Info, words, args);
}

/// A frame into `state`.
fn frame(state: &mut DeviceState, f: &[u8]) {
    if let [left, right, noise] = *f {
        state.battery.left = Some(left);
        state.battery.right = Some(right);
        state.noise = MODES.get(noise as usize).copied();
    }
}

impl Headphones {
    fn buds(&mut self, id: &str) -> Option<&mut Buds> {
        self.buds.iter_mut().find(|b| b.id == id)
    }
}

impl DeviceControl for Headphones {
    fn handles(&mut self, device: String) -> bool {
        let seen: Vec<String> = connected().into_iter().map(|d| d.id).collect();
        let elsewhere = Rfcomm::open("CAR", SPP).err().map(|e| format!("{:?}", e.kind)).unwrap_or_default();
        said("reach", &[seen.join(","), elsewhere]);
        let (Ok(channel), Ok(gatt)) = (Rfcomm::open(&device, SPP), Gatt::open(&device)) else { return false };
        let mut state = DeviceState { battery: Battery { left: None, right: None, case: None, single: None }, noise: None, noise_modes: MODES.to_vec() };
        if channel.write(b"?").is_err() {
            return false;
        }
        match channel.read(3, 5_000) {
            Ok(f) => frame(&mut state, &f),
            Err(_) => return false,
        }
        state.battery.case = gatt.read(BATTERY, LEVEL).ok().and_then(|b| b.first().copied());
        let _ = gatt.notify(BATTERY, LEVEL, true);
        said("gatt", &[gatt.mtu().to_string(), gatt.services().join(",")]);
        self.buds.push(Buds { id: device, channel, _gatt: gatt, state });
        timers::start(TICK, 1000, true, When::Always);
        true
    }

    fn state(&mut self, device: String) -> Option<DeviceState> {
        self.buds(&device).map(|b| b.state.clone())
    }

    fn set_noise(&mut self, device: String, noise: Noise) {
        let Some(b) = self.buds(&device) else { return };
        if b.channel.write(&[b'N', noise as u8]).is_ok() {
            if let Ok(f) = b.channel.read(3, 5_000) {
                frame(&mut b.state, &f);
            }
        }
    }

    fn gone(&mut self, device: String) {
        self.buds.retain(|b| b.id != device);
    }
}

impl Hooks for Headphones {
    fn on(&mut self, event: Event) {
        match event {
            Event::DeviceConnected(id) => said("connected", &[id]),
            Event::DeviceDisconnected(id) => said("disconnected", &[id]),
            Event::Timer(id) => said("tick", &[id.to_string()]),
            Event::Notified((device, characteristic, bytes)) => {
                let Some(b) = self.buds(&device) else { return };
                if characteristic == SPP {
                    frame(&mut b.state, &bytes);
                } else if characteristic == LEVEL {
                    b.state.battery.case = bytes.first().copied();
                }
            }
            _ => {}
        }
    }
}

pub const MANIFEST: Manifest = Manifest { devices: &[DeviceFilter::ANY.name("Test Buds*")], ..Manifest::new("fm.nori.test.headphones") };
