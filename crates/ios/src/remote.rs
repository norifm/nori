//! Remote control on the iPod: Bonjour through the app (`NetService`, `ios/Sources/Devices.swift`) for
//! the account's nearby devices, and the list of devices the music can move to.

use std::ffi::{c_char, CString};
use std::sync::{Arc, Mutex};

use nori_core::remote::wire::Refusal;
use nori_core::remote::{Announcement, Discovery, PairError, Remote, RemoteDevice};
use nori_core::Param;
use serde_json::{json, Map, Value};

use crate::pages::owned;
use crate::session::{c_text, report, with_session, REPORT_PAIRED};

/// Announces this device's door: `name`, `port` and its TXT record as a JSON object of strings; `name`
/// NULL withdraws it.
pub type AnnounceFn = unsafe extern "C" fn(name: *const c_char, port: u16, txt: *const c_char);
/// Looks for other doors while `on` is 1, reporting them to [`nori_ios_lan_found`] and [`nori_ios_lan_lost`].
pub type BrowseFn = unsafe extern "C" fn(on: i32);

/// The app's Bonjour. C callbacks carry no handle, so they live in a process-wide slot.
static HOOKS: Mutex<Option<(AnnounceFn, BrowseFn)>> = Mutex::new(None);

fn hooks() -> Option<(AnnounceFn, BrowseFn)> {
    *HOOKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the remote's mDNS goes. NULL clears it.
///
/// # Safety
/// Both, when set, are functions the process keeps while they stay set.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_on_bonjour(announce: Option<AnnounceFn>, browse: Option<BrowseFn>) {
    *HOOKS.lock().unwrap_or_else(|e| e.into_inner()) = announce.zip(browse);
}

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

/// [`Discovery`] over the app's Bonjour.
pub(crate) struct Bonjour;

impl Discovery for Bonjour {
    fn announce(&self, door: Option<Announcement>) {
        let Some((announce, _)) = hooks() else { return };
        match door {
            Some(d) => {
                let txt: Map<String, Value> = d.txt.into_iter().map(|p| (p.key, Value::String(p.value))).collect();
                let (name, txt) = (c(&d.name), c(&Value::Object(txt).to_string()));
                // SAFETY: the strings live through the call; the app copies what it keeps.
                unsafe { announce(name.as_ptr(), d.port, txt.as_ptr()) };
            }
            // SAFETY: NULL is the withdrawal the hook expects.
            None => unsafe { announce(std::ptr::null(), 0, std::ptr::null()) },
        }
    }

    fn browse(&self, on: bool) {
        if let Some((_, browse)) = hooks() {
            // SAFETY: a plain call into the app.
            unsafe { browse(i32::from(on)) };
        }
    }
}

fn remote() -> Option<Arc<Remote>> {
    with_session(|s| s.remote()).flatten()
}

/// A door Bonjour resolved: service `service` at `host`:`port`, its TXT record as a JSON object of strings.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lan_found(service: *const c_char, host: *const c_char, port: u16, txt: *const c_char) {
    let txt: Map<String, Value> = serde_json::from_str(&c_text(txt)).unwrap_or_default();
    let txt = txt.into_iter().filter_map(|(key, v)| v.as_str().map(|v| Param { key, value: v.into() })).collect();
    if let Some(r) = remote() {
        r.lan_found(c_text(service), c_text(host), port, txt);
    }
}

/// A door Bonjour saw go.
///
/// # Safety
/// `service` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_lan_lost(service: *const c_char) {
    if let Some(r) = remote() {
        r.lan_lost(c_text(service));
    }
}

/// The devices list is in sight (`on` 1): the other devices are followed meanwhile.
#[no_mangle]
pub extern "C" fn nori_ios_remote_watch(on: i32) {
    if let Some(r) = remote() {
        r.watch(on != 0);
    }
}

/// A refusal as the app words it: 0 none, 1 the queue changed there, 2 not allowed, 3 gone, 4 too many,
/// 5 another source.
fn refusal_code(r: Option<Refusal>) -> u8 {
    match r {
        None => 0,
        Some(Refusal::Stale) => 1,
        Some(Refusal::NotAllowed) => 2,
        Some(Refusal::Unknown) => 3,
        Some(Refusal::TooMany) => 4,
        Some(Refusal::OtherSource) => 5,
    }
}

