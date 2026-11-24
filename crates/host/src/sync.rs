//! What every host client (the desktop, the terminal, the iPod) does with what sync, nori-sync's
//! `Syncing`, brings. Each makes its sync with `Syncing::native` and its own sockets.

use std::path::Path;

use nori_core::sync::ProfilesSynced;

/// After sync changed the saved profiles: the rows of those it removed go, off this thread, as on
/// removing one by hand. Opening the profile in use again is the client's.
pub fn profiles_synced(data: &Path, synced: &ProfilesSynced) {
    for id in &synced.removed {
        crate::forget_profile_rows(data, id.clone());
    }
}
