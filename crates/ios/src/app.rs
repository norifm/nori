//! The app's queue session over its settings, one per database: playback's session, sync and the saved
//! servers all read and write the settings through it, so none writes back a copy older than another's
//! change.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nori_core::queue::Session;

/// The databases opened so far, each with its queue session. A C entry point has nothing to carry them.
static APPS: Mutex<Vec<(PathBuf, Arc<Session>)>> = Mutex::new(Vec::new());

/// The queue session over the database under `dir`, opened at the first ask.
pub(crate) fn app(dir: &Path) -> Result<Arc<Session>, String> {
    let mut apps = APPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, app)) = apps.iter().find(|(d, _)| d == dir) {
        return Ok(app.clone());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let app = Arc::new(Session::new(nori_core::settings_store::Settings::new()));
    app.settings.open(&nori_host::db_path(dir)).map_err(|e| format!("the database: {e}"))?;
    apps.push((dir.into(), app.clone()));
    Ok(app)
}
