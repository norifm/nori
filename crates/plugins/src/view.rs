//! What a client draws of the plugins: each one's card on Settings › Plugins, and its page of rows,
//! in the plugin's own words (its `strings/`, English when the user's language is missing: the one
//! place words come from below the clients).

use std::collections::BTreeMap;

use crate::{Plugins, Refusal, RowKind, Value};

/// A plugin as Settings › Plugins lists it, in the user's plugin order.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PluginCard {
    pub id: String,
    pub version: String,
    /// Its `name` string, else its id.
    pub name: String,
    /// Its `about` string, if it has one.
    pub about: Option<String>,
    pub enabled: bool,
    /// The repo it came from (it updates from there alone); None when installed from a file.
    pub repo: Option<String>,
    /// Kept at its version.
    pub held: bool,
    /// The extension points it fills (`wit/`'s names).
    pub points: Vec<String>,
    /// The nori values it overrides.
    pub overrides: Vec<String>,
    /// Hosts it sends HTTP to.
    pub net: Vec<String>,
    pub rows: Vec<PluginRow>,
}

/// One row of a plugin's page.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PluginRow {
    pub id: String,
    pub kind: RowKind,
    pub label: String,
    /// A choice's options, as said.
    pub options: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// As stored, else its default.
    pub value: Option<Value>,
}

/// A plugin's words in `lang` (`en` when missing), from its folder's `strings/`.
pub(crate) fn words(dir: &std::path::Path, lang: &str) -> BTreeMap<String, String> {
    let read = |l: &str| -> Option<BTreeMap<String, String>> { toml::from_str(&std::fs::read_to_string(dir.join("strings").join(format!("{l}.toml"))).ok()?).ok() };
    let mut w = read("en").unwrap_or_default();
    if lang != "en" {
        w.extend(read(lang).unwrap_or_default());
    }
    w
}

/// A row's default as a value of its kind.
fn default(kind: RowKind, d: Option<f64>) -> Option<Value> {
    let d = d?;
    Some(match kind {
        RowKind::Switch => Value::Switch(d != 0.0),
        RowKind::Choice => Value::Choice(d as u32),
        _ => Value::Level(d),
    })
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// The installed plugins in `lang`, in their order.
    pub fn cards(&self, lang: String) -> Vec<PluginCard> {
        self.installed()
            .into_iter()
            .map(|i| {
                let m = &i.manifest;
                let w = words(&self.folder(&m.id), &lang);
                let said = |k: &str| w.get(k).cloned();
                let stored = self.rows(&m.id);
                PluginCard {
                    id: m.id.clone(),
                    version: m.version.clone(),
                    name: said("name").unwrap_or_else(|| m.id.clone()),
                    about: said("about"),
                    enabled: i.enabled,
                    repo: i.repo.clone(),
                    held: i.held,
                    points: m.points.iter().map(|p| p.to_string()).collect(),
                    overrides: m.overrides.clone(),
                    net: m.net.clone(),
                    rows: m
                        .rows
                        .iter()
                        .map(|r| PluginRow {
                            id: r.id.clone(),
                            kind: r.kind,
                            label: said(&r.id).unwrap_or_else(|| r.id.clone()),
                            options: r.options.iter().map(|o| said(o).unwrap_or_else(|| o.clone())).collect(),
                            min: r.min,
                            max: r.max,
                            step: r.step,
                            value: stored.get(&r.id).cloned().or_else(|| default(r.kind, r.default)),
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Installs the `.noriplugin` or plugin folder at `path`, off; its id.
    pub fn install_from(&self, path: String) -> Result<String, Refusal> {
        self.install(std::path::Path::new(&path)).map(|m| m.id.clone())
    }

    pub fn turn(&self, id: String, on: bool) {
        self.set_enabled(&id, on)
    }

    pub fn place(&self, id: String, index: u32) {
        self.move_to(&id, index as usize)
    }

    pub fn uninstall(&self, id: String) {
        self.remove(&id)
    }

    pub fn set(&self, id: String, row: String, value: Value) {
        self.set_row(&id, &row, value)
    }

    /// Runs action row `row` of plugin `id`.
    pub fn act(&self, id: String, row: String) {
        if let Some(w) = self.worker(&id) {
            w.post(move |s, p| p.nori_plugin_lifecycle().call_action(s, &row));
        }
    }

    /// Plugin `id`'s word `key` in `lang`.
    pub fn word(&self, id: String, key: String, lang: String) -> Option<String> {
        words(&self.folder(&id), &lang).remove(&key)
    }
}
