//! The queue another device or app left on the server, offered at the top of Home (`Client::queue_offer`):
//! asked when Home comes into sight, played where it was left or turned down, and then not offered again
//! (`queue_offer_done`).

use std::ffi::c_char;
use std::sync::Mutex;

use nori_core::library::{offer_stands, QueueBy, QueueOffer};
use serde_json::json;

use crate::pages::owned;
use crate::session::{server, with_session};

/// The offer Home shows. A global: the C entry points carry no handle.
static OFFER: Mutex<Option<QueueOffer>> = Mutex::new(None);

fn offer() -> std::sync::MutexGuard<'static, Option<QueueOffer>> {
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

fn stands(o: &QueueOffer, here: Option<String>, playing: bool) -> bool {
    o.songs.get(o.index as usize).is_some_and(|song| offer_stands(&song.id, here, playing))
}

/// Asks the server for a queue to offer and keeps it, as JSON to free: `{song, index, count, app}` (`app` the
/// name of the app that left it, null for nori), or null for none. Blocks.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer() -> *mut c_char {
    let Some((client, _)) = server() else { return std::ptr::null_mut() };
    let (here, playing) = shown();
    let found = nori_core::transport::block_on(client.queue_offer(playing, here.clone())).ok().flatten();
    let found = found.filter(|o| stands(o, here, playing));
    let answer = found.as_ref().and_then(|o| {
        let song = o.songs.get(o.index as usize)?;
        let app = match &o.by {
            QueueBy::Nori => None,
            QueueBy::App { name } => Some(name.clone()),
        };
        Some(owned(&json!({
            "song": crate::pages::song_json(song, o.index as usize), "index": o.index, "count": o.songs.len(), "app": app,
        })))
    });
    *offer() = found;
    answer.unwrap_or(std::ptr::null_mut())
}

/// Whether the offer kept still stands against what the player shows (1); one that does not is let go.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer_stands() -> i32 {
    let (here, playing) = shown();
    let mut kept = offer();
    if kept.as_ref().is_some_and(|o| !stands(o, here, playing)) {
        *kept = None;
    }
    i32::from(kept.is_some())
}

/// The offer was taken (`take` 1: its queue plays where it was left) or turned down; it is not offered
/// again.
#[no_mangle]
pub extern "C" fn nori_ios_queue_offer_done(take: i32) {
    let Some(o) = offer().take() else { return };
    if take != 0 {
        with_session(|s| {
            s.play(o.songs.clone(), o.index as usize, false, None);
            s.seek(o.position_ms as i64);
        });
    }
    if let Some((client, _)) = server() {
        client.queue_offer_done(o);
    }
}
