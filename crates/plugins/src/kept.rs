//! The plugins as a backup keeps them, and putting them back: each reinstalled from its repo, its
//! store written again (its rows, overrides and secrets), its place, being on and being held kept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{store, Plugins};

/// The installed plugins, in their order, and the repos the user added.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KeptPlugins {
    #[serde(default)]
    pub plugins: Vec<KeptPlugin>,
    #[serde(default)]
    pub repos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptPlugin {
    pub id: String,
    pub version: String,
    /// None when it was installed from a file, which no restore can fetch again.
    #[serde(default)]
    pub repo: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub held: bool,
    /// Every value in its store, by key, as base64.
    #[serde(default, with = "base64_values")]
    pub store: BTreeMap<String, Vec<u8>>,
}

mod base64_values {
    use std::collections::BTreeMap;

    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(m: &BTreeMap<String, Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        m.iter().map(|(k, v)| (k, STANDARD.encode(v))).collect::<BTreeMap<_, _>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
        BTreeMap::<String, String>::deserialize(d)?.into_iter().map(|(k, v)| STANDARD.decode(v).map(|b| (k, b)).map_err(serde::de::Error::custom)).collect()
    }
}

/// How a backed-up plugin came back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PluginReturn {
    /// At the version backed up.
    AsKept,
    /// At another version: its repo no longer lists the one backed up.
    Other { version: String },
    /// Not here: it was installed from a file.
    FromFile,
    /// Not here: its repo doesn't list it now, or couldn't be read.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PluginRestored {
    pub id: String,
    pub version: String,
    pub outcome: PluginReturn,
}

impl Plugins {
    /// The installed plugins as a backup keeps them.
    pub fn kept(&self) -> KeptPlugins {
        let plugins = self
            .installed()
            .into_iter()
            .map(|i| {
                let id = i.manifest.id.clone();
                let s = store::Store::new(self.folder(&id).join("store"));
                let store = s.keys("").into_iter().filter_map(|k| Some((k.clone(), s.get(&k)?))).collect();
                KeptPlugin { id, version: i.manifest.version.clone(), repo: i.repo, enabled: i.enabled, held: i.held, store }
            })
            .collect();
        KeptPlugins { plugins, repos: self.added_repos() }
    }

    /// Puts back the plugins `kept` holds: each fetched from its repo at its version (else the newest
    /// listed, else the one installed here), its store replaced by the one kept, then the order (theirs
    /// first, the others after), being on and being held as kept. Plugins installed here and not in
    /// the backup stay.
    pub async fn restore(&self, kept: KeptPlugins) -> Vec<PluginRestored> {
        for repo in kept.repos {
            self.add_repo(repo);
        }
        let offers = if kept.plugins.is_empty() { Vec::new() } else { self.offers().await.offers };
        let mut out = Vec::new();
        for k in &kept.plugins {
            let here = self.installed().into_iter().find(|i| i.manifest.id == k.id).map(|i| i.manifest.version.clone());
            let listed = k.repo.as_ref().and_then(|repo| offers.iter().find(|o| o.id == k.id && o.repo.url == *repo));
            let wanted = listed.and_then(|o| o.versions.iter().find(|v| v.version == k.version).or(o.versions.first()));
            let now = match wanted {
                _ if here.as_deref() == Some(k.version.as_str()) => here,
                Some(v) if here.as_deref() == Some(v.version.as_str()) => here,
                Some(v) => match self.install_version(v.clone()).await {
                    Ok(_) => self.installed().into_iter().find(|i| i.manifest.id == k.id).map(|i| i.manifest.version.clone()),
                    Err(e) => {
                        nori_model::alog::info(&format!("restore: plugin {}: {e:?}", k.id));
                        here
                    }
                },
                None => here,
            };
            let outcome = match &now {
                Some(v) if *v == k.version => PluginReturn::AsKept,
                Some(v) => PluginReturn::Other { version: v.clone() },
                None if k.repo.is_none() => PluginReturn::FromFile,
                None => PluginReturn::Unavailable,
            };
            if now.is_some() {
                self.workers.lock().expect("not poisoned").remove(&k.id);
                self.lan.forget(&k.id);
                let dir = self.folder(&k.id).join("store");
                let _ = std::fs::remove_dir_all(&dir);
                let s = store::Store::new(dir);
                for (key, value) in &k.store {
                    s.set(key, value);
                }
            }
            out.push(PluginRestored { id: k.id.clone(), version: k.version.clone(), outcome });
        }
        let back: Vec<&KeptPlugin> = kept.plugins.iter().filter(|k| self.installed().iter().any(|i| i.manifest.id == k.id)).collect();
        {
            let mut list = self.installed.lock().expect("not poisoned");
            let (mut ordered, rest): (Vec<_>, Vec<_>) = list.drain(..).partition(|i| back.iter().any(|k| k.id == i.manifest.id));
            ordered.sort_by_key(|i| back.iter().position(|k| k.id == i.manifest.id));
            *list = ordered.into_iter().chain(rest).collect();
            self.keep(&list);
        }
        // As the user would: each held or not, then turned on or off, which lays it over the settings
        // again and starts its lifecycle.
        for k in &back {
            self.hold(k.id.clone(), k.held);
            self.turn(k.id.clone(), k.enabled);
        }
        out
    }
}
