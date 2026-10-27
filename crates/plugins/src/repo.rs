//! Plugin repos: static `index.json` files anyone can host. nori's own is always there; the user adds
//! others by URL (a client warns first: nori vouches only for its own). An index lists each plugin's
//! id, name, version, plugin API version, package URL and the package's sha256, which an install
//! checks before anything else.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Plugins, Refusal, API};

/// nori's own repo.
pub const OFFICIAL: &str = "https://raw.githubusercontent.com/norifm/nori-plugins/main/dist/index.json";

/// A repo the plugins come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Repo {
    pub url: String,
    /// nori's own.
    pub official: bool,
}

/// One plugin as an index lists it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Listed {
    id: String,
    name: String,
    #[serde(default)]
    about: Option<String>,
    version: String,
    api: u32,
    /// Absolute, or relative to the index.
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct Index {
    plugins: Vec<Listed>,
}

/// A plugin a repo offers.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Offer {
    pub repo: Repo,
    pub id: String,
    pub name: String,
    pub about: Option<String>,
    pub version: String,
    /// The version installed here, if it is.
    pub installed: Option<String>,
    /// Package URL and its sha256.
    url: String,
    sha256: String,
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

/// The offers of `index`'s text, from `repo`, that this host can run.
fn offers(repo: &Repo, text: &[u8]) -> Result<Vec<Offer>, String> {
    let index: Index = serde_json::from_slice(text).map_err(|e| format!("not an index: {e}"))?;
    Ok(index
        .plugins
        .into_iter()
        .filter(|l| l.api == API || l.api + 1 == API)
        .map(|l| Offer { repo: repo.clone(), url: resolved(&repo.url, &l.url), id: l.id, name: l.name, about: l.about, version: l.version, installed: None, sha256: l.sha256.to_ascii_lowercase() })
        .collect())
}

impl Plugins {
    pub(crate) fn transport_now(&self) -> Result<std::sync::Arc<dyn nori_net::transport::Transport>, String> {
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

    /// Fetches and installs `offer` (an update keeps the plugin's store and its being on); its id.
    pub async fn install_offer(&self, offer: Offer) -> Result<String, Refusal> {
        let unreadable = |e: String| Refusal::Unreadable(e);
        let t = self.transport_now().map_err(unreadable)?;
        let r = t.get(offer.url.clone(), 0).await.map_err(|e| unreadable(e.to_string()))?;
        if !(200..=299).contains(&r.status) {
            return Err(unreadable(format!("HTTP {}", r.status)));
        }
        let hash: String = Sha256::digest(&r.body).iter().map(|b| format!("{b:02x}")).collect();
        if hash != offer.sha256 {
            return Err(unreadable("the package isn't the one its repo lists".into()));
        }
        let file = self.dir.join(format!(".{}.noriplugin", offer.id));
        std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&file, &r.body)).map_err(|e| unreadable(e.to_string()))?;
        let installed = self.install(&file).map(|m| m.id.clone());
        let _ = std::fs::remove_file(&file);
        installed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_offers_what_this_host_runs_at_its_urls() {
        let repo = Repo { url: "https://example.com/nori/index.json".into(), official: false };
        let text = br#"{"plugins": [
            {"id": "a.b", "name": "A", "version": "1.0.0", "api": 1, "url": "a.noriplugin", "sha256": "AB"},
            {"id": "c.d", "name": "C", "version": "2.0.0", "api": 1, "url": "https://cdn.example/c.noriplugin", "sha256": "cd"},
            {"id": "e.f", "name": "E", "version": "1.0.0", "api": 9, "url": "e.noriplugin", "sha256": "ef"}
        ]}"#;
        let o = offers(&repo, text).unwrap();
        assert_eq!(o.iter().map(|o| (o.id.as_str(), o.url.as_str(), o.sha256.as_str())).collect::<Vec<_>>(), vec![("a.b", "https://example.com/nori/a.noriplugin", "ab"), ("c.d", "https://cdn.example/c.noriplugin", "cd")], "another API's plugin isn't offered");
        assert!(offers(&repo, b"<html>").is_err());
    }
}
