//! The live settings: loaded from the app database's `settings` table (one row per key), kept in memory
//! and written back on the background thread after each change.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nori_db as db;
use nori_db::background;
use nori_model::alog;
use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::settings::{load, save, set_band, set_by_name, set_level, EqLevel, PrefValue, SettingChange, SoundBand, SoundError, SoundSettings, StoredPrefs};

// Effect bits: what a settings change asks the player (or client) to apply again.
/// Which parts of the output chain may run, speed and pitch.
pub const APPLY_AUDIO: u32 = 1;
/// The ReplayGain volume.
pub const APPLY_GAIN: u32 = 2;
/// The transition plan for the playing song.
pub const REPLAN: u32 = 4;
/// The sound chain's values (bands, pre-amp, balance, crossfeed, limiter, effects).
pub const SOUND: u32 = 8;
/// The player's fades and high quality output.
pub const PLAYER: u32 = 16;
/// The stream cache's size limit, applied at once.
pub const CACHE_LIMIT: u32 = 32;

/// The open settings and the database they are kept in.
struct SettingsStore {
    db: Arc<Mutex<Connection>>,
    prefs: StoredPrefs,
    /// Bumped per change; a queued write skips itself when a newer one is queued behind it.
    writes: Arc<AtomicU64>,
    /// The settings as this app last read or wrote them; a write changes only the keys that differ from
    /// them, so another app over the same database keeps its own changes.
    written: Arc<Mutex<StoredPrefs>>,
}

fn to_json(v: &PrefValue) -> String {
    match v {
        PrefValue::Flag { v } => json!({ "b": v }),
        PrefValue::Number { v } => json!({ "i": v }),
        PrefValue::Big { v } => json!({ "l": v }),
        PrefValue::Decimal { v } => json!({ "f": v }),
        PrefValue::Text { v } => json!({ "s": v }),
    }
    .to_string()
}

fn from_json(text: &str) -> Option<PrefValue> {
    let Value::Object(o) = serde_json::from_str(text).ok()? else { return None };
    let (t, v) = o.into_iter().next()?;
    Some(match (t.as_str(), v) {
        ("b", Value::Bool(v)) => PrefValue::Flag { v },
        ("i", Value::Number(n)) => PrefValue::Number { v: n.as_i64()? as i32 },
        ("l", Value::Number(n)) => PrefValue::Big { v: n.as_i64()? },
        ("f", Value::Number(n)) => PrefValue::Decimal { v: n.as_f64()? as f32 },
        ("s", Value::String(v)) => PrefValue::Text { v },
        _ => return None,
    })
}

fn read(c: &Connection) -> rusqlite::Result<HashMap<String, PrefValue>> {
    let mut st = c.prepare("SELECT key, value FROM settings")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.filter_map(|r| r.ok()).filter_map(|(k, v)| Some((k, from_json(&v)?))).collect())
}

/// Whether any saved sound profile has a parametric equalizer; false without a profiles table.
fn profiles_parametric(c: &Connection) -> bool {
    let Ok(mut st) = c.prepare("SELECT json FROM profiles") else { return false };
    let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) else { return false };
    let jsons: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    jsons.iter().any(|json| crate::settings::profile_parametric(json))
}

/// Rewrites the saved sound profiles stored before 0.6.3 in the new form (`settings::sound_from`).
fn migrate_profiles(c: &Connection) -> rusqlite::Result<()> {
    let Ok(mut st) = c.prepare("SELECT name, json FROM profiles") else { return Ok(()) };
    let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect();
    for (name, json) in rows {
        if let Some(sound) = crate::settings::sound_from(&json).filter(|_| crate::settings::legacy_profile(&json)) {
            c.execute("UPDATE profiles SET json=?1 WHERE name=?2", params![crate::settings::sound_json(&sound), name])?;
        }
    }
    Ok(())
}

/// Replaces every stored setting in one transaction.
fn write(c: &mut Connection, prefs: &StoredPrefs) -> rusqlite::Result<()> {
    let put = save(prefs);
    let tx = c.transaction()?;
    tx.execute("DELETE FROM settings", [])?;
    {
        let mut st = tx.prepare("INSERT INTO settings(key, value) VALUES(?1, ?2)")?;
        for (k, v) in &put {
            st.execute(params![k, to_json(v)])?;
        }
    }
    tx.commit()
}

/// Writes the stored settings that differ between `before` and `after`, in one transaction.
fn write_changes(c: &mut Connection, before: &StoredPrefs, after: &StoredPrefs) -> rusqlite::Result<()> {
    let (before, after) = (save(before), save(after));
    let tx = c.transaction()?;
    {
        let mut put = tx.prepare("INSERT OR REPLACE INTO settings(key, value) VALUES(?1, ?2)")?;
        for (k, v) in after.iter().filter(|(k, v)| before.get(*k) != Some(v)) {
            put.execute(params![k, to_json(v)])?;
        }
        let mut drop = tx.prepare("DELETE FROM settings WHERE key=?1")?;
        for k in before.keys().filter(|k| !after.contains_key(*k)) {
            drop.execute([k])?;
        }
    }
    tx.commit()
}

