//! A backup: nori's settings (the server profiles and their passwords among them), the saved sound
//! profiles, the favorites kept on this device and the installed plugins (their order, being on and
//! held, their stores with their rows, overrides and secrets), in one file sealed with a password the
//! user sets as it is made (seal.rs). Restoring puts the settings back, the profiles and favorites here
//! kept beside the backup's, and reinstalls the plugins from their repos.

mod seal;

use std::sync::Arc;

use nori_db::favorites::Favorite;
use nori_db::ItemKind;
use nori_model::{alog, SoundProfile, Starrable};
use nori_plugins::{KeptPlugins, PluginRestored, Plugins};
use nori_settings::settings::{same_profile, SavedServer, SettingChange};
use nori_settings::settings_store::{backed_prefs, Settings};
use serde::{Deserialize, Serialize};

/// Why a backup couldn't be made or restored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum BackupError {
    #[error("no password")]
    NoPassword,
    #[error("not the password the backup was made with")]
    WrongPassword,
    #[error("not a nori backup")]
    NotABackup,
    #[error("made by a newer nori")]
    Newer,
    #[error("changed or cut short since it was made")]
    Damaged,
    #[error("the settings' database couldn't be read or written")]
    Database,
}

/// What a backup holds, sealed.
#[derive(Debug, Serialize, Deserialize)]
struct Contents {
    /// Every stored setting, as the settings table keeps it.
    settings: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    sound_profiles: Vec<SoundProfile>,
    #[serde(default)]
    plugins: KeptPlugins,
    /// Every profile's favorites kept on this device (a provider's account couldn't keep them).
    #[serde(default)]
    favorites: Vec<KeptFavorite>,
}

/// A favorite kept on this device, as a backup holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct KeptFavorite {
    /// The profile it is kept for.
    profile: String,
    kind: Starrable,
    id: String,
    /// The item's record.
    record: serde_json::Value,
    sealed_ms: i64,
}

impl KeptFavorite {
    fn of(profile: String, f: Favorite) -> Option<KeptFavorite> {
        let kind = match f.kind {
            ItemKind::Song => Starrable::Song,
            ItemKind::Album => Starrable::Album,
            ItemKind::Artist => Starrable::Artist,
        };
        Some(KeptFavorite { profile, kind, id: f.id, record: serde_json::from_str(&f.record).ok()?, sealed_ms: f.sealed_ms })
    }

    fn favorite(&self) -> Favorite {
        let kind = match self.kind {
            Starrable::Song => ItemKind::Song,
            Starrable::Album => ItemKind::Album,
            Starrable::Artist => ItemKind::Artist,
        };
        Favorite { kind, id: self.id.clone(), record: self.record.to_string(), sealed_ms: self.sealed_ms }
    }
}

/// Every profile's favorites kept on this device; none before the settings are open.
fn favorites(settings: &Settings) -> Result<Vec<KeptFavorite>, BackupError> {
    let Some(db) = settings.app_db() else { return Ok(Vec::new()) };
    let every = nori_db::favorites::every(&db.lock());
    let every = every.map_err(|e| database(e.into()))?;
    Ok(every.into_iter().filter_map(|(profile, f)| KeptFavorite::of(profile, f)).collect())
}

/// Keeps a backup's favorites beside those here, each under the profile it is restored as: a backup's
/// profile that is one here (`same_profile`) is that one.
fn restore_favorites(settings: &Settings, kept: &[KeptFavorite], backed: &[SavedServer], here: &[SavedServer]) -> Result<(), BackupError> {
    let Some(db) = settings.app_db() else { return Ok(()) };
    let profile_here = |id: &str| backed.iter().find(|b| b.id == id).and_then(|b| here.iter().find(|h| same_profile(h, b))).map_or(id.to_string(), |h| h.id.clone());
    let rows: Vec<(String, Favorite)> = kept.iter().map(|k| (profile_here(&k.profile), k.favorite())).collect();
    let restored = nori_db::favorites::restore(&mut db.lock(), &rows);
    restored.map_err(|e| database(e.into()))
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Restored {
    /// The settings now, their effect bits, and whether the profile in use changed.
    pub settings: SettingChange,
    /// Each plugin backed up, in its order, and how it came back.
    pub plugins: Vec<PluginRestored>,
}

fn database(e: nori_model::CoreError) -> BackupError {
    alog::info(&format!("backup: {e}"));
    BackupError::Database
}

/// A backup of `settings` and `plugins`, sealed with `password`.
pub fn make(settings: &Settings, plugins: &Plugins, password: &str) -> Result<Vec<u8>, BackupError> {
    make_at(settings, plugins, password, seal::COST)
}

fn make_at(settings: &Settings, plugins: &Plugins, password: &str, cost: seal::Cost) -> Result<Vec<u8>, BackupError> {
    if password.is_empty() {
        return Err(BackupError::NoPassword);
    }
    let contents = Contents { settings: settings.backed_up(), sound_profiles: settings.sound_profiles().map_err(database)?, plugins: plugins.kept(), favorites: favorites(settings)? };
    seal::seal(&serde_json::to_vec(&contents).expect("JSON values serialize"), password, cost)
}

/// Restores the backup `file` with `password`: the sound profiles (each replacing one of its name), the
/// settings, the favorites kept on this device, then the plugins, fetched through the open profile's
/// connection. Nothing changes when the file can't be opened.
pub async fn restore(settings: &Settings, plugins: &Plugins, file: &[u8], password: &str) -> Result<Restored, BackupError> {
    let contents: Contents = serde_json::from_slice(&seal::open(file, password)?).map_err(|_| BackupError::Damaged)?;
    settings.keep_sound_profiles(&contents.sound_profiles).map_err(database)?;
    let backed = backed_prefs(&contents.settings).servers;
    let change = settings.restore(&contents.settings);
    restore_favorites(settings, &contents.favorites, &backed, &change.prefs.servers)?;
    let plugins = plugins.restore(contents.plugins).await;
    Ok(Restored { settings: change, plugins })
}

/// [`make`] for the platform.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn backup_make(settings: Arc<Settings>, plugins: Arc<Plugins>, password: String) -> Result<Vec<u8>, BackupError> {
    make(&settings, &plugins, &password)
}

/// [`restore`] for the platform.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub async fn backup_restore(settings: Arc<Settings>, plugins: Arc<Plugins>, file: Vec<u8>, password: String) -> Result<Restored, BackupError> {
    restore(&settings, &plugins, &file, &password).await
}

/// The name a new backup file is offered under.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn backup_file_name() -> String {
    "nori.noribackup".into()
}

#[cfg(test)]
mod tests;
