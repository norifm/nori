//! The plugin host as a client runs it: over the app's settings, which it lays the enabled plugins'
//! overrides and sound stages over, and the open profile's transport.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use nori_settings::overrides::Layer;
use nori_settings::settings_model::value_of;
use nori_settings::settings_store::Settings;

use crate::host::{Side, Value};
use crate::Plugins;

/// What a client does with what plugins say.
pub trait Client: Send + Sync {
    /// The settings' layer changed: the client derives the engine's settings again.
    fn layer_changed(&self);
    /// A status row's line, from plugin `id`'s strings.
    fn status(&self, id: &str, row: &str, words: &str, args: &[String]);
    fn shown(&self, id: &str, row: &str, shown: bool, enabled: bool);
    fn notice(&self, id: &str, problem: bool, words: &str, args: &[String]);
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
        self.settings.set_layer(Layer { overrides: plugins.overrides(), stages: plugins.stages() });
        self.client.layer_changed();
    }

    fn status(&self, id: &str, row: &str, words: &str, args: &[String]) {
        self.client.status(id, row, words, args);
    }

    fn shown(&self, id: &str, row: &str, shown: bool, enabled: bool) {
        self.client.shown(id, row, shown, enabled);
    }

    fn notice(&self, id: &str, problem: bool, words: &str, args: &[String]) {
        self.client.notice(id, problem, words, args);
    }
}

/// The plugin host for the app: the plugins installed under `dir`, laid over `settings` (so the
/// first settings derived already have them).
pub fn open(dir: PathBuf, settings: Arc<Settings>, client: Arc<dyn Client>) -> Arc<Plugins> {
    let app = Arc::new(App { settings, client, plugins: OnceLock::new() });
    let plugins = Arc::new(Plugins::open(dir, app.clone()));
    let _ = app.plugins.set(Arc::downgrade(&plugins));
    if plugins.installed().iter().any(|i| i.enabled) {
        app.settings.set_layer(Layer { overrides: plugins.overrides(), stages: plugins.stages() });
    }
    plugins
}
