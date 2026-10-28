//! What a client draws of the plugins: each one's card on Settings › Plugins, and its page of rows,
//! in the plugin's own words (its `strings/`, English when the user's language is missing: the one
//! place words come from below the clients).

use std::collections::BTreeMap;

use crate::{Manifest, Plugins, Refusal, RowKind, Value};

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
    /// A quiet line under its label.
    pub hint: Option<String>,
    /// A level's unit, after its value.
    pub unit: Option<String>,
    /// Digits a level's value is shown with.
    pub decimals: u8,
    /// A link row's address.
    pub url: Option<String>,
}

/// A plugin not installed yet, as its install sheet shows it, and its package.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PluginPreview {
    pub card: PluginCard,
    /// The package: the file picked, or the one fetched from a repo.
    pub file: String,
    /// The repo it comes from, which will update it.
    pub repo: Option<String>,
}

/// A plugin's words in `lang` (`en` for what that lacks), each language's strings file read by `read`.
fn words_by(read: impl Fn(&str) -> Option<String>, lang: &str) -> BTreeMap<String, String> {
    let mut w = read("en").map(|t| flat(&t)).unwrap_or_default();
    if lang != "en" {
        w.extend(read(lang).map(|t| flat(&t)).unwrap_or_default());
    }
    w
}

/// An installed plugin's words in `lang`, from its folder's `strings/`.
pub(crate) fn words(dir: &std::path::Path, lang: &str) -> BTreeMap<String, String> {
    words_by(|l| std::fs::read_to_string(dir.join("strings").join(format!("{l}.toml"))).ok(), lang)
}

/// A strings file: one `key = "text"` per line (TOML's basic strings and comments); any other line is
/// skipped.
fn flat(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once('=') else { continue };
        let (key, rest) = (key.trim(), rest.trim());
        let Some(body) = rest.strip_prefix('"') else { continue };
        let mut value = String::new();
        let mut chars = body.chars();
        let closed = loop {
            match chars.next() {
                None => break false,
                Some('"') => break true,
                Some('\\') => match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    Some(c) => value.push(c),
                    None => break false,
                },
                Some(c) => value.push(c),
            }
        };
        if closed && !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            out.insert(key.to_string(), value);
        }
    }
    out
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

/// What a client shows of a plugin of manifest `m`, in its words `w`, with its rows' `stored` values.
fn card(m: &Manifest, w: &BTreeMap<String, String>, stored: &BTreeMap<String, Value>, enabled: bool, repo: Option<String>, held: bool) -> PluginCard {
    let said = |k: &str| w.get(k).cloned();
    PluginCard {
        id: m.id.clone(),
        version: m.version.clone(),
        name: said("name").unwrap_or_else(|| m.id.clone()),
        about: said("about"),
        enabled,
        repo,
        held,
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
                hint: r.hint.as_deref().and_then(said),
                unit: r.unit.as_deref().and_then(said),
                decimals: r.decimals,
                url: r.url.clone(),
            })
            .collect(),
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// The installed plugins in `lang`, in their order.
    pub fn cards(&self, lang: String) -> Vec<PluginCard> {
        self.installed().into_iter().map(|i| card(&i.manifest, &words(&self.folder(&i.manifest.id), &lang), &self.rows(&i.manifest.id), i.enabled, i.repo.clone(), i.held)).collect()
    }

    /// The `.noriplugin` or plugin folder at `path` as its install sheet shows it, in `lang`; nothing is
    /// installed.
    pub fn preview_file(&self, path: String, lang: String) -> Result<PluginPreview, Refusal> {
        let files = crate::package::files(std::path::Path::new(&path))?;
        let get = |n: &str| files.iter().find(|(name, _)| name == n).map(|(_, b)| b);
        let m = Manifest::of_wasm(get("plugin.wasm").ok_or_else(|| Refusal::Unreadable("no plugin.wasm".into()))?)?;
        let w = words_by(|l| get(&format!("strings/{l}.toml")).map(|b| String::from_utf8_lossy(b).into_owned()), &lang);
        Ok(PluginPreview { card: card(&m, &w, &BTreeMap::new(), false, None, false), file: path, repo: None })
    }

    /// `version` of a plugin a repo offers, fetched (and checked against its sha256) and shown as its
    /// install sheet shows it.
    pub async fn preview_version(&self, version: crate::Version, lang: String) -> Result<PluginPreview, Refusal> {
        let file = self.fetch(&version).await?;
        let mut p = self.preview_file(file.to_string_lossy().into_owned(), lang)?;
        p.repo = Some(version.repo);
        Ok(p)
    }

    /// Installs what `preview` showed, off when new; its id. A package fetched for it is let go.
    pub fn install_preview(&self, preview: PluginPreview) -> Result<String, Refusal> {
        let file = std::path::PathBuf::from(&preview.file);
        let installed = self.install_as(&file, preview.repo).map(|m| m.id.clone());
        if file.parent() == Some(self.dir.as_path()) {
            let _ = std::fs::remove_file(&file);
        }
        installed
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strings_file_reads_as_its_keys_and_texts() {
        let text = "# Orbit\nname = \"Orbit\"\nabout = \"Circles \\\"you\\\"\\nround\"\nunit-s = \"\\u00e9\"\n[table]\nbroken = \"no end\n";
        let w = flat(text);
        assert_eq!(w.get("name").map(String::as_str), Some("Orbit"));
        assert_eq!(w.get("about").map(String::as_str), Some("Circles \"you\"\nround"));
        assert_eq!(w.get("unit-s").map(String::as_str), Some("é"));
        assert!(!w.contains_key("broken") && w.len() == 3);
    }
}
