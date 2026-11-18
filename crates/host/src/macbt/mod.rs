//! Plugins' Bluetooth on macOS: classic devices and their RFCOMM channels through IOBluetooth
//! ([`classic`]), GATT through CoreBluetooth ([`le`]). A device is its classic address; a device only on
//! Bluetooth LE is its CoreBluetooth identifier. A GATT link to a classic device is to the LE peripheral
//! of the same name.

mod classic;
mod le;

use std::sync::Weak;
use std::time::Duration;

use nori_plugins::{Bluetooth, BtDevice, BtFailure, Plugins};

use crate::btids;

/// Runs `app` on a thread of its own while the main thread, which calls this, runs its run loop: IOBluetooth
/// tells of devices and channels only there. For a client whose main thread runs nothing else; returns
/// what `app` returns.
pub fn beside_main_run_loop<R: Send + 'static>(app: impl FnOnce() -> R + Send + 'static) -> R {
    /// Stops the main run loop once the app ends, returned or panicked.
    struct Ended;
    impl Drop for Ended {
        fn drop(&mut self) {
            dispatch2::DispatchQueue::main().exec_async(|| {
                if let Some(main) = objc2_core_foundation::CFRunLoop::current() {
                    main.stop();
                }
            });
        }
    }
    let app = std::thread::Builder::new()
        .name("nori-app".into())
        // As much as the main thread has.
        .stack_size(8 << 20)
        .spawn(move || {
            let _ended = Ended;
            app()
        })
        .expect("a thread for the app");
    objc2_core_foundation::CFRunLoop::run();
    app.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// The longest a channel, a connection, a device's services or a scan for it take to come.
const SLOWEST: Duration = Duration::from_secs(10);

/// What a failed IOKit call (`IOReturn`) means.
fn io_failed(status: i32) -> BtFailure {
    match status as u32 {
        0xe000_02d6 => BtFailure::Timeout,
        0xe000_02e2 | 0xe000_02c1 => BtFailure::Refused,
        0xe000_02c7 => BtFailure::Unsupported,
        0xe000_02f0 => BtFailure::NotFound,
        _ => BtFailure::Failed { detail: format!("IOReturn {status:#x}") },
    }
}

fn io(status: i32) -> Result<(), BtFailure> {
    if status == 0 {
        Ok(())
    } else {
        Err(io_failed(status))
    }
}

/// IOBluetooth and CoreBluetooth as the plugin host's Bluetooth.
pub struct MacBluetooth {
    classic: classic::Classic,
    le: le::Le,
}

impl MacBluetooth {
    pub fn new(plugins: Weak<Plugins>) -> MacBluetooth {
        MacBluetooth { classic: classic::Classic::new(plugins.clone()), le: le::Le::new(plugins) }
    }
}

impl Bluetooth for MacBluetooth {
    fn watch(&self, on: bool) {
        self.classic.watch(on);
        self.le.watch(on);
    }

    fn connected(&self) -> Vec<BtDevice> {
        btids::merged(self.classic.connected(), self.le.connected())
    }

    fn rfcomm(&self, link: u64, device: String, service: String) -> Result<(), BtFailure> {
        self.classic.rfcomm(link, device, service)
    }

    fn rfcomm_write(&self, link: u64, bytes: Vec<u8>) -> Result<(), BtFailure> {
        self.classic.write(link, bytes)
    }

    fn gatt(&self, link: u64, device: String) -> Result<(), BtFailure> {
        let name = if le::is_identifier(&device) { None } else { Some(self.classic.name(device.clone()).ok_or(BtFailure::NotFound)?) };
        self.le.gatt(link, device, name)
    }

    fn gatt_mtu(&self, link: u64) -> u16 {
        self.le.mtu(link)
    }

    fn gatt_services(&self, link: u64) -> Vec<String> {
        self.le.services(link)
    }

    fn gatt_read(&self, link: u64, service: String, characteristic: String) -> Result<Vec<u8>, BtFailure> {
        self.le.read(link, &service, &characteristic)
    }

    fn gatt_write(&self, link: u64, service: String, characteristic: String, bytes: Vec<u8>, with_response: bool) -> Result<(), BtFailure> {
        self.le.write(link, &service, &characteristic, bytes, with_response)
    }

    fn gatt_notify(&self, link: u64, service: String, characteristic: String, on: bool) -> Result<(), BtFailure> {
        self.le.notify(link, &service, &characteristic, on)
    }

    fn close(&self, link: u64) {
        if !self.classic.close(link) {
            self.le.close(link);
        }
    }
}
