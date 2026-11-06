//! The queue another device or app left on the server, offered at the top of Home (`offer::Offered`):
//! asked when Home comes into sight, played where it was left or turned down, and then not offered again.

use std::ffi::c_char;
use std::sync::Mutex;

use nori_core::library::QueueBy;
use nori_core::offer::Offered;
use serde_json::json;

use crate::pages::owned;
use crate::session::with_session;

/// The offer Home shows. A global: the C entry points carry no handle.
static OFFER: Mutex<Offered> = Mutex::new(Offered::new());

fn offer() -> std::sync::MutexGuard<'static, Offered> {
    OFFER.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the player shows: its song and whether it plays.
fn shown() -> (Option<String>, bool) {
    with_session(|s| {
        let n = s.now();
        (n.song.map(|song| song.id), n.playing)
    })
    .unwrap_or((None, false))
}

/// Asks the server for a queue to offer and keeps it, as JSON to free: `{song, index, count, app}` (`app` the
/// name of the app that left it, null for nori), or null for none. Blocks.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer() -> *mut c_char {
    let Some(found) = with_session(|s| s.queue_offer()) else { return std::ptr::null_mut() };
    let (here, playing) = shown();
    let mut kept = offer();
    let Some(o) = kept.found(found, here, playing) else { return std::ptr::null_mut() };
    let Some(song) = o.songs.get(o.index as usize) else { return std::ptr::null_mut() };
    let app = match &o.by {
        QueueBy::Nori => None,
        QueueBy::App { name } => Some(name.clone()),
    };
    owned(&json!({
        "song": crate::pages::song_json(song, o.index as usize), "index": o.index, "count": o.songs.len(), "app": app,
    }))
}

/// Whether the offer kept still stands against what the player shows (1); one that does not is let go.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer_stands() -> i32 {
    let (here, playing) = shown();
    i32::from(offer().still(here, playing))
}

/// The offer was taken (`take` 1: its queue plays where it was left) or turned down; it is not offered
/// again.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer_done(take: i32) {
    let Some(o) = offer().close() else { return };
    with_session(|s| s.queue_offer_closed(o, take != 0));
}