/// What a change from `a` to `b` asks the player to apply again: each changed setting's effect bits, plus
/// [`APPLY_AUDIO`] when the sound chain starts or stops being needed.
fn effects(a: &StoredPrefs, b: &StoredPrefs) -> u32 {
    let chain = if a.sound_chain_on() != b.sound_chain_on() { APPLY_AUDIO } else { 0 };
    crate::settings::ROWS.iter().filter(|r| r.effect != 0 && (r.changed)(a, b)).fold(chain, |e, r| e | r.effect)
}

impl SettingsStore {
    /// The settings in the app database at `db_path`; the defaults are written the first time.
    fn open(db_path: &str) -> nori_model::Result<Self> {
        let mut c = db::open_app(db_path)?;
        let raw = read(&c)?;
        let mut prefs = load(&raw);
        // Stored before the equalizer had a graphic mode: a saved profile with a parametric curve keeps
        // the parametric equalizer too. The choice is written at once.
        let chosen = raw.contains_key(crate::settings::EQ_MODE_KEY);
        if !raw.is_empty() && !chosen && profiles_parametric(&c) {
            prefs.eq_mode = crate::settings::EqMode::Parametric;
        }
        let legacy = crate::settings::legacy(&raw);
        if raw.is_empty() || !chosen || legacy || crate::settings::picked(&raw) || crate::settings::pure_black(&raw) || crate::settings::old_screen_defaults(&raw) || raw.contains_key("jam") || raw.contains_key("jamAlong") {
            write(&mut c, &prefs)?;
        }
        if legacy {
            migrate_profiles(&c)?;
        }
        Ok(SettingsStore { db: Arc::new(Mutex::new(c)), written: Arc::new(Mutex::new(prefs.clone())), prefs, writes: Arc::new(AtomicU64::new(0)) })
    }

    /// Replaces the settings with what `make` makes of them and queues the write. Returns the effect
    /// bits, or None when nothing changed.
    fn edit(&mut self, make: impl FnOnce(&StoredPrefs) -> StoredPrefs) -> Option<u32> {
        let prefs = make(&self.prefs);
        if prefs == self.prefs {
            return None;
        }
        let effect = effects(&self.prefs, &prefs);
        self.prefs = prefs;
        let (db, writes, written, prefs) = (self.db.clone(), self.writes.clone(), self.written.clone(), self.prefs.clone());
        let n = writes.fetch_add(1, Ordering::SeqCst) + 1;
        background::run(move || {
            if writes.load(Ordering::SeqCst) != n {
                return;
            }
            let mut written = written.lock();
            match write_changes(&mut db.lock(), &written, &prefs) {
                Ok(()) => *written = prefs,
                Err(e) => alog::info(&format!("settings: could not write: {e}")),
            }
        });
        Some(effect)
    }

    fn edit_sound(&mut self, sound: SoundSettings) -> Option<u32> {
        self.edit(|p| p.clone().with_sound(sound))
    }

    fn edit_band(&mut self, index: u32, asked: SoundBand) -> Option<(u32, SoundBand)> {
        let s = set_band(self.prefs.sound(), index, asked);
        let kept = *s.eq_bands.get(index as usize)?;
        Some((self.edit_sound(s)?, kept))
    }

    fn edit_graphic(&mut self, index: u32, gain_db: f32) -> Option<(u32, f32)> {
        let s = crate::settings::set_graphic(self.prefs.sound(), index, gain_db);
        let kept = *s.eq_graphic.get(index as usize)?;
        Some((self.edit_sound(s)?, kept))
    }

    fn edit_level(&mut self, level: EqLevel, value: f32) -> Option<(u32, f32)> {
        let s = set_level(self.prefs.sound(), level, value);
        let kept = match level {
            EqLevel::Preamp => s.eq_preamp_db.unwrap_or(value),
            other => other.of(&s),
        };
        Some((self.edit_sound(s)?, kept))
    }

    /// The active server's own settings (`SettingChange::server`) are returned but not kept: the
    /// platform applies them through its server update.
    fn edit_by_name(&mut self, name: &str, value: &str) -> Option<SettingChange> {
        let mut change = None;
        let effect = self.edit(|p| {
            let Some(c) = set_by_name(p, name, value) else { return p.clone() };
            let next = if c.server { p.clone() } else { c.prefs.clone() };
            change = Some(c);
            next
        });
        change.map(|c| SettingChange { effect: effect.unwrap_or(0), ..c })
    }

    fn sound_tool(&mut self, tool: SoundTool) -> Result<Option<SoundChange>, SoundError> {
        let sound = tool.apply(self.prefs.sound())?;
        Ok(self.edit_sound(sound.clone()).map(|effect| SoundChange { sound, effect }))
    }

    fn app_value(&self, key: &str) -> Option<String> {
        self.db.lock().query_row("SELECT value FROM app_kv WHERE key=?1", [key], |r| r.get(0)).optional().ok().flatten()
    }
}