fn device_json(d: &RemoteDevice, active: bool) -> Value {
    let state = d.state.as_ref();
    let song = d.now.as_ref();
    json!({
        "id": d.id, "name": d.name, "active": active, "playing": state.is_some_and(|s| s.playing),
        "title": song.map(|e| e.title.as_str()), "artist": song.map(|e| e.artist.as_str()),
        "refused": refusal_code(d.refused), "kind": d.kind as u8,
    })
}

/// Where the music can play, as JSON to free; NULL while remote control is off: `{"here": this iPod plays,
/// "devices": [{"id", "name", "kind", "active", "playing", "title", "artist", "refused"}]}`, `kind` a `DeviceKind`.
#[no_mangle]
pub extern "C" fn nori_ios_remote_devices() -> *mut c_char {
    with_session(|s| {
        let r = s.remote()?;
        let active = s.elsewhere().map(|e| e.mirror.id);
        let devices: Vec<Value> = r.devices().iter().map(|d| device_json(d, active.as_ref() == Some(&d.id))).collect();
        let here = matches!(s.destination(), nori_core::destination::Destination::Here { .. });
        Some(owned(&json!({ "here": here, "devices": devices })))
    })
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// Moves the music to device `id`, or to this iPod (empty or NULL).
///
/// # Safety
/// `id` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_remote_pick(id: *const c_char) {
    let id = c_text(id);
    if let Some(r) = remote() {
        r.pick((!id.is_empty()).then_some(id));
    }
}

/// The current profile's device pairing code; NULL where pairing is unavailable.
#[no_mangle]
pub extern "C" fn nori_ios_pairing() -> *mut c_char {
    remote().and_then(|r| r.pairing_link()).map(|link| c(&link).into_raw()).unwrap_or(std::ptr::null_mut())
}

/// The devices paired with this iPod, as JSON to free; NULL where pairing is unavailable:
/// `[{"id", "name", "kind", "nearby", "days"}]`, `kind` a `DeviceKind`, `days` whole days since it was last
/// nearby.
#[no_mangle]
pub extern "C" fn nori_ios_paired() -> *mut c_char {
    let Some(r) = remote().filter(|r| r.pairing_link().is_some()) else { return std::ptr::null_mut() };
    let paired: Vec<Value> = r.paired().iter().map(|d| json!({ "id": d.id, "name": d.name, "kind": d.kind as u8, "nearby": d.nearby, "days": d.seen_days })).collect();
    owned(&Value::Array(paired))
}

/// [`nori_ios_pair`]: pairing started; its answer is a `REPORT_PAIRED`.
pub const PAIR_STARTED: i32 = 0;
/// The link is no pairing code.
pub const PAIR_NOT_A_CODE: i32 = 1;
/// No session is open, or remote control is off.
pub const PAIR_CLOSED: i32 = 2;

/// Why pairing failed, as a `REPORT_PAIRED` says it.
fn pair_error_code(e: PairError) -> i32 {
    match e {
        PairError::NotACode => 0,
        PairError::Own => 1,
        PairError::Unavailable => 2,
        PairError::Unreachable => 3,
        PairError::TimedOut => 4,
        PairError::Refused => 5,
    }
}

/// Pairs with the device whose code `link` is, on a thread of its own: a [`PAIR_STARTED`] answer comes as
/// a `REPORT_PAIRED`: flag 1 paired (text: its name), 0 not (index: why, a `NORI_PAIR_*` code; text: its
/// name, when its code says it).
///
/// # Safety
/// `link` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_pair(link: *const c_char) -> i32 {
    let link = c_text(link).trim().to_string();
    if !nori_core::remote::pairing_valid(link.clone()) {
        return PAIR_NOT_A_CODE;
    }
    let Some(r) = remote() else { return PAIR_CLOSED };
    nori_host::spawn("nori-ios-pair", move || {
        let name = nori_core::remote::pairing_name(link.clone()).unwrap_or_default();
        match nori_core::transport::block_on(r.pair(link)) {
            Ok(device) => report(REPORT_PAIRED, 1, 0, &device.name),
            Err(e) => report(REPORT_PAIRED, 0, pair_error_code(e), &name),
        }
    });
    PAIR_STARTED
}

/// Forgets paired device `id`: neither controls the other any more.
///
/// # Safety
/// `id` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_forget(id: *const c_char) {
    let id = c_text(id);
    if let Some(r) = remote() {
        r.forget(id);
    }
}
