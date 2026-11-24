//! A profile as it travels between a group's devices: what goes, what stays on each device, and whose
//! sign-in goes where.

use nori_settings::settings::{server_from, server_json, SavedServer, ServerKind, SERVER_KEYS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Whether a provider's sign-in may go to another device. A plugin that says nothing is
/// [`SignInCopy::EachDevice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInCopy {
    /// The same sign-in works on every device (a server's user and password).
    Copied,
    /// Each device signs in itself, to the same account (a session the service keeps replacing).
    #[default]
    EachDevice,
}

/// The profiles a device can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Plays {
    /// A server's only (a browser, the iPod).
    Servers,
    /// A server's and a plugin's.
    ServersAndPlugins,
}

impl Plays {
    pub fn plays(self, kind: &ServerKind) -> bool {
        matches!((self, kind), (_, ServerKind::Subsonic) | (Plays::ServersAndPlugins, ServerKind::Plugin { .. }))
    }
}

/// A plugin a profile needs, as a device that has it installed knows it: a device without it installs
/// it from `repo`, only a version signed by `key` once one was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NeededPlugin {
    pub id: String,
    /// None when it was installed from a file, which no other device can fetch.
    pub repo: Option<String>,
    pub key: Option<String>,
}

/// What this device brings to profile sync; the client builds it from its providers and plugins.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Here {
    pub plays: Plays,
    /// The kinds (`ServerKind::id`) whose sign-in is [`SignInCopy::Copied`] by their provider's word.
    pub copied: Vec<String>,
    /// The plugins installed here.
    pub plugins: Vec<NeededPlugin>,
}

impl Here {
    /// Whether this device sends `kind`'s sign-in: it plays the kind and its provider lets it go.
    fn sends_sign_in(&self, kind: &ServerKind) -> bool {
        self.plays.plays(kind) && self.copied.iter().any(|k| k == kind.id())
    }

    /// The plugin `kind` needs, as installed here.
    pub(crate) fn plugin(&self, kind: &ServerKind) -> Option<NeededPlugin> {
        let ServerKind::Plugin { id } = kind else { return None };
        self.plugins.iter().find(|p| p.id == *id).cloned()
    }
}

/// Where a change stands in a profile's order: a counter first, which every device moves past the
/// highest it has seen, then the wall clock and the device, only to settle a tie.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub(crate) struct Stamp {
    pub n: u64,
    pub ms: i64,
    pub by: String,
}

/// What stays on each device: its own id for the profile, its client certificate (a file there), and
/// Sync this profile.
const LOCAL: [&str; 4] = ["id", "clientCert", "clientCertPassword", "syncOff"];
/// The sign-in, sent only where it may go.
const SIGN_IN: [&str; 2] = ["password", "apiKey"];

/// One profile as it travels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Shared {
    /// Its sync identity, the same on every device and apart from each device's own id for it.
    pub id: String,
    pub stamp: Stamp,
    /// The profile as the list stores it, without [`LOCAL`]; with its sign-in only where it may go. A
    /// newer nori's keys stay in it unchanged.
    pub profile: Map<String, Value>,
    /// The account a sign-in that is not copied must be to: the provider's stable identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<NeededPlugin>,
    /// What a newer nori says about the profile beside these, kept unchanged.
    #[serde(flatten)]
    pub newer: Map<String, Value>,
}

/// `server` as this device sends it, keeping what a newer nori put in `before`.
pub(crate) fn body(server: &SavedServer, here: &Here, before: Option<&Map<String, Value>>) -> Map<String, Value> {
    let Value::Object(mut map) = server_json(server) else { unreachable!("a server is an object") };
    for key in LOCAL {
        map.remove(key);
    }
    if !here.sends_sign_in(&server.kind) {
        for key in SIGN_IN {
            map.remove(key);
        }
    }
    for (key, value) in before.into_iter().flatten().filter(|(k, _)| !SERVER_KEYS.contains(&k.as_str())) {
        map.insert(key.clone(), value.clone());
    }
    map
}

impl Shared {
    /// What `kind` this profile is.
    pub(crate) fn kind(&self) -> ServerKind {
        self.profile.get("kind").and_then(Value::as_str).and_then(ServerKind::from_id).unwrap_or_default()
    }

    /// As sent to a device that plays `plays`: without a sign-in it could not use.
    pub(crate) fn fitted(mut self, plays: Plays) -> Shared {
        if !plays.plays(&self.kind()) {
            for key in SIGN_IN {
                self.profile.remove(key);
            }
        }
        self
    }

    /// The profile it makes on this device: `local`'s own parts (its id, certificate, Sync this profile,
    /// and its sign-in when none came) under this, or a new profile as `new_id`.
    pub(crate) fn server(&self, local: Option<&SavedServer>, new_id: &str) -> Option<SavedServer> {
        let mut map = self.profile.clone();
        let mine = local.map(server_json);
        let mine = mine.as_ref().and_then(Value::as_object);
        let mut keep = |key: &str| {
            if let Some(v) = mine.and_then(|m| m.get(key)) {
                map.insert(key.to_string(), v.clone());
            }
        };
        LOCAL.into_iter().for_each(&mut keep);
        if !SIGN_IN.iter().any(|k| self.profile.contains_key(*k)) {
            SIGN_IN.into_iter().for_each(&mut keep);
        }
        map.entry("id").or_insert_with(|| new_id.into());
        server_from(&Value::Object(map))
    }
}

/// How a sign-in on this device stands against the account the profile is synced with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInCheck {
    Accepted,
    /// Another account than the one the profile's other devices are signed in to.
    OtherAccount,
}

