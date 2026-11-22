//! Plugin repos: static `index.json` files anyone can host. nori's own is always there; the user adds
//! others by a GitHub `user/repo` or a URL (a client warns first: nori vouches only for its own), and
//! may name them. An index may name its repo, and lists each plugin's
//! id, name, about and source, and every version it published: its plugin API version, the day it was
//! released, its package's URL and the package's sha256, which an install checks before anything else,
//! and its author's key with the key's ed25519 signature over that sha256 (its 32 bytes). Old versions
//! stay listed, so a plugin can be taken back to one.

use std::collections::BTreeMap;

use nori_plugin_index::package::{checked, Listed, PackageRefusal};
use nori_plugin_index::{address, older, resolved, Index};
use serde::{Deserialize, Serialize};

use crate::{PluginCard, Plugins, Refusal, WhenNew, API};

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
    /// What it is called: the name it was given (the user's, else its index's), else its address's
    /// short form. None for nori's own, which each client names.
    pub name: Option<String>,
}

/// The repos the user added, as kept: their addresses in order, and the names given them by address
/// (apart, so an older nori still reads the addresses).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedRepos {
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repo_names: BTreeMap<String, String>,
}

impl AddedRepos {
    fn add(&mut self, url: &str) {
        if url != OFFICIAL && !self.repos.iter().any(|u| u == url) {
            self.repos.push(url.to_string());
        }
    }

    fn remove(&mut self, url: &str) {
        self.repos.retain(|u| u != url);
        self.repo_names.remove(url);
    }

    /// Names repo `url` `name`; an empty name takes its name away.
    fn rename(&mut self, url: &str, name: &str) {
        match name.trim() {
            _ if !self.repos.iter().any(|u| u == url) => {}
            "" => _ = self.repo_names.remove(url),
            name => _ = self.repo_names.insert(url.to_string(), name.to_string()),
        }
    }

    /// Names repo `url` `name` (its index's) if it has no name yet.
    fn name_once(&mut self, url: &str, name: &str) {
        if !self.repo_names.contains_key(url) {
            self.rename(url, name);
        }
    }

    /// Those of `kept` not here yet, with the names `kept` gives them.
    pub(crate) fn take_in(&mut self, kept: AddedRepos) {
        for url in &kept.repos {
            self.add(url);
        }
        for (url, name) in kept.repo_names {
            self.rename(&url, &name);
        }
    }

    fn listed(&self) -> Vec<Repo> {
        let added = self.repos.iter().map(|url| Repo { url: url.clone(), official: false, name: Some(self.repo_names.get(url).cloned().unwrap_or_else(|| address::short_name(url))) });
        std::iter::once(Repo { url: OFFICIAL.into(), official: true, name: None }).chain(added).collect()
    }
}

/// Why a repo couldn't be read or added, from the least telling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RepoProblem {
    /// No answer: no connection, or no such host.
    Unreachable,
    /// The server has no index there, or no such address could be one.
    NotFound,
    /// What is there isn't a repo's index.
    NotARepo,
}

/// A repo added, or why not.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RepoAdd {
    /// Added now, or there already.
    Added { repo: Repo },
    Failed { problem: RepoProblem },
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
    /// Other when its repo doesn't say.
    pub category: crate::Category,
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
    pub problem: RepoProblem,
}

/// What the repos offer, and those that couldn't be read.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Offers {
    pub offers: Vec<Offer>,
    pub failed: Vec<RepoFailure>,
}

