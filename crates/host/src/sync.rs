//! A host client's sync, nori-sync's [`Syncing`]: the desktop, the terminal and the iPod make it once at
//! start with their sockets. It runs on a thread of its own, made only while the device is paired with
//! sync on, or pairing.

use std::path::Path;
use std::sync::Arc;

use nori_core::settings_store::Settings;
use nori_core::sync::{ProfilesSynced, Reach, Sockets, SyncClient, Syncing};
use nori_tasks::Tasks;

/// This device's sync over `settings`.
pub fn syncing(settings: Arc<Settings>, sockets: Arc<dyn Sockets>, client: Arc<dyn SyncClient>, reach: Reach) -> Arc<Syncing> {
    Syncing::new(settings, sockets, client, reach, Box::new(|| Arc::new(nori_tasks::Native::new("nori-sync").expect("a thread for sync")) as Arc<dyn Tasks>))
}

/// What every host client does after sync changed the saved profiles: the rows of those it removed go,
/// off this thread, as on removing one by hand. Opening the profile in use again is the client's.
pub fn profiles_synced(data: &Path, synced: &ProfilesSynced) {
    for id in &synced.removed {
        crate::forget_profile_rows(data, id.clone());
    }
}
