//! Plugin repos: static `index.json` files anyone can host. nori's own is always there; the user adds
//! others by URL (a client warns first: nori vouches only for its own). An index lists each plugin's
//! id, name, about and source, and every version it published: its plugin API version, the day it was
//! released, its package's URL and the package's sha256, which an install checks before anything else.
//! Old versions stay listed, so a plugin can be taken back to one.

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Plugins, Refusal, API};

/// nori's own repo: nori-plugins' index (its packages are that repository's releases).
pub const OFFICIAL: &str = "https://raw.githubusercontent.com/norifm/nori-plugins/main/index.json";

/// How often updates are looked for, at most: once a day, when a client asks.
const UPDATE_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

/// A repo the plugins come from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Repo {
    pub url: String,
    /// nori's own.
    pub official: bool,
}

#[derive(Debug, Deserialize)]
struct Index {
    plugins: Vec<Listed>,
}

#[derive(Debug, Deserialize)]
struct Listed {
    id: String,
    name: String,
    #[serde(default)]
    about: Option<String>,
    #[serde(default)]
    source: Option<String>,
    versions: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct Release {
    version: String,
    api: u32,
    #[serde(default)]
    released: Option<String>,
    /// Absolute, or relative to the index.
    url: String,
    sha256: String,
}

/// A plugin a repo offers.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Offer {
    pub repo: Repo,
    pub id: String,
    pub name: String,
    pub about: Option<String>,
    /// Where its code is, when its repo says.
    pub source: Option<String>,
    /// The versions this nori runs, newest first.
    pub versions: Vec<Version>,
    /// The version installed here, if it is.
    pub installed: Option<String>,
}

/// One published version of a plugin.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Version {
    /// The repo that lists it.
    pub repo: String,
    pub version: String,
    /// The day it was released (YYYY-MM-DD), when its repo says.
    pub released: Option<String>,
    pub url: String,
    pub sha256: String,
}

/// A plugin updated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Updated {
    pub id: String,
    pub from: String,
    pub to: String,
}

/// Why a repo couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RepoFailure {
    pub repo: Repo,
    pub reason: String,
}

/// What the repos offer, and those that couldn't be read.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Offers {
    pub offers: Vec<Offer>,
    pub failed: Vec<RepoFailure>,
}

/// `url` relative to the index at `index`.
fn resolved(index: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_string();
    }
    let base = index.rsplit_once('/').map_or(index, |(b, _)| b);
    format!("{base}/{}", url.trim_start_matches("./"))
}

/// `a` < `b` as dotted versions, number by number.
fn older(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) < parts(b)
}

/// The plugins in `index`'s text, from `repo`, with the versions this host runs; one with none is left out.
fn offers(repo: &Repo, text: &[u8]) -> Result<Vec<Offer>, String> {
    let index: Index = serde_json::from_slice(text).map_err(|e| format!("not an index: {e}"))?;
    Ok(index
        .plugins
        .into_iter()
        .filter_map(|l| {
            let mut versions: Vec<Version> = l
                .versions
                .into_iter()
                .filter(|r| r.api == API || r.api + 1 == API)
                .map(|r| Version { repo: repo.url.clone(), url: resolved(&repo.url, &r.url), version: r.version, released: r.released, sha256: r.sha256.to_ascii_lowercase() })
                .collect();
            versions.sort_by(|a, b| if older(&a.version, &b.version) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less });
            (!versions.is_empty()).then(|| Offer { repo: repo.clone(), id: l.id, name: l.name, about: l.about, source: l.source, versions, installed: None })
        })
        .collect())
}

