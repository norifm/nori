//! A backup: nori's settings (the server profiles and their passwords among them), the saved sound
//! profiles and the installed plugins (their order, being on and held, their stores with their rows,
//! overrides and secrets), in one file sealed with a password the user sets as it is made
//! (seal.rs). Restoring puts the settings back, the profiles here kept beside the backup's, and
//! reinstalls the plugins from their repos.

mod seal;

use std::sync::Arc;

use nori_model::{alog, SoundProfile};
use nori_plugins::{KeptPlugins, PluginRestored, Plugins};
use nori_settings::settings::SettingChange;
use nori_settings::settings_store::Settings;
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
    let contents = Contents { settings: settings.backed_up(), sound_profiles: settings.sound_profiles().map_err(database)?, plugins: plugins.kept() };
    seal::seal(&serde_json::to_vec(&contents).expect("JSON values serialize"), password, cost)
}

/// Restores the backup `file` with `password`: the sound profiles (each replacing one of its name), the
/// settings, then the plugins, fetched through the open profile's connection. Nothing changes when the
/// file can't be opened.
pub async fn restore(settings: &Settings, plugins: &Plugins, file: &[u8], password: &str) -> Result<Restored, BackupError> {
    let contents: Contents = serde_json::from_slice(&seal::open(file, password)?).map_err(|_| BackupError::Damaged)?;
    settings.keep_sound_profiles(&contents.sound_profiles).map_err(database)?;
    let settings = settings.restore(&contents.settings);
    let plugins = plugins.restore(contents.plugins).await;
    Ok(Restored { settings, plugins })
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