/// The plugins in `index`, from `repo`, with the versions this host runs; one with none is left out.
fn offers(repo: &Repo, index: Index) -> Vec<Offer> {
    index
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
            versions.sort_by_cached_key(|v| std::cmp::Reverse(nori_plugin_index::version_numbers(&v.version)));
            let (readme, icon) = (l.readme.map(|r| resolved(&repo.url, &r)), l.icon.map(|i| resolved(&repo.url, &i)));
            (!versions.is_empty()).then(|| Offer { repo: repo.clone(), id: l.id, name: l.name, about: l.about, source: l.source, readme, icon, points: l.points, category: crate::Category::named(l.category.as_deref()), versions, installed: None, state: OfferState::New })
        })
        .collect()
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
        let listed = Listed { sha256: version.sha256.clone(), key: version.key.clone(), signature: version.signature.clone() };
        let hash = checked(&r.body, &listed).map_err(|e| match e {
            PackageRefusal::NotListed => unreadable("the package isn't the one its repo lists".into()),
            PackageRefusal::Signature => Refusal::Signature,
        })?;
        let file = self.dir.join(format!(".{hash}.noriplugin"));
        std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&file, &r.body)).map_err(|e| unreadable(e.to_string()))?;
        Ok(file)
    }

    /// The index at `url`.
    async fn index_at(&self, url: &str) -> Result<Index, RepoProblem> {
        let failed = |problem, why: &dyn std::fmt::Display| {
            nori_model::alog::info(&format!("plugin repo {url}: {why}"));
            problem
        };
        let t = self.transport_now().map_err(|e| failed(RepoProblem::Unreachable, &e))?;
        let r = t.get(url.to_string(), 0).await.map_err(|e| failed(RepoProblem::Unreachable, &e))?;
        match r.status {
            200..=299 => nori_plugin_index::parse(&r.body).map_err(|e| failed(RepoProblem::NotARepo, &e)),
            s @ 500.. => Err(failed(RepoProblem::Unreachable, &format!("HTTP {s}"))),
            s => Err(failed(RepoProblem::NotFound, &format!("HTTP {s}"))),
        }
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

/// Plugin `id`'s name: installed, as `cards` show it, else as `offers` have it, else its id.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn plugin_display_name(id: String, cards: Vec<PluginCard>, offers: Vec<Offer>) -> String {
    cards.into_iter().find(|c| c.id == id).map(|c| c.name).or_else(|| offers.into_iter().find(|o| o.id == id).map(|o| o.name)).unwrap_or(id)
}

/// What the repo at `url` is called among `repos`, else by its address's short form; none for nori's own.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn repo_called(url: String, repos: Vec<Repo>) -> Option<String> {
    match repos.into_iter().find(|r| r.url == url) {
        Some(r) => r.name,
        None => (url != OFFICIAL).then(|| address::short_name(&url)),
    }
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
    let shown = offers.into_iter().filter(|o| repo.as_ref().is_none_or(|r| *r == o.repo.url)).collect();
    nori_plugin_index::found(shown, &query, |o: &Offer| &o.name, |o| o.about.as_deref())
}

