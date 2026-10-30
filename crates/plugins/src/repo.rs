//! Plugin repos: static `index.json` files anyone can host. nori's own is always there; the user adds
//! others by URL (a client warns first: nori vouches only for its own). An index lists each plugin's
//! id, name, about and source, and every version it published: its plugin API version, the day it was
//! released, its package's URL and the package's sha256, which an install checks before anything else,
//! and its author's key with the key's ed25519 signature over that sha256 (its 32 bytes). Old versions
//! stay listed, so a plugin can be taken back to one.

use sha2::{Digest, Sha256};

use crate::{PluginCard, Plugins, Refusal, API};

pub use nori_plugin_index::OFFICIAL;

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
    /// Where its README is, when its repo says.
    pub readme: Option<String>,
    /// Where its icon is, when its repo says.
    pub icon: Option<String>,
    /// The extension points it fills, when its repo says.
    pub points: Vec<String>,
    /// The versions this nori runs, newest first.
    pub versions: Vec<Version>,
    /// The version installed here, if it is.
    pub installed: Option<String>,
    pub state: OfferState,
}

/// Where a plugin a repo offers stands here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum OfferState {
    New,
    /// This repo, the one it came from, has a newer version.
    Update,
    Installed,
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
    /// Its author's ed25519 public key (hex), and the key's signature over the package's sha256 (hex):
    /// both, or neither for an unsigned version.
    pub key: Option<String>,
    pub signature: Option<String>,
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

/// The bytes hex `text` spells; None when it isn't `N` of them.
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let mut out = [0; N];
    if text.len() != 2 * N {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Whether `signature` (hex) is ed25519 key `key`'s (hex) over `digest`.
fn signed(key: &str, signature: &str, digest: &[u8]) -> bool {
    let (Some(key), Some(signature)) = (unhex::<32>(key), unhex::<64>(signature)) else { return false };
    ed25519_dalek::VerifyingKey::from_bytes(&key).is_ok_and(|k| k.verify_strict(digest, &ed25519_dalek::Signature::from_bytes(&signature)).is_ok())
}

/// A dotted version's numbers, which order versions.
fn numbers(v: &str) -> Vec<u64> {
    v.split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect()
}

fn older(a: &str, b: &str) -> bool {
    numbers(a) < numbers(b)
}

/// The plugins in `index`'s text, from `repo`, with the versions this host runs; one with none is left out.
fn offers(repo: &Repo, text: &[u8]) -> Result<Vec<Offer>, String> {
    let index = nori_plugin_index::parse(text)?;
    Ok(index
        .plugins
        .into_iter()
        .filter_map(|l| {
            let mut versions: Vec<Version> = l
                .versions
                .into_iter()
                .filter(|r| r.api == API)
                .map(|r| Version {
                    repo: repo.url.clone(),
                    url: resolved(&repo.url, &r.url),
                    version: r.version,
                    released: r.released,
                    sha256: r.sha256.to_ascii_lowercase(),
                    key: r.key.map(|k| k.to_ascii_lowercase()),
                    signature: r.signature.map(|s| s.to_ascii_lowercase()),
                })
                .collect();
            versions.sort_by_cached_key(|v| std::cmp::Reverse(numbers(&v.version)));
            let (readme, icon) = (l.readme.map(|r| resolved(&repo.url, &r)), l.icon.map(|i| resolved(&repo.url, &i)));
            (!versions.is_empty()).then(|| Offer { repo: repo.clone(), id: l.id, name: l.name, about: l.about, source: l.source, readme, icon, points: l.points, versions, installed: None, state: OfferState::New })
        })
        .collect())
}

impl Plugins {
    /// `version`'s package, fetched into the plugins' folder once its sha256 is the one listed.
    pub(crate) async fn fetch(&self, version: &Version) -> Result<std::path::PathBuf, Refusal> {
        let unreadable = |e: String| Refusal::Unreadable(e);
        let t = self.transport_now().map_err(unreadable)?;
        let r = t.get(version.url.clone(), 0).await.map_err(|e| unreadable(e.to_string()))?;
        if !(200..=299).contains(&r.status) {
            return Err(unreadable(format!("HTTP {}", r.status)));
        }
        let digest = Sha256::digest(&r.body);
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if hash != version.sha256 {
            return Err(unreadable("the package isn't the one its repo lists".into()));
        }
        match (&version.key, &version.signature) {
            (None, None) => {}
            (Some(key), Some(signature)) if signed(key, signature, &digest) => {}
            _ => return Err(Refusal::Signature),
        }
        let file = self.dir.join(format!(".{hash}.noriplugin"));
        std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&file, &r.body)).map_err(|e| unreadable(e.to_string()))?;
        Ok(file)
    }

    pub(crate) fn transport_now(&self) -> Result<std::sync::Arc<dyn nori_net::transport::Transport>, String> {
        self.transport.lock().expect("not poisoned").clone().ok_or_else(|| "no connection yet".to_string())
    }
}