/// The live settings of one app, empty (the defaults) until opened, and the beat model's file, which
/// follows its switch. The platform holds one for the app and hands it to the queue's session.
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Settings {
    kept: RwLock<Option<SettingsStore>>,
    pub model: nori_automix::beat_model::ModelFile,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Settings {
    /// Empty (the defaults) until [`Settings::open`].
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> Arc<Settings> {
        Arc::default()
    }

    /// Opens the settings kept in the app database at `db_path` and makes them the live ones.
    pub fn open(&self, db_path: &str) -> nori_model::Result<StoredPrefs> {
        let store = SettingsStore::open(db_path)?;
        let prefs = store.prefs.clone();
        self.write(|k| *k = Some(store));
        Ok(prefs)
    }

    /// Replaces the settings. Returns the effect bits; 0 when nothing changed or the settings are not open.
    pub fn put(&self, prefs: StoredPrefs) -> u32 {
        self.with_store(|s| s.edit(|_| prefs)).unwrap_or(0)
    }

    /// Applies an equalizer tool to the live settings. None when nothing changed or the settings are not
    /// open; an import without filters is an error.
    pub fn sound_tool(&self, tool: SoundTool) -> Result<Option<SoundChange>, SoundError> {
        self.write(|k| k.as_mut().map_or(Ok(None), |s| s.sound_tool(tool)))
    }
}

impl Settings {
    /// `f` over the open settings, the beat model's file following its switch under the same lock, so
    /// concurrent edits reach it in the order they were kept.
    fn write<R>(&self, f: impl FnOnce(&mut Option<SettingsStore>) -> R) -> R {
        let mut k = self.kept.write();
        let r = f(&mut k);
        if let Some(s) = k.as_ref() {
            self.model.switched(s.prefs.auto_mix_better_beats);
        }
        r
    }

    fn with_store<R>(&self, f: impl FnOnce(&mut SettingsStore) -> Option<R>) -> Option<R> {
        self.write(|k| k.as_mut().and_then(f))
    }

    /// A value from the app database's `app_kv` table; None before the settings are open.
    pub fn app_value(&self, key: &str) -> Option<String> {
        self.kept.read().as_ref()?.app_value(key)
    }

    /// The app database the settings were opened from, for other app-wide tables; None before it is open.
    pub fn app_db(&self) -> Option<Arc<Mutex<Connection>>> {
        self.kept.read().as_ref().map(|k| k.db.clone())
    }

    /// Stores an `app_kv` value on the background thread.
    pub fn keep_app_value(&self, key: &'static str, value: String) {
        let Some(db) = self.app_db() else { return };
        background::run(move || {
            if let Err(e) = db.lock().execute("INSERT OR REPLACE INTO app_kv(key, value) VALUES(?1, ?2)", params![key, value]) {
                alog::info(&format!("{key}: could not write: {e}"));
            }
        });
    }

    /// Removes an `app_kv` value on the background thread.
    pub fn forget_app_value(&self, key: &'static str) {
        let Some(db) = self.app_db() else { return };
        background::run(move || {
            if let Err(e) = db.lock().execute("DELETE FROM app_kv WHERE key=?1", [key]) {
                alog::info(&format!("{key}: could not remove: {e}"));
            }
        });
    }

    /// One parametric band changed (`settings::set_band`): the effect bits and the band as kept (held in
    /// range); None when nothing changed.
    pub fn edit_band(&self, index: u32, asked: SoundBand) -> Option<(u32, SoundBand)> {
        self.with_store(|s| s.edit_band(index, asked))
    }

    /// One graphic slider changed (`settings::set_graphic`): the effect bits and the value as kept.
    pub fn edit_graphic(&self, index: u32, gain_db: f32) -> Option<(u32, f32)> {
        self.with_store(|s| s.edit_graphic(index, gain_db))
    }

    /// A level slider changed (`settings::set_level`): the effect bits and the value as kept (held in
    /// range and snapped).
    pub fn edit_level(&self, level: EqLevel, value: f32) -> Option<(u32, f32)> {
        self.with_store(|s| s.edit_level(level, value))
    }

    /// A change by name (`settings::set_by_name`), kept, with the settings after it and the effect bits.
    /// None for an unknown name. Before the settings are open it is applied to the defaults and not kept.
    pub fn edit_by_name(&self, name: &str, value: &str) -> Option<SettingChange> {
        self.write(|k| match k.as_mut() {
            Some(s) => s.edit_by_name(name, value),
            None => set_by_name(&StoredPrefs::default(), name, value),
        })
    }

    /// A copy of the live settings; None before they are open.
    pub fn current(&self) -> Option<StoredPrefs> {
        self.with_prefs(StoredPrefs::clone)
    }

    /// `f` applied to the live settings without copying them; None before they are open.
    pub fn with_prefs<R>(&self, f: impl FnOnce(&StoredPrefs) -> R) -> Option<R> {
        self.kept.read().as_ref().map(|k| f(&k.prefs))
    }

    /// `f` applied to the live settings, or to the defaults before they are open.
    pub fn prefs<R>(&self, f: impl FnOnce(&StoredPrefs) -> R) -> R {
        match self.kept.read().as_ref() {
            Some(k) => f(&k.prefs),
            None => f(&StoredPrefs::default()),
        }
    }
}

/// An equalizer screen tool (sliders are [`Settings::edit_band`] and [`Settings::edit_level`]).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SoundTool {
    Preset { preset: nori_model::NamedPreset },
    AutoPreamp { automatic: bool },
    AddBand,
    RemoveBand { index: u32 },
    ResetBands,
    /// AutoEQ "ParametricEQ.txt" or Equalizer APO text.
    Import { text: String },
}

/// The sound after a [`SoundTool`], and the effect bits.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SoundChange {
    pub sound: SoundSettings,
    pub effect: u32,
}