impl Plugins {
    pub(crate) async fn fetch_and_install(&self, version: Version, when_new: WhenNew) -> Result<String, Refusal> {
        let file = self.fetch(&version).await?;
        let installed = self.install_as(&file, Some(version.repo.clone()), version.key.clone(), when_new).map(|m| m.id.clone());
        let _ = std::fs::remove_file(&file);
        installed
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// nori's repo, then those the user added.
    pub fn repos(&self) -> Vec<Repo> {
        self.added_repos().listed()
    }

    /// Adds the repo `typed` names (a GitHub `user/repo`, or an index's URL): the first place it may be
    /// whose index reads; a client warns before. One added before is found as it is.
    pub async fn add_repo(&self, typed: String) -> RepoAdd {
        let mut problem = None;
        for url in address::candidates(&typed) {
            if let Some(repo) = self.repos().into_iter().find(|r| r.url == url) {
                return RepoAdd::Added { repo };
            }
            match self.index_at(&url).await {
                Ok(index) => {
                    self.change_repos(|r| {
                        r.add(&url);
                        index.name.iter().for_each(|n| r.name_once(&url, n));
                    });
                    let repo = self.repos().into_iter().find(|r| r.url == url).expect("just added");
                    return RepoAdd::Added { repo };
                }
                Err(p) => problem = problem.max(Some(p)),
            }
        }
        RepoAdd::Failed { problem: problem.unwrap_or(RepoProblem::NotFound) }
    }

    pub fn remove_repo(&self, url: String) {
        self.change_repos(|r| r.remove(&url));
    }

    /// Names repo `url` `name`, which an empty name takes away; nori's own keeps its name.
    pub fn rename_repo(&self, url: String, name: String) {
        self.change_repos(|r| r.rename(&url, &name));
    }

    /// What every repo offers that this host can run, each with the version installed here. A repo
    /// with no name yet takes its index's.
    pub async fn offers(&self) -> Offers {
        let mut read = Vec::new();
        for repo in self.repos() {
            read.push((repo.url.clone(), self.index_at(&repo.url).await));
        }
        self.change_repos(|r| {
            for (url, index) in &read {
                if let Ok(Index { name: Some(name), .. }) = index {
                    r.name_once(url, name);
                }
            }
        });
        let repos = self.repos();
        let mut out = Offers { offers: Vec::new(), failed: Vec::new() };
        for (url, index) in read {
            let Some(repo) = repos.iter().find(|r| r.url == url) else { continue };
            match index {
                Ok(index) => out.offers.extend(offers(repo, index)),
                Err(problem) => out.failed.push(RepoFailure { repo: repo.clone(), problem }),
            }
        }
        self.offers_stand(&mut out.offers);
        out
    }

    /// Fetches and installs `version` of a plugin a repo offers, newer or older than the one here (its
    /// store and its being on are kept), on when new; its id.
    pub async fn install_version(&self, version: Version) -> Result<String, Refusal> {
        self.fetch_and_install(version, WhenNew::On).await
    }

    /// Whether a day has passed since updates were last looked for, at `now_ms` (device ms), or a plugin
    /// built for another API waits for its update.
    pub fn updates_due(&self, now_ms: i64) -> bool {
        now_ms - self.checked_ms() >= UPDATE_EVERY_MS || now_ms < self.checked_ms() || self.installed().iter().any(|i| !i.runs_here())
    }

    /// Updates each plugin not held to the newest version its own repo offers; what was updated. One
    /// built for another API, which can't run here, takes the newest this host runs, held or not.
    pub async fn update(&self, now_ms: i64) -> Vec<Updated> {
        let offers = self.offers().await;
        self.keep_checked(now_ms);
        let mut done = Vec::new();
        for i in self.installed().into_iter().filter(|i| !i.held || !i.runs_here()) {
            let Some(repo) = &i.repo else { continue };
            let newest = offers.offers.iter().find(|o| o.id == i.manifest.id && o.repo.url == *repo).and_then(|o| o.versions.first());
            let Some(v) = newest.filter(|v| !i.runs_here() || older(&i.manifest.version, &v.version)) else { continue };
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
        let repo = Repo { url: "https://example.com/nori/index.json".into(), official: false, name: None };
        let text = br#"{"plugins": [
            {"id": "a.b", "name": "A", "source": "https://git.example/a", "versions": [
                {"version": "0.9.0", "api": 7, "url": "a-0.9.0.noriplugin", "sha256": "AB"},
                {"version": "0.10.0", "api": 7, "url": "https://cdn.example/a-0.10.0.noriplugin", "sha256": "cd", "released": "2026-11-02"},
                {"version": "2.0.0", "api": 6, "url": "a-2.0.0.noriplugin", "sha256": "ef"}
            ]},
            {"id": "c.d", "name": "C", "versions": [{"version": "1.0.0", "api": 9, "url": "c.noriplugin", "sha256": "00"}]}
        ]}"#;
        let o = offers(&repo, nori_plugin_index::parse(text).unwrap());
        assert_eq!(o.len(), 1, "a plugin with no version this host runs isn't offered");
        let v: Vec<(&str, &str, &str)> = o[0].versions.iter().map(|v| (v.version.as_str(), v.url.as_str(), v.sha256.as_str())).collect();
        assert_eq!(v, vec![("0.10.0", "https://cdn.example/a-0.10.0.noriplugin", "cd"), ("0.9.0", "https://example.com/nori/a-0.9.0.noriplugin", "ab")], "newest first, as numbers; another API's left out");
        assert_eq!(o[0].source.as_deref(), Some("https://git.example/a"));
    }
}
