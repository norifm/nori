//! What every host client (the desktop, the terminal, the iPod) does with what sync, nori-sync's
//! `Syncing`, brings. The desktop and the terminal make their sync with [`computer`]; the iPod makes
//! its own over the app's Bonjour.

use std::path::Path;

use nori_core::sync::ProfilesSynced;

/// After sync changed the saved profiles: the rows of those it removed go, off this thread, as on
/// removing one by hand. Opening the profile in use again is the client's.
pub fn profiles_synced(data: &Path, synced: &ProfilesSynced) {
    for id in &synced.removed {
        crate::forget_profile_rows(data, id.clone());
    }
}

/// A computer's sync over `settings`, telling `client`: nori-http's WebSockets, and the nearby way of
/// pairing over mdns-sd.
#[cfg(feature = "desktop")]
pub fn computer(settings: std::sync::Arc<nori_core::settings_store::Settings>, client: std::sync::Arc<dyn nori_core::sync::SyncClient>, reach: nori_core::sync::Reach) -> std::sync::Arc<nori_core::sync::Syncing> {
    let sockets = nori_http::WebSockets::new();
    let sync = nori_core::sync::Syncing::native(settings, sockets.clone(), client, reach);
    sockets.tell(&sync);
    NearbyMdns::serve(&sync);
    sync
}

/// The nearby way of pairing over mdns-sd (`nori_sync::Nearby`), for the desktop and the terminal. Its
/// daemon starts when a pairing screen announces or looks, and stops when neither does, so a client
/// that never opens pairing runs none.
#[cfg(feature = "desktop")]
struct NearbyMdns {
    sync: std::sync::Weak<nori_core::sync::Syncing>,
    state: parking_lot::Mutex<MdnsState>,
}

#[cfg(feature = "desktop")]
#[derive(Default)]
struct MdnsState {
    daemon: Option<mdns_sd::ServiceDaemon>,
    /// The full name of what is announced.
    announced: Option<String>,
    browsing: bool,
}

#[cfg(feature = "desktop")]
const PAIRING_TYPE: &str = "_nori-pair._tcp.local.";

#[cfg(feature = "desktop")]
impl NearbyMdns {
    /// Hands `sync` its nearby way of pairing.
    fn serve(sync: &std::sync::Arc<nori_core::sync::Syncing>) {
        sync.nearby_with(std::sync::Arc::new(NearbyMdns { sync: std::sync::Arc::downgrade(sync), state: Default::default() }));
    }
}

#[cfg(feature = "desktop")]
impl MdnsState {
    fn daemon(&mut self) -> Option<&mdns_sd::ServiceDaemon> {
        if self.daemon.is_none() {
            self.daemon = crate::remote::mdns_daemon();
        }
        self.daemon.as_ref()
    }

    /// Stops the daemon when nothing is announced or looked for.
    fn settle(&mut self) {
        if self.announced.is_none() && !self.browsing {
            if let Some(d) = self.daemon.take() {
                let _ = d.shutdown();
            }
        }
    }
}

#[cfg(feature = "desktop")]
impl nori_core::sync::Nearby for NearbyMdns {
    fn announce(&self, waiting: Option<nori_core::sync::NearbyAnnouncement>) {
        let mut s = self.state.lock();
        if let (Some(old), Some(d)) = (s.announced.take(), s.daemon.as_ref()) {
            let _ = d.unregister(&old);
        }
        if let Some(w) = waiting {
            let txt: std::collections::HashMap<String, String> = w.txt.into_iter().map(|t| (t.key, t.value)).collect();
            // Nothing listens: the announcement carries the code, and a port is only what DNS-SD asks.
            let info = mdns_sd::ServiceInfo::new(PAIRING_TYPE, &w.name, &format!("{}.local.", w.name), "", 9, txt).map(mdns_sd::ServiceInfo::enable_addr_auto);
            if let (Ok(info), Some(d)) = (info, s.daemon()) {
                let name = info.get_fullname().to_string();
                if d.register(info).is_ok() {
                    s.announced = Some(name);
                }
            }
        }
        s.settle();
    }

    fn browse(&self, on: bool) {
        let mut s = self.state.lock();
        s.browsing = on;
        if !on {
            if let Some(d) = s.daemon.as_ref() {
                let _ = d.stop_browse(PAIRING_TYPE);
            }
            return s.settle();
        }
        let Some(events) = s.daemon().and_then(|d| d.browse(PAIRING_TYPE).ok()) else { return };
        let sync = self.sync.clone();
        crate::spawn("nori-pair-mdns", move || {
            while let Ok(e) = events.recv() {
                let Some(sync) = sync.upgrade() else { return };
                match e {
                    mdns_sd::ServiceEvent::ServiceResolved(found) => {
                        let txt = found.txt_properties.iter().map(|p| nori_core::sync::NearbyTxt { key: p.key().to_string(), value: p.val_str().to_string() }).collect();
                        sync.nearby_found(found.fullname.clone(), txt);
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, name) => sync.nearby_lost(name),
                    mdns_sd::ServiceEvent::SearchStopped(_) => return,
                    _ => {}
                }
            }
        });
    }
}
