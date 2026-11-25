//! Pairing and profile sync for the iPod (nori-sync's `Syncing`), over nori-http's WebSockets, with its
//! own handle on the app database. The iPod is reached as a phone is: while open or playing, its
//! keep-alive riding on the engine's wakes in the background. What sync tells the app comes as
//! `REPORT_SYNC`.

use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use nori_core::settings_store::Settings;
use nori_core::sync::{ActiveProfile, Came, Here, NeededPlugin, PairingView, ProfilesSynced, Reach, SyncClient, SyncEvent, SyncState, Syncing};
use serde_json::json;

use crate::session::{c, c_text, report, REPORT_SYNC};

/// The process's sync: a C entry point has nothing to carry it.
static SYNC: OnceLock<Arc<Syncing>> = OnceLock::new();

/// [`REPORT_SYNC`]'s flag: the state or the pairing changed (`text` is `nori_ios_sync_state`'s JSON).
pub const SYNC_CHANGED: i32 = 0;
/// Sync changed the saved profiles: `index` 1 when the profile in use changed or another took its place
/// (`nori_ios_reopen` opens it), 0 otherwise.
pub const SYNC_PROFILES: i32 = 1;

struct Told {
    data: PathBuf,
}

impl SyncClient for Told {
    fn here(&self) -> Here {
        nori_core::providers::Kinds::builtin().sync_here()
    }

    fn profiles(&self, synced: ProfilesSynced) {
        nori_host::sync::profiles_synced(&self.data, &synced);
        report(REPORT_SYNC, SYNC_PROFILES, (synced.active != ActiveProfile::Same) as i32, "");
    }

    /// The iPod plays no plugin's profile: none comes here.
    fn install(&self, _: Vec<NeededPlugin>) {}

    fn event(&self, event: SyncEvent) {
        nori_core::alog::info(&format!("sync: {event:?}"));
    }

    fn changed(&self) {
        if let Some(s) = SYNC.get() {
            report(REPORT_SYNC, SYNC_CHANGED, 0, &state(s));
        }
    }
}

/// Sync over the database under `dir`, made once.
fn open(dir: &Path) -> Result<(), String> {
    let settings = Settings::new();
    settings.open(&nori_host::db_path(dir)).map_err(|e| format!("the settings: {e}"))?;
    let sockets = nori_http::WebSockets::new();
    let sync = Syncing::native(settings, sockets.clone(), Arc::new(Told { data: dir.into() }), Reach::Phone);
    sockets.tell(&sync);
    let _ = SYNC.set(sync);
    Ok(())
}

/// The process's sync, once made: a session's remote control reaches my devices through it.
pub(crate) fn syncing() -> Option<Arc<Syncing>> {
    SYNC.get().cloned()
}

/// The engine of the session opened now carries sync's keep-alive on its wakes.
pub(crate) fn ride(engine: &nori_engine::Engine) {
    if let Some(s) = SYNC.get().cloned() {
        engine.on_wake(Some(Box::new(move || s.woke())));
    }
}

/// The app came into sight or left it.
pub(crate) fn shown(on: bool) {
    if let Some(s) = SYNC.get() {
        s.foreground(on);
    }
}

/// Music started or stopped.
pub(crate) fn playing(on: bool) {
    if let Some(s) = SYNC.get() {
        s.playing(on);
    }
}

fn state(sync: &Syncing) -> String {
    let (state, refused, retry_ms) = match sync.state() {
        SyncState::Offline { retry_ms } => ("Offline".to_string(), None, Some(retry_ms)),
        SyncState::Refused { refused, retry_ms } => ("Refused".into(), Some(format!("{refused:?}")), Some(retry_ms)),
        SyncState::Stopped { refused } => ("Stopped".into(), Some(format!("{refused:?}")), None),
        other => (format!("{other:?}"), None, None),
    };
    let devices: Vec<_> = sync.devices().iter().map(|d| json!({ "name": d.name, "online": d.online, "lastMs": d.last_ms })).collect();
    let mut v = json!({ "state": state, "refused": refused, "retryMs": retry_ms, "devices": devices });
    match sync.pairing() {
        PairingView::Code { link, words } => v["pairing"] = json!({ "code": link, "words": words }),
        PairingView::Confirm { confirmation: c } => v["pairing"] = json!({ "confirm": { "name": c.name, "profiles": c.profiles, "leaves": c.leaves, "words": c.words, "asksKeepCopy": c.asks_keep_copy } }),
        PairingView::Refused { reason } => v["pairing"] = json!({ "refused": format!("{reason:?}") }),
        other => v["pairing"] = format!("{other:?}").into(),
    }
    v.to_string()
}

/// Starts sync over the database under `data_dir`: nothing runs unless the iPod is paired with sync on.
/// `NULL` on success; otherwise an English error to free with `nori_ios_free`.
///
/// # Safety
/// `data_dir` is NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_sync_open(data_dir: *const c_char) -> *mut c_char {
    match open(Path::new(&c_text(data_dir))) {
        Ok(()) => std::ptr::null_mut(),
        Err(e) => c(&e).into_raw(),
    }
}

/// Where sync stands, as JSON: `{ state, refused, retryMs, devices: [{ name, online, lastMs }], pairing }`.
/// Free it with `nori_ios_free`; `NULL` before `nori_ios_sync_open`.
#[no_mangle]
pub extern "C" fn nori_ios_sync_state() -> *mut c_char {
    SYNC.get().map_or(std::ptr::null_mut(), |s| c(&state(s)).into_raw())
}

/// Shows a code to pair this iPod into a group; `REPORT_SYNC` follows with it.
#[no_mangle]
pub extern "C" fn nori_ios_sync_code() {
    if let Some(s) = SYNC.get() {
        s.pair_show(crate::session::DEVICE_NAME.into());
    }
}

/// Takes another device's pasted code. `NULL` when it reads; otherwise why not, to free with
/// `nori_ios_free`.
///
/// # Safety
/// `link` is NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_sync_take(link: *const c_char) -> *mut c_char {
    let Some(s) = SYNC.get() else { return c("sync is not open").into_raw() };
    match s.pair_take(c_text(link), Came::Link) {
        None => std::ptr::null_mut(),
        Some(r) => c(&format!("{r:?}")).into_raw(),
    }
}

/// Confirms the pairing asked; `keep` 1 keeps a copy on the server at a group's first pairing.
#[no_mangle]
pub extern "C" fn nori_ios_sync_confirm(keep: i32) {
    if let Some(s) = SYNC.get() {
        s.pair_confirm(crate::session::DEVICE_NAME.into(), keep != 0);
    }
}

#[no_mangle]
pub extern "C" fn nori_ios_sync_cancel() {
    if let Some(s) = SYNC.get() {
        s.pair_cancel();
    }
}

/// Reset pairing: this iPod leaves its group for one of its own.
#[no_mangle]
pub extern "C" fn nori_ios_sync_reset() {
    if let Some(s) = SYNC.get() {
        s.reset();
    }
}

/// The sync or sync server setting changed.
#[no_mangle]
pub extern "C" fn nori_ios_sync_settings_changed() {
    if let Some(s) = SYNC.get() {
        s.settings_changed();
    }
}