/// Whether a sign-in to `signed_in` is accepted for a profile synced with `synced`: only the same
/// account, compared when both are known.
pub(crate) fn sign_in_check(synced: Option<&str>, signed_in: Option<&str>) -> SignInCheck {
    match (synced, signed_in) {
        (Some(a), Some(b)) if a != b => SignInCheck::OtherAccount,
        _ => SignInCheck::Accepted,
    }
}

/// Whether a profile is a jam guest's, which lasts only the jam and never syncs.
pub(crate) fn guest(server: &SavedServer) -> bool {
    nori_remote::is_guest_key(&server.api_key) || nori_remote::lan::guest_secret(&server.api_key).is_some()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const PLUGIN: &str = "fm.nori.youtube-music";

    pub(crate) fn phone() -> Here {
        Here { plays: Plays::ServersAndPlugins, copied: vec!["subsonic".into()], plugins: vec![NeededPlugin { id: PLUGIN.into(), repo: Some("https://repo.example".into()), key: Some("k".into()) }] }
    }

    pub(crate) fn browser() -> Here {
        Here { plays: Plays::Servers, copied: vec!["subsonic".into()], plugins: vec![] }
    }

    pub(crate) fn navidrome(id: &str, url: &str) -> SavedServer {
        SavedServer { id: id.into(), name: "Home".into(), url: url.into(), user: "me".into(), password: "pw".into(), client_cert: "c.p12".into(), ..Default::default() }
    }

    pub(crate) fn youtube(id: &str) -> SavedServer {
        SavedServer { id: id.into(), kind: ServerKind::Plugin { id: PLUGIN.into() }, name: "YT".into(), api_key: "cookie".into(), ..Default::default() }
    }

    fn shared(server: &SavedServer, here: &Here) -> Shared {
        Shared { id: "s".into(), stamp: Stamp::default(), profile: body(server, here, None), account: None, plugin: here.plugin(&server.kind), newer: Map::new() }
    }

    #[test]
    fn what_travels_leaves_the_device_s_own_parts_and_a_sign_in_that_may_not_go() {
        let nav = shared(&navidrome("a1", "https://music.example"), &phone());
        assert!(!["id", "clientCert", "clientCertPassword", "syncOff"].iter().any(|k| nav.profile.contains_key(*k)));
        assert_eq!(nav.profile["password"], "pw", "a server's sign-in is copied");
        let yt = shared(&youtube("b2"), &phone());
        assert!(!yt.profile.contains_key("apiKey"), "a plugin that says nothing is signed in on each device");
        assert_eq!(yt.plugin.as_ref().map(|p| p.repo.as_deref()), Some(Some("https://repo.example")));
        let copied = Here { copied: vec!["subsonic".into(), PLUGIN.into()], ..phone() };
        assert_eq!(shared(&youtube("b2"), &copied).profile["apiKey"], "cookie", "a plugin that lets it go");
    }

    #[test]
    fn a_device_that_cannot_play_a_profile_is_not_sent_its_sign_in() {
        let copied = Here { copied: vec!["subsonic".into(), PLUGIN.into()], ..phone() };
        let yt = shared(&youtube("b2"), &copied);
        assert!(!yt.clone().fitted(Plays::Servers).profile.contains_key("apiKey"));
        assert!(yt.fitted(Plays::ServersAndPlugins).profile.contains_key("apiKey"));
        let nav = shared(&navidrome("a1", "https://m"), &phone());
        assert!(nav.fitted(Plays::Servers).profile.contains_key("password"), "a browser plays a server");
        assert!(!Plays::Servers.plays(&ServerKind::Plugin { id: PLUGIN.into() }));
    }

    #[test]
    fn a_profile_arriving_keeps_this_device_s_own_parts() {
        let here = navidrome("local", "https://old.example");
        let mut theirs = shared(&SavedServer { name: "Renamed".into(), password: "new".into(), ..navidrome("x", "https://music.example") }, &phone());
        let made = theirs.server(Some(&here), "unused").unwrap();
        assert_eq!((made.id.as_str(), made.client_cert.as_str(), made.name.as_str(), made.password.as_str()), ("local", "c.p12", "Renamed", "new"));
        theirs.profile.remove("password");
        theirs.profile.remove("apiKey");
        assert_eq!(theirs.server(Some(&here), "unused").unwrap().password, "pw", "no sign-in came: this device's own stays");
        let new = theirs.server(None, "n3").unwrap();
        assert_eq!((new.id.as_str(), new.client_cert.as_str(), new.password.as_str()), ("n3", "", ""));
    }

    #[test]
    fn a_newer_nori_s_keys_stay_through_an_edit_here() {
        let mut before = body(&navidrome("a", "https://m"), &phone(), None);
        before.insert("lyricsAccount".into(), "x".into());
        let after = body(&SavedServer { name: "New".into(), ..navidrome("a", "https://m") }, &phone(), Some(&before));
        assert_eq!((after["lyricsAccount"].as_str(), after["name"].as_str()), (Some("x"), Some("New")));
        let read: Shared = serde_json::from_value(serde_json::json!({"id": "s", "stamp": {"n": 1, "ms": 0, "by": "d"}, "profile": {}, "mood": [1]})).unwrap();
        assert_eq!(serde_json::to_value(&read).unwrap()["mood"], serde_json::json!([1]), "beside the profile too");
    }

    #[test]
    fn only_the_same_account_signs_in() {
        assert_eq!(sign_in_check(Some("UC1"), Some("UC1")), SignInCheck::Accepted);
        assert_eq!(sign_in_check(Some("UC1"), Some("UC2")), SignInCheck::OtherAccount);
        assert_eq!(sign_in_check(None, Some("UC2")), SignInCheck::Accepted, "the first sign-in names the account");
        assert_eq!(sign_in_check(Some("UC1"), None), SignInCheck::Accepted, "a provider that names no account cannot be checked");
    }
}
