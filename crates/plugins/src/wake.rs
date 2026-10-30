//! What starts nori while it is closed (`wake`) and commands other apps run. Each plugin's manifest
//! declares both; a plugin turns on the wakes it wants of those (kept in its store, so they hold while
//! nori is closed), and the client keeps on only the receivers some enabled plugin wants: on Android
//! its receivers are declared disabled and enabled only then, and the commands' receiver and D-Bus
//! object exist only while an enabled plugin has commands.

use std::collections::BTreeSet;

use crate::manifest::WakeEvent;
use crate::runtime::nori::plugin::wake;
use crate::runtime::points::hooks::Event;
use crate::runtime::Host;
use crate::{store, Plugins};

/// Where the wakes a plugin wants are kept in its store: their names, one per line.
const WANTED: &str = "nori/wake";

/// A command another app may run: the plugin's id and the command's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Command {
    pub plugin: String,
    pub command: String,
}

impl Plugins {
    fn wanted(&self, id: &str) -> BTreeSet<WakeEvent> {
        let kept = store::Store::new(self.folder(id).join("store")).get(WANTED).unwrap_or_default();
        let names = String::from_utf8_lossy(&kept).into_owned();
        WakeEvent::ALL.into_iter().filter(|e| names.lines().any(|n| n == e.name())).collect()
    }

    /// The wakes and commands of the enabled plugins, told to the client when they changed.
    pub(crate) fn wake_again(&self) {
        let on = self.running();
        let wakes: BTreeSet<WakeEvent> = on.iter().flat_map(|i| self.wanted(&i.manifest.id).into_iter().filter(|e| i.manifest.wake.contains(e))).collect();
        let commands: Vec<Command> = on.iter().flat_map(|i| i.manifest.commands.iter().map(|c| Command { plugin: i.manifest.id.clone(), command: c.clone() })).collect();
        let mut told = self.told.lock().expect("not poisoned");
        let now = (wakes.into_iter().collect::<Vec<_>>(), commands);
        if *told != now {
            self.side.wake(&now.0);
            self.side.commands(&now.1);
            *told = now;
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// `event` started nori: each enabled plugin that wants it hears `woke`.
    pub fn woke(&self, event: WakeEvent) {
        for i in self.running().into_iter().filter(|i| i.manifest.wake.contains(&event)) {
            if self.wanted(&i.manifest.id).contains(&event) {
                self.tell_one(&i.manifest.id, Event::Woke(event.name().into()));
            }
        }
    }

    /// Another app ran `command`: its plugin's action of that id, when the plugin is on and declares it.
    pub fn command(&self, command: Command) {
        let declared = self.running().into_iter().any(|i| i.manifest.id == command.plugin && i.manifest.commands.contains(&command.command) && i.manifest.extends("lifecycle"));
        if declared {
            self.act(command.plugin, command.command);
        }
    }
}

impl wake::Host for Host {
    /// Of the wakes its manifest declares; others are dropped.
    fn want(&mut self, events: Vec<String>) {
        let names: Vec<&str> = self.manifest.wake.iter().map(|e| e.name()).filter(|n| events.iter().any(|e| e == n)).collect();
        self.store.set(WANTED, names.join("\n").as_bytes());
        if let Some(p) = self.plugins() {
            p.wake_again();
        }
    }
}
