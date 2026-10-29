//! The plugin host as a client runs it: over the app's settings, which it lays the enabled plugins'
//! overrides and sound stages over, and the open profile's transport.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use nori_settings::settings_model::value_of;
use nori_settings::settings_store::Settings;

use nori_settings::settings::StoredPrefs;

use crate::host::{Side, Value};
use crate::{AssetState, Command, Control, Hook, Plugins, WakeEvent};

/// What a client does with what plugins say.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait Client: Send + Sync {
    /// The settings' layer changed: the client applies `effect` again (the settings' effect bits) and
    /// derives the engine's settings with it.
    fn layer_changed(&self, effect: u32);
    /// A status row's line, from plugin `id`'s strings.
    fn status(&self, id: String, row: String, words: String, args: Vec<String>);
    fn shown(&self, id: String, row: String, shown: bool, enabled: bool);
    fn notice(&self, id: String, problem: bool, words: String, args: Vec<String>);
    /// A plugin asks the player to do `control`, as a press of the client's own would.
    fn control(&self, control: Control);
    /// Opens the sign-in page of plugin `id`'s provider at `url`.
    fn sign_in(&self, id: String, url: String);
    /// Keep on only the receivers that start nori for these.
    fn wake(&self, events: Vec<WakeEvent>);
    /// The commands other apps may run now; with none, no receiver or bus name.
    fn commands(&self, commands: Vec<Command>);
    /// Say whether the screen is on ([`Plugins::screen`]) from now on, or stop.
    fn watch_screen(&self, on: bool);
    /// Plugin `id`'s asset `name` is now at `state`: its line on the plugin's page.
    fn asset(&self, id: String, name: String, state: AssetState);
}

struct App {
    settings: Arc<Settings>,
    client: Arc<dyn Client>,
    plugins: Arc<OnceLock<Weak<Plugins>>>,
}

/// Each setting whose value differs between `before` and `after`, as a plugin reads it.
fn changes(before: &StoredPrefs, after: &StoredPrefs) -> Vec<(String, Value)> {
    nori_settings::settings_model::specs()
        .into_iter()
        .filter_map(|s| {
            let now = value_of(after, &s.name)?;
            (value_of(before, &s.name).as_ref() != Some(&now)).then(|| (s.name, value(now)))
        })
        .collect()
}

/// A setting's text as a plugin reads it.
fn value(text: String) -> Value {
    match text.as_str() {
        "true" => Value::Switch(true),
        "false" => Value::Switch(false),
        t => t.parse::<f64>().map_or(Value::Text(text), Value::Level),
    }
}

impl Side for App {
    fn stored(&self, key: &str) -> Option<Value> {
        self.settings.current().and_then(|p| value_of(&p, key)).map(value)
    }

    fn effective(&self, key: &str) -> Option<Value> {
        let p = self.settings.current()?;
        value_of(&nori_settings::overrides::effective(&p, &self.settings.layer().overrides), key).map(value)
    }

    fn laid(&self, _id: &str) {
        let Some(plugins) = self.plugins.get().and_then(Weak::upgrade) else { return };
        let effective = |s: &Settings| s.current().map(|p| nori_settings::overrides::effective(&p, &s.layer().overrides));
        let before = plugins.hears(Hook::SettingChanged).then(|| effective(&self.settings)).flatten();
        let effect = self.settings.set_layer(plugins.layer());
        if effect != 0 {
            self.client.layer_changed(effect);
        }
        if let (Some(before), Some(after)) = (before, effective(&self.settings)) {
            for (key, value) in changes(&before, &after) {
                plugins.setting_changed(&key, value);
            }
        }
    }

    fn status(&self, id: &str, row: &str, words: &str, args: &[String]) {
        self.client.status(id.into(), row.into(), words.into(), args.to_vec());
    }

    fn shown(&self, id: &str, row: &str, shown: bool, enabled: bool) {
        self.client.shown(id.into(), row.into(), shown, enabled);
    }

    fn notice(&self, id: &str, problem: bool, words: &str, args: &[String]) {
        self.client.notice(id.into(), problem, words.into(), args.to_vec());
    }

    fn control(&self, c: Control) {
        self.client.control(c);
    }

    fn sign_in(&self, id: &str, url: &str) -> bool {
        self.client.sign_in(id.into(), url.into());
        true
    }

    fn wake(&self, events: &[WakeEvent]) {
        self.client.wake(events.to_vec());
    }

    fn commands(&self, commands: &[Command]) {
        self.client.commands(commands.to_vec());
    }

    fn watch_screen(&self, on: bool) {
        self.client.watch_screen(on);
    }

    fn asset(&self, id: &str, name: &str, state: AssetState) {
        self.client.asset(id.into(), name.into(), state);
    }

    /// The stored settings' changes; those of what is in effect come with the layer ([`Side::laid`]).
    fn watch_settings(&self, on: bool) {
        let plugins = self.plugins.clone();
        let watch: Option<nori_settings::settings_store::Watch> = on.then(|| {
            Arc::new(move |before: &StoredPrefs, after: &StoredPrefs| {
                let Some(p) = plugins.get().and_then(Weak::upgrade) else { return };
                for (key, value) in changes(before, after) {
                    p.setting_changed(&key, value);
                }
            }) as nori_settings::settings_store::Watch
        });
        self.settings.watch(watch);
    }
}

/// The plugin host for the app, for a client over uniffi ([`open`]).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn open_plugins(dir: String, settings: Arc<Settings>, client: Arc<dyn Client>) -> Arc<Plugins> {
    open(PathBuf::from(dir), settings, client)
}

/// The plugin host for the app: the plugins installed under `dir`, laid over `settings` (so the
/// first settings derived already have them).
pub fn open(dir: PathBuf, settings: Arc<Settings>, client: Arc<dyn Client>) -> Arc<Plugins> {
    let app = Arc::new(App { settings, client, plugins: Arc::default() });
    let plugins = Plugins::open(dir, app.clone());
    let _ = app.plugins.set(Arc::downgrade(&plugins));
    if plugins.installed().iter().any(|i| i.enabled) {
        app.laid("");
    }
    plugins
}
