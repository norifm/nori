//! The plugin host as a client runs it: over the app's settings, which it lays the enabled plugins'
//! overrides and sound stages over, and the open profile's transport.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use nori_settings::settings_model::value_of;
use nori_settings::settings_store::Settings;

use crate::host::{Side, Value};
use crate::Plugins;

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
}

struct App {
    settings: Arc<Settings>,
    client: Arc<dyn Client>,
    plugins: OnceLock<Weak<Plugins>>,
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
        let effect = self.settings.set_layer(plugins.layer());
        if effect != 0 {
            self.client.layer_changed(effect);
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
}

/// The plugin host for the app, for a client over uniffi ([`open`]).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn open_plugins(dir: String, settings: Arc<Settings>, client: Arc<dyn Client>) -> Arc<Plugins> {
    open(PathBuf::from(dir), settings, client)
}

/// The plugin host for the app: the plugins installed under `dir`, laid over `settings` (so the
/// first settings derived already have them).
pub fn open(dir: PathBuf, settings: Arc<Settings>, client: Arc<dyn Client>) -> Arc<Plugins> {
    let app = Arc::new(App { settings, client, plugins: OnceLock::new() });
    let plugins = Arc::new(Plugins::open(dir, app.clone()));
    let _ = app.plugins.set(Arc::downgrade(&plugins));
    if plugins.installed().iter().any(|i| i.enabled) {
        app.laid("");
    }
    plugins
}