impl Plugins {
    fn transport_now(&self) -> Result<std::sync::Arc<dyn nori_net::transport::Transport>, String> {
        self.transport.lock().expect("not poisoned").clone().ok_or_else(|| "no connection yet".to_string())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// nori's repo, then those the user added.
    pub fn repos(&self) -> Vec<Repo> {
        std::iter::once(Repo { url: OFFICIAL.into(), official: true }).chain(self.added_repos().into_iter().map(|url| Repo { url, official: false })).collect()
    }

    /// Adds the repo at `url` (an index.json); a client warns before.
    pub fn add_repo(&self, url: String) {
        let url = url.trim().to_string();
        if url.is_empty() || url == OFFICIAL {
            return;
        }
        let mut added = self.added_repos();
        if !added.contains(&url) {
            added.push(url);
            self.keep_repos(added);
        }
    }

    pub fn remove_repo(&self, url: String) {
        let mut added = self.added_repos();
        added.retain(|u| *u != url);
        self.keep_repos(added);
    }

    /// What every repo offers that this host can run, each with the version installed here.
    pub async fn offers(&self) -> Offers {
        let installed: Vec<(String, String)> = self.installed().into_iter().map(|i| (i.manifest.id.clone(), i.manifest.version.clone())).collect();
        let mut out = Offers { offers: Vec::new(), failed: Vec::new() };
        for repo in self.repos() {
            let read = match self.transport_now() {
                Ok(t) => t.get(repo.url.clone(), 0).await.map_err(|e| e.to_string()).and_then(|r| match r.status {
                    200..=299 => offers(&repo, &r.body),
                    s => Err(format!("HTTP {s}")),
                }),
                Err(e) => Err(e),
            };
            match read {
                Ok(o) => out.offers.extend(o),
                Err(reason) => out.failed.push(RepoFailure { repo, reason }),
            }
        }
        for o in &mut out.offers {
            o.installed = installed.iter().find(|(id, _)| *id == o.id).map(|(_, v)| v.clone());
        }
        out
    }

    /// Fetches and installs `version` of a plugin a repo offers, newer or older than the one here (its
    /// store and its being on are kept); its id.
    pub async fn install_version(&self, version: Version) -> Result<String, Refusal> {
        let unreadable = |e: String| Refusal::Unreadable(e);
        let t = self.transport_now().map_err(unreadable)?;
        let r = t.get(version.url.clone(), 0).await.map_err(|e| unreadable(e.to_string()))?;
        if !(200..=299).contains(&r.status) {
            return Err(unreadable(format!("HTTP {}", r.status)));
        }
        let hash: String = Sha256::digest(&r.body).iter().map(|b| format!("{b:02x}")).collect();
        if hash != version.sha256 {
            return Err(unreadable("the package isn't the one its repo lists".into()));
        }
        let file = self.dir.join(".download.noriplugin");
        std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&file, &r.body)).map_err(|e| unreadable(e.to_string()))?;
        let installed = self.install_as(&file, Some(version.repo.clone())).map(|m| m.id.clone());
        let _ = std::fs::remove_file(&file);
        installed
    }

    /// Whether a day has passed since updates were last looked for, at `now_ms` (device ms).
    pub fn updates_due(&self, now_ms: i64) -> bool {
        now_ms - self.checked_ms() >= UPDATE_EVERY_MS || now_ms < self.checked_ms()
    }

    /// Updates each plugin not held to the newest version its own repo offers; what was updated.
    pub async fn update(&self, now_ms: i64) -> Vec<Updated> {
        let offers = self.offers().await;
        self.keep_checked(now_ms);
        let mut done = Vec::new();
        for i in self.installed().into_iter().filter(|i| !i.held) {
            let Some(repo) = &i.repo else { continue };
            let newest = offers.offers.iter().find(|o| o.id == i.manifest.id && o.repo.url == *repo).and_then(|o| o.versions.first());
            let Some(v) = newest.filter(|v| older(&i.manifest.version, &v.version)) else { continue };
            if self.install_version(v.clone()).await.is_ok() {
                done.push(Updated { id: i.manifest.id.clone(), from: i.manifest.version.clone(), to: v.version.clone() });
            }
        }
        done
    }

    /// Keeps plugin `id` at its version (`held`), or lets it update again.
    pub fn hold_at(&self, id: String, held: bool) {
        self.hold(&id, held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_offers_the_versions_this_host_runs_newest_first() {
        let repo = Repo { url: "https://example.com/nori/index.json".into(), official: false };
        let text = br#"{"plugins": [
            {"id": "a.b", "name": "A", "source": "https://git.example/a", "versions": [
                {"version": "0.9.0", "api": 1, "url": "a-0.9.0.noriplugin", "sha256": "AB"},
                {"version": "0.10.0", "api": 1, "url": "https://cdn.example/a-0.10.0.noriplugin", "sha256": "cd", "released": "2026-11-02"},
                {"version": "2.0.0", "api": 9, "url": "a-2.0.0.noriplugin", "sha256": "ef"}
            ]},
            {"id": "c.d", "name": "C", "versions": [{"version": "1.0.0", "api": 9, "url": "c.noriplugin", "sha256": "00"}]}
        ]}"#;
        let o = offers(&repo, text).unwrap();
        assert_eq!(o.len(), 1, "a plugin with no version this host runs isn't offered");
        let v: Vec<(&str, &str, &str)> = o[0].versions.iter().map(|v| (v.version.as_str(), v.url.as_str(), v.sha256.as_str())).collect();
        assert_eq!(v, vec![("0.10.0", "https://cdn.example/a-0.10.0.noriplugin", "cd"), ("0.9.0", "https://example.com/nori/a-0.9.0.noriplugin", "ab")], "newest first, as numbers; another API's left out");
        assert_eq!(o[0].source.as_deref(), Some("https://git.example/a"));
        assert!(offers(&repo, b"<html>").is_err());
    }
}
