//! The web client's sync: the browser's WebSocket in this worker, read as binary, with the text `ping` as
//! its keep-alive (a browser sends no WebSocket ping). A browser is reached while its tab runs, as a
//! computer is. What sync tells the page goes out as `{ said: { kind: "sync", … } }`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use nori_core::settings_store::Settings;
use nori_core::sync::{Ended, Here, NeededPlugin, PairingView, ProfilesSynced, Reach, SocketId, Sockets, SyncClient, SyncEvent, SyncState, Syncing};
use nori_tasks::{Local, OneThread, Tasks};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

/// An open socket and the handlers it calls, kept while it lives.
struct Open {
    socket: WebSocket,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}

#[derive(Default)]
struct Inner {
    open: RefCell<HashMap<SocketId, Open>>,
    sync: RefCell<Weak<Syncing>>,
}

/// The worker's WebSockets. The module runs on one thread, so `OneThread` makes them `Send`.
#[derive(Clone)]
struct BrowserSockets(OneThread<std::rc::Rc<Inner>>);

impl BrowserSockets {
    fn new() -> BrowserSockets {
        BrowserSockets(OneThread(std::rc::Rc::default()))
    }

    fn ended(inner: &Inner, id: SocketId, how: Ended) {
        if inner.open.borrow_mut().remove(&id).is_some() {
            if let Some(s) = inner.sync.borrow().upgrade() {
                s.ended(id, how);
            }
        }
    }
}

impl Sockets for BrowserSockets {
    fn open(&self, id: SocketId, url: String) {
        let inner = self.0.0.clone();
        let Ok(socket) = WebSocket::new(&url) else {
            // An address the browser refuses: told once the call has returned, as sync asks.
            let later = OneThread(inner);
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(s) = later.0.sync.borrow().upgrade() {
                    s.ended(id, Ended::Dropped);
                }
            });
            return;
        };
        socket.set_binary_type(BinaryType::Arraybuffer);
        let heard = std::rc::Rc::downgrade(&inner);
        let message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            // A text frame is the keep-alive's `pong`.
            let (Some(inner), Ok(buf)) = (heard.upgrade(), e.data().dyn_into::<js_sys::ArrayBuffer>()) else { return };
            let sync = inner.sync.borrow().upgrade();
            if let Some(s) = sync.filter(|_| inner.open.borrow().contains_key(&id)) {
                s.frame(id, js_sys::Uint8Array::new(&buf).to_vec());
            }
        });
        let gone = std::rc::Rc::downgrade(&inner);
        let close = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            let Some(inner) = gone.upgrade() else { return };
            // 1006: no close frame came, the network dropped it or it never opened.
            let how = if e.code() == 1006 { Ended::Dropped } else { Ended::Closed { code: e.code() } };
            BrowserSockets::ended(&inner, id, how);
        });
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        inner.open.borrow_mut().insert(id, Open { socket, _message: message, _close: close });
    }

    fn send(&self, id: SocketId, frame: Vec<u8>) {
        if let Some(o) = self.0.0.open.borrow().get(&id) {
            let _ = o.socket.send_with_u8_array(&frame);
        }
    }

    fn ping(&self, id: SocketId) {
        if let Some(o) = self.0.0.open.borrow().get(&id) {
            let _ = o.socket.send_with_str("ping");
        }
    }

    fn close(&self, id: SocketId) {
        if let Some(o) = self.0.0.open.borrow_mut().remove(&id) {
            o.socket.set_onmessage(None);
            o.socket.set_onclose(None);
            let _ = o.socket.close_with_code(1000);
        }
    }
}

/// What sync tells the page.
struct Told {
    tell: OneThread<js_sys::Function>,
}

impl Told {
    fn say(&self, v: Value) {
        let _ = self.tell.call1(&JsValue::NULL, &JsValue::from_str(&v.to_string()));
    }
}

impl SyncClient for Told {
    fn here(&self) -> Here {
        nori_core::providers::Kinds::builtin().sync_here()
    }

    /// The page opens the profile in use again when it changed; the rows of removed ones go here.
    fn profiles(&self, synced: ProfilesSynced) {
        for id in &synced.removed {
            let _ = nori_core::db::db_forget_server(crate::db(), id.clone());
        }
        self.say(json!({ "kind": "sync", "profiles": format!("{:?}", synced.active) }));
    }

    fn install(&self, plugins: Vec<NeededPlugin>) {
        self.say(json!({ "kind": "sync", "install": plugins.iter().map(|p| &p.id).collect::<Vec<_>>() }));
    }

    fn event(&self, event: SyncEvent) {
        self.say(json!({ "kind": "sync", "event": format!("{event:?}") }));
    }

    fn changed(&self) {
        self.say(json!({ "kind": "sync", "changed": true }));
    }
}

/// The worker's sync over `settings`, telling the page through `tell`.
pub(crate) fn syncing(settings: Arc<Settings>, tell: js_sys::Function) -> Arc<Syncing> {
    let sockets = BrowserSockets::new();
    let syncing = Syncing::new(settings, Arc::new(sockets.clone()), Arc::new(Told { tell: OneThread(tell) }), Reach::Computer, Box::new(|| Arc::new(Local) as Arc<dyn Tasks>));
    *sockets.0.0.sync.borrow_mut() = Arc::downgrade(&syncing);
    syncing
}

/// Where sync stands, as the page reads it: `{ state, refused?, retryMs?, pairing, code?, words?,
/// confirm?, devices: [{ name, online, lastMs }] }`.
pub(crate) fn state(sync: &Syncing) -> String {
    let mut v = json!({ "devices": sync.devices().iter().map(|d| json!({ "name": d.name, "online": d.online, "lastMs": d.last_ms })).collect::<Vec<_>>() });
    match sync.state() {
        SyncState::Offline { retry_ms } => {
            v["state"] = "Offline".into();
            v["retryMs"] = retry_ms.into();
        }
        SyncState::Refused { refused, retry_ms } => {
            v["state"] = "Refused".into();
            v["refused"] = format!("{refused:?}").into();
            v["retryMs"] = retry_ms.into();
        }
        SyncState::Stopped { refused } => {
            v["state"] = "Stopped".into();
            v["refused"] = format!("{refused:?}").into();
        }
        other => v["state"] = format!("{other:?}").into(),
    }
    match sync.pairing() {
        PairingView::Code { link, words } => {
            v["pairing"] = "Code".into();
            v["code"] = link.into();
            v["words"] = words.into();
        }
        PairingView::Confirm { confirmation: c } => {
            v["pairing"] = "Confirm".into();
            v["confirm"] = json!({ "name": c.name, "profiles": c.profiles, "leaves": c.leaves, "words": c.words, "asksKeepCopy": c.asks_keep_copy });
        }
        PairingView::Refused { reason } => v["pairing"] = format!("Refused {reason:?}").into(),
        other => v["pairing"] = format!("{other:?}").into(),
    }
    v.to_string()
}