impl Plugins {
    /// Sets where each of `offers` stands against what is installed now, for offers read before an
    /// install, update or removal.
    pub fn offers_stand(&self, offers: &mut [Offer]) {
        let installed = self.installed();
        for o in offers {
            let here = installed.iter().find(|i| i.manifest.id == o.id);
            o.installed = here.map(|i| i.manifest.version.clone());
            o.state = match here {
                None => OfferState::New,
                Some(i) if i.repo.as_ref() == Some(&o.repo.url) && o.versions.first().is_some_and(|v| older(&i.manifest.version, &v.version)) => OfferState::Update,
                Some(_) => OfferState::Installed,
            };
        }
    }
}

/// What `offers` hold of plugin `id`, with its place among them: from the repo `card` came from when it
/// is installed (one from a file comes from no repo), else from the first repo that lists it.
pub fn offer_of<'a>(offers: &'a [Offer], id: &str, card: Option<&PluginCard>) -> Option<(usize, &'a Offer)> {
    offers.iter().enumerate().find(|(_, o)| o.id == id && card.is_none_or(|c| c.repo.as_ref() == Some(&o.repo.url)))
}

/// Those of `offers` with an update for a plugin of `cards` that isn't kept at its version.
pub fn updates_waiting<'a>(offers: &'a [Offer], cards: &[PluginCard]) -> Vec<&'a Offer> {
    offers.iter().filter(|o| o.state == OfferState::Update && cards.iter().any(|c| c.id == o.id && !c.held)).collect()
}

/// [`offer_of`] without its place, for a client that keeps none.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn offer_for(offers: Vec<Offer>, id: String, card: Option<PluginCard>) -> Option<Offer> {
    offer_of(&offers, &id, card.as_ref()).map(|(_, o)| o.clone())
}

/// [`updates_waiting`], owned.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn offers_to_update(offers: Vec<Offer>, cards: Vec<PluginCard>) -> Vec<Offer> {
    updates_waiting(&offers, &cards).into_iter().cloned().collect()
}

/// The plugins that would add a kind of profile if they were on: of `offers`, those filling the provider
/// point and not on among `cards`, once each, by name. While `all_off`, those on too: none adds a kind then.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn provider_offers(mut offers: Vec<Offer>, cards: Vec<PluginCard>, all_off: bool) -> Vec<Offer> {
    offers.retain(|o| o.points.iter().any(|p| p == "provider") && (all_off || !cards.iter().any(|c| c.id == o.id && c.enabled)));
    offers.sort_by_cached_key(|o| o.name.to_lowercase());
    offers.dedup_by(|a, b| a.id == b.id);
    offers
}

/// Those of `offers` from `repo` (any, without one) whose name or about holds `query`, by name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn offers_found(offers: Vec<Offer>, query: String, repo: Option<String>) -> Vec<Offer> {
    let query = query.trim().to_lowercase();
    let holds = |text: &str| text.to_lowercase().contains(&query);
    let mut found: Vec<Offer> = offers.into_iter().filter(|o| repo.as_ref().is_none_or(|r| *r == o.repo.url) && (holds(&o.name) || o.about.as_deref().is_some_and(holds))).collect();
    found.sort_by_cached_key(|o| o.name.to_lowercase());
    found
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
        self.offers_stand(&mut out.offers);
        out
    }

    /// Fetches and installs `version` of a plugin a repo offers, newer or older than the one here (its
    /// store and its being on are kept); its id.
    pub async fn install_version(&self, version: Version) -> Result<String, Refusal> {
        let file = self.fetch(&version).await?;
        let installed = self.install_as(&file, Some(version.repo.clone()), version.key.clone()).map(|m| m.id.clone());
        let _ = std::fs::remove_file(&file);
        installed
    }

    /// Whether a day has passed since updates were last looked for, at `now_ms` (device ms), or a plugin
    /// built for another API waits for its update.
    pub fn updates_due(&self, now_ms: i64) -> bool {
        now_ms - self.checked_ms() >= UPDATE_EVERY_MS || now_ms < self.checked_ms() || self.installed().iter().any(|i| i.manifest.api != API)
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
            match self.install_version(v.clone()).await {
                Ok(_) => done.push(Updated { id: i.manifest.id.clone(), from: i.manifest.version.clone(), to: v.version.clone() }),
                Err(e) => nori_model::alog::info(&format!("plugin {}: not updated to {}: {e}", i.manifest.id, v.version)),
            }
        }
        done
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
                {"version": "0.9.0", "api": 2, "url": "a-0.9.0.noriplugin", "sha256": "AB"},
                {"version": "0.10.0", "api": 2, "url": "https://cdn.example/a-0.10.0.noriplugin", "sha256": "cd", "released": "2026-11-02"},
                {"version": "2.0.0", "api": 1, "url": "a-2.0.0.noriplugin", "sha256": "ef"}
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
