//! Devices that pair and sync through the in-memory hub, for this crate's tests.

use std::sync::Arc;
use std::time::Duration;

use nori_settings::settings::SavedServer;
use nori_settings::settings_store::Settings;

use crate::hub::{Hub, HubLink};
use crate::pairing::{AdmitStep, Admitting, Came, Confirmation, JoinStep, Joining};
use crate::profile::Here;
use crate::session::{Session, SyncEvent};

pub(crate) const SERVER: &str = "https://sync.example";

/// One device: its connection, its profiles and, once paired, its session.
pub(crate) struct Device {
    pub name: &'static str,
    pub link: HubLink,
    pub here: Here,
    pub servers: Vec<SavedServer>,
    pub session: Option<Session>,
    pub events: Vec<SyncEvent>,
    pub store: Arc<Settings>,
}

impl Device {
    pub(crate) fn new(hub: &Hub, name: &'static str, here: Here, servers: Vec<SavedServer>) -> Device {
        Device { name, link: hub.link(), here, servers, session: None, events: Vec::new(), store: Settings::new() }
    }

    pub(crate) fn session(&self) -> &Session {
        self.session.as_ref().expect("paired")
    }

    /// `f` over the session and this device's connection.
    pub(crate) fn act<R>(&mut self, f: impl FnOnce(&mut Session, &HubLink) -> R) -> R {
        f(self.session.as_mut().expect("paired"), &self.link)
    }

    /// Hands the session what this device heard; true when it heard anything.
    pub(crate) fn run(&mut self, now_ms: i64) -> bool {
        let heard = self.link.heard();
        let any = !heard.is_empty();
        for h in heard {
            let Some(session) = self.session.as_mut() else { continue };
            let out = session.heard(&self.link, h, &self.servers, &self.here, now_ms);
            if let Some(servers) = out.servers {
                self.servers = servers;
            }
            self.events.extend(out.events);
        }
        any
    }

    /// The user edited the profiles here.
    pub(crate) fn edit(&mut self, now_ms: i64, f: impl FnOnce(&mut Vec<SavedServer>)) {
        f(&mut self.servers);
        let (link, servers, here) = (&self.link, &self.servers, &self.here);
        self.session.as_mut().expect("paired").changed(link, servers, here, now_ms);
    }

    pub(crate) fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.servers.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    pub(crate) fn take_events(&mut self) -> Vec<SyncEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Runs every device until none hears anything more.
pub(crate) fn settle(devices: &mut [&mut Device], now_ms: i64) {
    for _ in 0..100 {
        let mut any = false;
        for d in devices.iter_mut() {
            any |= d.run(now_ms);
        }
        if !any {
            return;
        }
    }
    panic!("the devices never went quiet");
}

/// Pairs `new` into `old`'s group (making it, keeping a copy, if `old` has none) by the camera, and
/// connects both; what `old` was asked to confirm.
pub(crate) fn pair(old: &mut Device, new: &mut Device, now_ms: i64) -> Confirmation {
    let mut joining = Joining::start(&new.link, SERVER.into(), new.name.into(), new.here.plays, new.session.as_ref(), Duration::ZERO);
    let link = new.link.heard().into_iter().find_map(|h| match joining.heard(&new.link, h, Duration::ZERO) {
        Some(JoinStep::Code { link, .. }) => Some(link),
        _ => None,
    });
    let mut admitting = Admitting::read(&link.expect("a code"), Came::Camera, old.session.as_ref()).expect("a code to take");
    admitting.check(&old.link);
    let confirm = old.link.heard().into_iter().find_map(|h| match admitting.heard(&old.link, h, old.session.as_ref(), &old.servers) {
        Some(AdmitStep::Confirm(c)) => Some(c),
        _ => None,
    });
    let confirm = confirm.expect("something to confirm");
    let first = old.session.is_none();
    match old.session.as_mut() {
        Some(own) => assert_eq!(admitting.admit(&old.link, own, now_ms), Some(AdmitStep::Sent)),
        None => old.session = Some(admitting.admit_first(&old.link, old.name.into(), true, old.store.clone(), now_ms).expect("the new device waits")),
    }
    if first {
        old.session.as_ref().unwrap().connect(&old.link);
    }
    let welcomed = new.link.heard().into_iter().find_map(|h| match joining.heard(&new.link, h, Duration::ZERO) {
        Some(JoinStep::Paired(w)) => Some(w),
        _ => None,
    });
    let previous = new.session.take().map(|s| (s, &new.link as &dyn crate::room::Relay));
    new.session = Some(welcomed.expect("the key").join(previous, new.store.clone()));
    new.session.as_ref().unwrap().connect(&new.link);
    confirm
}