impl SoundTool {
    fn apply(self, s: SoundSettings) -> Result<SoundSettings, SoundError> {
        use crate::settings as st;
        Ok(match self {
            SoundTool::Preset { preset } => st::apply_preset(s, &preset),
            SoundTool::AutoPreamp { automatic } => st::set_auto_preamp(s, automatic),
            SoundTool::AddBand => st::add_band(s),
            SoundTool::RemoveBand { index } => st::remove_band(s, index),
            SoundTool::ResetBands => st::eq_reset_bands(s),
            SoundTool::Import { text } => st::import(s, &text)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{band_from, sound_json, CompressorPreset, EqMode, GainMode, HeadphoneSound, Level, SavedServer, ThemeMode, EQ_MODE_KEY, EQ_RANGES};

    fn open(dir: &nori_testdir::TempDir) -> SettingsStore {
        SettingsStore::open(&dir.join("nori.db").display().to_string()).unwrap()
    }

    #[test]
    fn round_trips() {
        for v in [
            PrefValue::Flag { v: true },
            PrefValue::Number { v: -3 },
            PrefValue::Big { v: 1 << 40 },
            PrefValue::Decimal { v: 0.1 },
            PrefValue::Text { v: "x\"y".into() },
        ] {
            assert_eq!(from_json(&to_json(&v)), Some(v));
        }

        // Saved settings load back.
        let dir = nori_testdir::TempDir::new("settings");
        let s = open(&dir);
        assert_eq!(s.prefs, StoredPrefs::default());
        write(&mut s.db.lock(), &StoredPrefs { crossfade_sec: 4, ..s.prefs.clone() }).unwrap();
        drop(s);
        assert_eq!(open(&dir).prefs.crossfade_sec, 4);
    }

    #[test]
    fn effects_and_rebuilds() {
        let a = StoredPrefs::default();
        let cases = [
            (StoredPrefs { theme: ThemeMode::Midnight, ..a.clone() }, 0),
            (StoredPrefs { eq_bands: vec![band_from(0, 100.0, 3.0, 1.0, 0)], ..a.clone() }, SOUND),
            (StoredPrefs { eq_enabled: true, ..a.clone() }, APPLY_AUDIO | SOUND),
            (StoredPrefs { replay_gain: GainMode::Track, ..a.clone() }, APPLY_GAIN | REPLAN | SOUND),
            (StoredPrefs { headphone: HeadphoneSound::Natural, ..a.clone() }, APPLY_AUDIO | SOUND),
            (StoredPrefs { level: Level::Loud, ..a.clone() }, APPLY_GAIN),
            (StoredPrefs { crossfade_sec: 6, ..a.clone() }, APPLY_AUDIO | REPLAN),
            (StoredPrefs { auto_mix: true, ..a.clone() }, APPLY_AUDIO | REPLAN),
            (StoredPrefs { hi_res: !a.hi_res, ..a.clone() }, PLAYER),
            (StoredPrefs { cache_mb: 4096, ..a.clone() }, CACHE_LIMIT),
        ];
        for (b, want) in cases {
            assert_eq!(effects(&a, &b), want, "{b:?}");
        }

        // Balance and crossfeed rebuild only when chain toggles.
        let a = StoredPrefs::default();
        assert!(!a.sound_chain_on());
        let off_centre = StoredPrefs { balance: -0.4, ..a.clone() };
        assert_eq!(effects(&a, &off_centre), APPLY_AUDIO | SOUND, "chain starts");
        assert_eq!(effects(&off_centre, &StoredPrefs { balance: -0.5, ..a.clone() }), SOUND, "drag step");
        assert_eq!(effects(&off_centre, &a), APPLY_AUDIO | SOUND, "chain stops");
        let feed = StoredPrefs { headphone: HeadphoneSound::Natural, ..a.clone() };
        assert_eq!(effects(&feed, &StoredPrefs { headphone: HeadphoneSound::Wide, ..a.clone() }), SOUND, "the chain keeps running");
        let eq = StoredPrefs { eq_enabled: true, ..a.clone() };
        assert_eq!(effects(&eq, &StoredPrefs { balance: 0.3, ..eq.clone() }), SOUND, "chain already running");
        assert_eq!(effects(&eq, &StoredPrefs { limiter_threshold_db: -3.0, ..eq.clone() }), SOUND);
    }

    #[test]
    fn slider_edits() {
        let dir = nori_testdir::TempDir::new("settings-edit");
        let mut s = open(&dir);
        let band = s.prefs.eq_bands[2];
        let (effect, kept) = s.edit_band(2, SoundBand { gain_db: 99.0, ..band }).unwrap();
        assert_eq!(effect, SOUND);
        assert_eq!(kept.gain_db, EQ_RANGES.gain.max, "held in range");
        assert_eq!(s.prefs.eq_bands[2], kept);
        assert_eq!(s.edit_band(2, kept), None, "unchanged");
        assert_eq!(s.edit_band(99, band), None, "no such band");
        assert_eq!(s.edit_graphic(0, 20.0), Some((SOUND, 12.0)), "held in range");
        assert_eq!(s.prefs.eq_graphic[0], 12.0);
        assert_eq!(s.edit_level(EqLevel::Balance, 0.02), None, "snaps to the centre");
        assert_eq!(s.edit_level(EqLevel::Balance, -0.5), Some((APPLY_AUDIO | SOUND, -0.5)));
        assert_eq!(s.prefs.balance, -0.5);
        assert_eq!(s.edit_level(EqLevel::Balance, -0.6), Some((SOUND, -0.6)));
        assert_eq!(s.edit_level(EqLevel::VolumeBoost, 20.0), Some((SOUND, 12.0)), "held in range");
        assert_eq!(s.prefs.volume_boost_db, 12.0);
    }

    #[test]
    fn two_apps_over_one_database_keep_each_others_changes() {
        let dir = nori_testdir::TempDir::new("settings-shared");
        let (mut a, mut b) = (open(&dir), open(&dir));
        a.edit(|p| StoredPrefs { crossfade_sec: 4, ..p.clone() });
        background::flush();
        b.edit(|p| StoredPrefs { theme: ThemeMode::Light, ..p.clone() });
        b.edit(|p| StoredPrefs { cache_mb: 4096, ..p.clone() });
        background::flush();
        let kept = open(&dir).prefs;
        assert_eq!((kept.crossfade_sec, kept.theme, kept.cache_mb), (4, ThemeMode::Light, 4096));
    }

    #[test]
    fn remote_migration_is_saved_once() {
        let dir = nori_testdir::TempDir::new("remote-migration");
        let path = dir.join("nori.db").display().to_string();
        {
            let mut c = db::open_app(&path).unwrap();
            write(&mut c, &StoredPrefs { remote_control: false, ..Default::default() }).unwrap();
            c.execute("INSERT INTO settings VALUES ('jam', ?1)", [to_json(&PrefValue::Flag { v: true })]).unwrap();
        }
        let mut migrated = SettingsStore::open(&path).unwrap();
        assert!(migrated.prefs.remote_control);
        assert!(!read(&migrated.db.lock()).unwrap().contains_key("jam"));
        migrated.prefs.remote_control = false;
        write(&mut migrated.db.lock(), &migrated.prefs).unwrap();
        assert!(!SettingsStore::open(&path).unwrap().prefs.remote_control);
    }

    /// An install from before the graphic equalizer: the mode is chosen from its settings and sound
    /// profiles, and written at once.
    #[test]
    fn old_install_keeps_parametric() {
        let open_with = |name: &str, prefs: &StoredPrefs, profile: Option<String>| {
            let dir = nori_testdir::TempDir::new(name);
            let path = dir.join("nori.db").display().to_string();
            {
                let mut c = db::open_app(&path).unwrap();
                write(&mut c, &StoredPrefs { eq_mode: EqMode::Graphic, ..prefs.clone() }).unwrap();
                c.execute("DELETE FROM settings WHERE key = ?1", [EQ_MODE_KEY]).unwrap();
                if let Some(json) = profile {
                    c.execute_batch("CREATE TABLE profiles(name TEXT PRIMARY KEY, json TEXT NOT NULL, outputs TEXT NOT NULL DEFAULT '') WITHOUT ROWID").unwrap();
                    c.execute("INSERT INTO profiles(name, json) VALUES('Mine', ?1)", [json]).unwrap();
                }
            }
            let opened = SettingsStore::open(&path).unwrap().prefs.eq_mode;
            let raw = read(&db::open_app(&path).unwrap()).unwrap();
            (opened, raw.contains_key(EQ_MODE_KEY), load(&raw).eq_mode)
        };
        let d = StoredPrefs::default();
        assert_eq!(open_with("settings-old-flat", &d, None), (EqMode::Graphic, true, EqMode::Graphic));
        let with_bands = StoredPrefs { eq_enabled: true, eq_bands: vec![band_from(1, 100.0, 6.0, 0.7, 0)], ..d.clone() };
        assert_eq!(open_with("settings-old-bands", &with_bands, None), (EqMode::Parametric, true, EqMode::Parametric));
        let profile = sound_json(&with_bands.sound());
        assert_eq!(open_with("settings-old-profile", &d, Some(profile)), (EqMode::Parametric, true, EqMode::Parametric));
        let dir = nori_testdir::TempDir::new("settings-new-install");
        let s = open(&dir);
        assert_eq!(s.prefs.eq_mode, EqMode::Graphic);
        assert!(read(&s.db.lock()).unwrap().contains_key(EQ_MODE_KEY), "a new install writes the mode");
    }

    #[test]
    fn edit_by_name_and_sound_tools() {
        let dir = nori_testdir::TempDir::new("settings-by-name");
        let mut s = open(&dir);
        let c = s.edit_by_name("limiter", "true").unwrap();
        assert!(c.prefs.limiter && s.prefs.limiter);
        assert_eq!(c.effect, APPLY_AUDIO | SOUND);
        assert_eq!(c.prefs, s.prefs);
        assert_eq!(s.edit_by_name("limiter", "true").unwrap().effect, 0, "unchanged");
        assert_eq!(s.edit_by_name("cacheMb", "512").unwrap().effect, CACHE_LIMIT);
        assert!(s.edit_by_name("noSuchSetting", "1").is_none());
        // The active server's own settings are returned, not kept.
        let with_server = StoredPrefs { servers: vec![SavedServer { id: "s1".into(), ..Default::default() }], active_server_id: "s1".into(), ..s.prefs.clone() };
        s.edit(|_| with_server.clone());
        let c = s.edit_by_name("musicFolders", "7").unwrap();
        assert!(c.server);
        assert_eq!(c.prefs.servers[0].music_folder_ids, ["7"]);
        assert_eq!(s.prefs, with_server);
        assert_eq!(c.effect, 0);

        let tool = s.sound_tool(SoundTool::AddBand).unwrap().unwrap();
        assert_eq!(tool.sound, s.prefs.sound());
        assert_eq!(tool.effect, SOUND);
        let n = tool.sound.eq_bands.len();
        assert_eq!(s.sound_tool(SoundTool::RemoveBand { index: n as u32 - 1 }).unwrap().unwrap().sound.eq_bands.len(), n - 1);
        assert!(matches!(s.sound_tool(SoundTool::Import { text: "nothing here".into() }), Err(SoundError::NoFilters)));
        assert_eq!(s.prefs.eq_bands.len(), n - 1, "a failed import changes nothing");
        assert_eq!(s.sound_tool(SoundTool::RemoveBand { index: 999 }).unwrap(), None);
    }

    /// Settings as 0.6.2 stored them (testdata/stored_0.6.2.txt): its defaults, or every setting changed.
    fn stored_062(changed: bool) -> HashMap<String, PrefValue> {
        let file = include_str!("../testdata/stored_0.6.2.txt");
        let section = file.split("## ").filter(|s| !s.is_empty()).nth(if changed { 0 } else { 1 }).unwrap();
        section
            .lines()
            .skip(1)
            .filter(|l| !l.is_empty())
            .map(|l| {
                let (key, stored) = l.split_once(" = ").unwrap();
                let (kind, v) = stored.split_once(' ').unwrap();
                let value = match kind {
                    "flag" => PrefValue::Flag { v: v == "true" },
                    "int" => PrefValue::Number { v: v.parse().unwrap() },
                    "long" => PrefValue::Big { v: v.parse().unwrap() },
                    "float" => PrefValue::Decimal { v: v.parse().unwrap() },
                    "text" => PrefValue::Text { v: serde_json::from_str(v).unwrap() },
                    _ => PrefValue::Text { v: v.to_string() },
                };
                (key.to_string(), value)
            })
            .collect()
    }

    /// 0.6.2's defaults with `changes`.
    fn old(changes: &[(&str, PrefValue)]) -> HashMap<String, PrefValue> {
        let mut raw = stored_062(false);
        raw.extend(changes.iter().map(|(k, v)| (k.to_string(), v.clone())));
        raw
    }

    /// Settings stored as `raw` opened: what they are, and what is stored after.
    fn open_old(name: &str, raw: &HashMap<String, PrefValue>) -> (StoredPrefs, HashMap<String, PrefValue>) {
        let dir = nori_testdir::TempDir::new(name);
        let path = dir.join("nori.db").display().to_string();
        {
            let c = db::open_app(&path).unwrap();
            for (k, v) in raw {
                c.execute("INSERT INTO settings(key, value) VALUES(?1, ?2)", params![k, to_json(v)]).unwrap();
            }
        }
        let prefs = SettingsStore::open(&path).unwrap().prefs;
        (prefs, read(&db::open_app(&path).unwrap()).unwrap())
    }

    fn flag(v: bool) -> PrefValue {
        PrefValue::Flag { v }
    }
    fn number(v: i32) -> PrefValue {
        PrefValue::Number { v }
    }
    fn decimal(v: f32) -> PrefValue {
        PrefValue::Decimal { v }
    }

    #[test]
    fn screen_defaults_migrate_once() {
        use crate::settings::DimIdle;
        for (dim, want) in [(0, DimIdle::Never), (1, DimIdle::HalfMinute), (2, DimIdle::Never), (3, DimIdle::FiveMinutes)] {
            let raw = old(&[("burnInProtection", flag(true)), ("dimWhenIdle", number(dim))]);
            let (prefs, stored) = open_old("screen-defaults", &raw);
            assert_eq!((prefs.burn_in_protection, prefs.dim_when_idle), (false, want));
            assert!(!stored.contains_key("burnInProtection") && !stored.contains_key("dimWhenIdle"));
            let chosen = StoredPrefs { burn_in_protection: true, dim_when_idle: DimIdle::Minute, ..prefs };
            assert_eq!(open_old("screen-chosen", &save(&chosen)).0, chosen);
        }
    }

    #[test]
    fn old_settings_are_stored_without_removed_keys() {
        for changed in [false, true] {
            let (prefs, stored) = open_old("old-keys", &stored_062(changed));
            let left: Vec<&String> = stored.keys().filter(|k| crate::settings::LEGACY_KEYS.contains(&k.as_str())).collect();
            assert!(left.is_empty(), "{left:?}");
            assert_eq!(load(&stored), prefs, "read back as migrated, once");
            assert_eq!(stored, save(&prefs));
        }
        let (prefs, _) = open_old("old-changed", &stored_062(true));
        assert_eq!((prefs.servers.len(), prefs.theme, prefs.cache_mb, prefs.crossfade_keep_albums), (1, ThemeMode::Midnight, 4096, false), "the rest as it was, pure black with dark becoming Midnight");
    }

    #[test]
    fn old_leveling_becomes_automatic() {
        for (mode, want) in [(0, GainMode::Off), (1, GainMode::Auto), (2, GainMode::Auto), (3, GainMode::Auto)] {
            assert_eq!(open_old("old-leveling", &old(&[("replayGain", number(mode))])).0.replay_gain, want, "{mode}");
        }
    }

    #[test]
    fn old_loudness_target_becomes_nearest_level() {
        for (lufs, want) in [(-14.0, Level::Loud), (-16.0, Level::Loud), (-18.0, Level::Normal), (-23.0, Level::Quiet)] {
            assert_eq!(open_old("old-target", &old(&[("loudnessTarget", decimal(lufs))])).0.level, want, "{lufs}");
        }
    }

    #[test]
    fn old_crossfeed_and_virtualizer_become_headphone_sound() {
        for (crossfeed, virtualizer, want) in [(0.0, 0.0, HeadphoneSound::Off), (4.5, 0.0, HeadphoneSound::Natural), (9.5, 0.0, HeadphoneSound::Natural), (0.0, 0.3, HeadphoneSound::Wide), (3.0, 1.0, HeadphoneSound::Wide)] {
            let raw = old(&[("crossfeedDb", decimal(crossfeed)), ("virtualizer", decimal(virtualizer))]);
            assert_eq!(open_old("old-headphone", &raw).0.headphone, want, "{crossfeed} dB, {virtualizer}");
        }
    }

    #[test]
    fn old_compressor_controls_become_nearest_preset() {
        let gentle = [("compThresholdDb", -16.0), ("compRatio", 2.0), ("compAttackMs", 20.0), ("compReleaseMs", 250.0), ("compMakeupDb", 2.5), ("compKneeDb", 6.0)];
        let strong = [("compThresholdDb", -28.0), ("compRatio", 5.0), ("compAttackMs", 5.0), ("compReleaseMs", 120.0), ("compMakeupDb", 8.0), ("compKneeDb", 8.0)];
        let quick = [("compThresholdDb", -20.0), ("compRatio", 8.0), ("compAttackMs", 1.0), ("compReleaseMs", 150.0), ("compMakeupDb", 4.0), ("compKneeDb", 6.0)];
        for (knobs, want) in [(gentle, CompressorPreset::Light), (strong, CompressorPreset::Car), (quick, CompressorPreset::Night)] {
            let mut raw = old(&[("compressor", flag(true))]);
            raw.extend(knobs.iter().map(|(k, v)| (k.to_string(), decimal(*v))));
            let p = open_old("old-compressor", &raw).0;
            assert!(p.compressor);
            assert_eq!(p.compressor_preset, want, "{knobs:?}");
        }
        assert_eq!(open_old("old-compressor-default", &stored_062(false)).0.compressor_preset, CompressorPreset::Light, "the old default");
    }

    #[test]
    fn old_bass_boost_becomes_the_equalizer_shelf() {
        // The graphic equalizer off: on, flat but for the shelf, its pre-amp automatic.
        let p = open_old("old-bass-graphic", &old(&[("bassBoostDb", decimal(6.0))])).0;
        assert!(p.eq_enabled && p.eq_mode == EqMode::Graphic && p.eq_preamp_db.is_none());
        assert!(p.eq_graphic[0] > 4.0 && p.eq_graphic[9].abs() < 0.1, "{:?}", p.eq_graphic);
        let shelf = nori_player::dsp::bass_shelf(6.0);
        let want = nori_player::graphic::sliders_for(&[nori_player::dsp::Band::from(&shelf)], 10, EQ_RANGES.gain.max as f64);
        assert_eq!(p.eq_graphic, want.iter().map(|v| EQ_RANGES.gain.hold(*v as f32)).collect::<Vec<_>>(), "the same curve as the bass boost");
        // A parametric equalizer in use keeps its bands and gains the shelf, its own pre-amp making room.
        let raw = old(&[("bassBoostDb", decimal(4.0)), ("eqMode", number(0)), ("eqEnabled", flag(true)), ("eqBands", PrefValue::Text { v: "0:1000.0:-2.0:1.0:0".into() }), ("eqPreampDb", decimal(-1.0))]);
        let p = open_old("old-bass-parametric", &raw).0;
        assert_eq!(p.eq_bands, [band_from(0, 1000.0, -2.0, 1.0, 0), band_from(8, 100.0, 4.0, 0.8, 0)]);
        assert_eq!(p.eq_preamp_db, Some(-5.0));
        assert!(!open_old("old-no-bass", &old(&[("bassBoostDb", decimal(0.0))])).0.eq_enabled, "no boost: the equalizer as it was");
    }

    #[test]
    fn pure_black_with_the_dark_theme_becomes_midnight() {
        use ThemeMode::*;
        // (theme, pure black) -> theme; 0 is the system's, 1 light, 2 dark.
        for (theme, pure_black, want) in [(2, true, Midnight), (2, false, Dark), (0, true, System), (1, true, Light), (3, false, Midnight)] {
            let raw = old(&[("theme", number(theme)), ("amoled", flag(pure_black)), ("coverColorsOn", PrefValue::Text { v: "PAGES".into() })]);
            let (p, stored) = open_old("pure-black", &raw);
            assert_eq!(p.theme, want, "theme {theme}, pure black {pure_black}");
            assert!(!stored.contains_key("amoled") && !stored.contains_key("coverColorsOn"), "{stored:?}");
            assert_eq!(stored, save(&p), "stored once, as read");
        }
    }

    #[test]
    fn old_picks_become_conditions() {
        use crate::settings::AwakeWhen as A;
        use crate::settings::HideBarWhen as H;
        let bars = [(0, vec![]), (1, vec![H::Sideways]), (2, vec![H::Upright]), (3, vec![H::Sideways, H::Upright]), (4, vec![H::Protected])];
        for (pick, want) in bars {
            let (p, stored) = open_old("old-bars", &old(&[("hideStatusBar", number(pick)), ("hideGestureBar", number(pick))]));
            assert_eq!(p.hide_status_bar, want, "{pick}");
            assert!(!stored.contains_key("hideStatusBar") && !stored.contains_key("hideGestureBar"), "{stored:?}");
        }
        let awake = [(0, vec![]), (1, vec![A::Sideways]), (2, vec![A::Charging]), (3, vec![A::Sideways, A::Charging]), (4, vec![A::Sideways, A::Upright])];
        for (pick, want) in awake {
            assert_eq!(open_old("old-awake", &old(&[("keepAwake", number(pick))])).0.keep_awake, want, "{pick}");
        }
        // Stored by 0.6.3's first builds, with no key of 0.6.2's left.
        let raw = HashMap::from([("coverColorsWhere".to_string(), number(2)), ("keepAwake".to_string(), number(4)), ("motion".to_string(), number(2))]);
        let (p, stored) = open_old("old-places", &raw);
        assert_eq!((p.keep_awake, p.motion), (vec![A::Sideways, A::Upright], crate::settings::Motion::Eink));
        assert_eq!(stored, save(&load(&stored)), "stored once, as read");
        assert!(!crate::settings::PICK_KEYS.iter().any(|k| stored.contains_key(*k)) && !stored.contains_key("coverColorsWhere"), "{stored:?}");
    }

    #[test]
    fn old_motion_switches_become_motion() {
        use crate::settings::Motion::*;
        for (reduce, eink, want) in [(false, false, Full), (true, false, Reduced), (false, true, Eink), (true, true, Eink)] {
            let raw = old(&[("reduceMotion", flag(reduce)), ("einkScreen", flag(eink)), ("animateAnyway", flag(false))]);
            assert_eq!(open_old("old-motion", &raw).0.motion, want, "reduce {reduce}, e-ink {eink}");
        }
    }

    #[test]
    fn old_crossfade_fades_become_the_longer() {
        for (length, fade_in, fade_out, want) in [(6, 0, 0, 6), (6, 2, 4, 4), (8, 1, 0, 8), (4, 6, 2, 4), (0, 2, 2, 0)] {
            let raw = old(&[("crossfadeSec", number(length)), ("crossfadeInSec", number(fade_in)), ("crossfadeOutSec", number(fade_out))]);
            assert_eq!(open_old("old-crossfade", &raw).0.crossfade_sec, want, "{length} s, in {fade_in}, out {fade_out}");
        }
    }

    #[test]
    fn old_playlist_descriptions_become_playlist_notes() {
        for shown in [false, true] {
            let raw = old(&[("playlistDescriptions", flag(shown)), ("hideImportNotes", flag(!shown))]);
            assert_eq!(open_old("old-notes", &raw).0.playlist_notes, shown);
        }
    }

    #[test]
    fn old_sound_profiles_are_stored_new() {
        let dir = nori_testdir::TempDir::new("old-profiles");
        let path = dir.join("nori.db").display().to_string();
        let profile = r#"{"eqEnabled":false,"eqMode":1,"crossfeedDb":4.5,"crossfeedHz":700.0,"virtualizer":0.0,"bassBoostDb":3.0,"compressor":true,"compThresholdDb":-28.0,"compRatio":5.0,"compAttackMs":5.0,"compReleaseMs":120.0,"compMakeupDb":8.0,"compKneeDb":8.0,"replayGain":2,"preampDb":1.0,"crossfadeSec":4}"#;
        {
            let c = db::open_app(&path).unwrap();
            for (k, v) in stored_062(false) {
                c.execute("INSERT INTO settings(key, value) VALUES(?1, ?2)", params![k, to_json(&v)]).unwrap();
            }
            c.execute_batch("CREATE TABLE profiles(name TEXT PRIMARY KEY, json TEXT NOT NULL, outputs TEXT NOT NULL DEFAULT '') WITHOUT ROWID").unwrap();
            c.execute("INSERT INTO profiles(name, json) VALUES('Car', ?1)", [profile]).unwrap();
        }
        SettingsStore::open(&path).unwrap();
        let json: String = db::open_app(&path).unwrap().query_row("SELECT json FROM profiles", [], |r| r.get(0)).unwrap();
        assert!(!crate::settings::legacy_profile(&json), "{json}");
        let s = crate::settings::sound_from(&json).unwrap();
        assert_eq!((s.effects.headphone, s.effects.compressor_preset, s.replay_gain, s.crossfade_sec), (HeadphoneSound::Natural, CompressorPreset::Car, GainMode::Auto, 4));
        assert!(s.effects.compressor && s.eq_enabled && s.eq_graphic[0] > 2.0, "the bass boost on the equalizer");
        assert_eq!(crate::settings::sound_from(profile).unwrap(), s, "read the same before it is stored again");
    }
}
