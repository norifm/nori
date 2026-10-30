//! A deep sink's connection (`audio-out.to-socket` with `pace.deep`): the host speaks
//! nori-sink-protocol over the plugin's socket. Here: the handshake, the reader that hears the device,
//! and the frames it sends; the engine's cast output (`nori_engine::cast`) decides what to send and when.
//! What the device says of its buttons and knob goes to the plugin host's cast; the rest to the engine.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use nori_sink_protocol::{Button, Caps, Codec, Device, Format, Host as Frame};
use nori_sink_protocol::{Frames, VERSION};

/// How long the device has to answer Hello with its caps.
const CAPS_WAIT: Duration = Duration::from_secs(3);
/// How long a device may hold a write up before it is let go: it reads always, so only a dead one does.
const WRITE_WAIT: Duration = Duration::from_secs(5);
/// The kernel's send buffer: small, so nothing but the credit's audio waits in it and a control frame
/// goes out at once.
const SEND_BUFFER: usize = 64 * 1024;

/// What a device said, for whoever takes it.
pub type Hears = Box<dyn Fn(Option<Device>) + Send + Sync>;

/// Told when a burst starts (true) and ends.
pub(crate) type Bursts = Box<dyn Fn(bool) + Send + Sync>;

pub struct Deep {
    pub caps: Caps,
    /// The depth the plugin prefers, ms.
    pub depth_ms: Option<u32>,
    out: Mutex<Box<dyn Write + Send>>,
    /// The socket, to shut when the sink goes.
    socket: Option<TcpStream>,
    gone: AtomicBool,
    /// The engine's cast output: what the device says of its audio. None: it is gone.
    engine: Mutex<Option<Hears>>,
    /// The plugin host's cast: buttons and the knob.
    controls: Mutex<Option<Hears>>,
    /// Told when a burst starts and ends (the Wi-Fi lock).
    bursting: Mutex<Option<Bursts>>,
    /// Bytes encoded for one send.
    scratch: Mutex<Vec<u8>>,
}

impl Deep {
    /// A connection over `out` to a device that said `caps`; what the device says comes to
    /// [`Deep::heard`]. Tests make one over a buffer.
    pub fn new(caps: Caps, depth_ms: Option<u32>, out: Box<dyn Write + Send>) -> Arc<Deep> {
        Arc::new(Deep { caps, depth_ms, out: Mutex::new(out), socket: None, gone: AtomicBool::new(false), engine: Mutex::default(), controls: Mutex::default(), bursting: Mutex::default(), scratch: Mutex::default() })
    }

    /// Says Hello on `socket`, waits for the device's caps and reads it from then on, on a thread of its own.
    pub(crate) fn connect(socket: TcpStream, depth_ms: Option<u32>) -> Result<Arc<Deep>, String> {
        let _ = socket.set_nodelay(true);
        let _ = socket2::SockRef::from(&socket).set_send_buffer_size(SEND_BUFFER);
        let _ = socket.set_write_timeout(Some(WRITE_WAIT));
        let mut hello = Vec::new();
        Frame::Hello { version: VERSION }.encode(&mut hello);
        let mut s = &socket;
        s.write_all(&hello).map_err(|e| e.to_string())?;
        socket.set_read_timeout(Some(CAPS_WAIT)).map_err(|e| e.to_string())?;
        let mut frames = Frames::new();
        let mut buf = vec![0u8; 16 * 1024];
        let caps = loop {
            let n = s.read(&mut buf).map_err(|e| format!("no caps: {e}"))?;
            if n == 0 {
                return Err("the device closed before saying its caps".into());
            }
            frames.push(&buf[..n]);
            match frames.next_device() {
                Some(Ok(Device::Caps(c))) => break c,
                Some(Ok(other)) => return Err(format!("the device said {other:?} before its caps")),
                Some(Err(e)) => return Err(format!("the device's caps: {e:?}")),
                None => {}
            }
        };
        if caps.version != VERSION {
            return Err(format!("the device speaks version {}, nori {VERSION}", caps.version));
        }
        socket.set_read_timeout(None).map_err(|e| e.to_string())?;
        let out = socket.try_clone().map_err(|e| e.to_string())?;
        let reader = socket.try_clone().map_err(|e| e.to_string())?;
        let deep = Arc::new(Deep { caps, depth_ms, out: Mutex::new(Box::new(out)), socket: Some(socket), gone: AtomicBool::new(false), engine: Mutex::default(), controls: Mutex::default(), bursting: Mutex::default(), scratch: Mutex::default() });
        let weak = Arc::downgrade(&deep);
        std::thread::Builder::new()
            .name("nori-sink-read".into())
            .spawn(move || read(reader, frames, buf, weak))
            .map_err(|e| e.to_string())?;
        Ok(deep)
    }

    /// Sends `frame`; false once the device is gone.
    pub fn send(&self, frame: Frame) -> bool {
        let mut b = self.scratch.lock().expect("not poisoned");
        b.clear();
        frame.encode(&mut b);
        self.send_raw(&b)
    }

    /// Sends frames already encoded, whole; false once the device is gone.
    pub fn send_raw(&self, frames: &[u8]) -> bool {
        if self.gone() {
            return false;
        }
        let mut out = self.out.lock().expect("not poisoned");
        if out.write_all(frames).and_then(|_| out.flush()).is_err() {
            drop(out);
            self.lost();
            return false;
        }
        true
    }

    pub fn gone(&self) -> bool {
        self.gone.load(Ordering::Acquire)
    }

    /// The socket closed or failed.
    pub fn lost(&self) {
        if !self.gone.swap(true, Ordering::AcqRel) {
            for to in [&self.engine, &self.controls] {
                if let Some(h) = to.lock().expect("not poisoned").as_ref() {
                    h(None);
                }
            }
        }
    }

    /// The device said `frame`.
    pub fn heard(&self, frame: Device) {
        let to = match frame {
            Device::Press(_) | Device::Volume { .. } => &self.controls,
            _ => &self.engine,
        };
        if let Some(h) = to.lock().expect("not poisoned").as_ref() {
            h(Some(frame));
        }
    }

    /// What the device says of its audio goes to `hears` from now on.
    pub fn listen(&self, hears: Hears) {
        *self.engine.lock().expect("not poisoned") = Some(hears);
    }

    pub(crate) fn controls(&self, hears: Hears) {
        *self.controls.lock().expect("not poisoned") = Some(hears);
    }

    pub(crate) fn on_burst(&self, told: Bursts) {
        *self.bursting.lock().expect("not poisoned") = Some(told);
    }

    /// A burst starts (true) or is all sent (false).
    pub fn bursting(&self, on: bool) {
        if let Some(b) = self.bursting.lock().expect("not poisoned").as_ref() {
            b(on);
        }
    }
}

impl Drop for Deep {
    fn drop(&mut self) {
        if let Some(s) = &self.socket {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

/// Reads the device until its socket ends or the connection is let go.
fn read(mut socket: TcpStream, mut frames: Frames, mut buf: Vec<u8>, deep: std::sync::Weak<Deep>) {
    loop {
        // Frames that came with the caps first.
        loop {
            let Some(d) = deep.upgrade() else { return };
            match frames.next_device() {
                Some(Ok(f)) => d.heard(f),
                Some(Err(e)) => {
                    nori_model::alog::info(&format!("deep sink: {e:?}: let go"));
                    return d.lost();
                }
                None => break,
            }
        }
        match socket.read(&mut buf) {
            Ok(n) if n > 0 => frames.push(&buf[..n]),
            _ => {
                if let Some(d) = deep.upgrade() {
                    d.lost();
                }
                return;
            }
        }
    }
}
