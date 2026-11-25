//! The nearby way a pairing code travels. While a new device's pairing screen shows its code, the device
//! announces the code on the local network, with a tag for each of its server accounts. The pairing
//! screen of a device with one of those accounts lists it, and the user picks it there. Nothing is
//! announced or looked for outside a pairing screen, so no device is reached whose user did not open
//! pairing. The three words are checked as for a pasted link. The platform's mDNS carries it
//! ([`Nearby`]).

use hkdf::Hkdf;
use nori_settings::settings::{SavedServer, ServerKind};
use sha2::Sha256;

/// The mDNS service type of a device waiting to be paired, as the platforms write it.
pub const SERVICE: &str = "_nori-pair._tcp";

/// The most account tags an announcement carries: each is 16 characters of one TXT entry.
const TAGS_MOST: usize = 12;
/// The most characters of the code a TXT entry holds; the code takes as many as it needs.
const CHUNK: usize = 200;

/// One TXT entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NearbyTxt {
    pub key: String,
    pub value: String,
}

/// What a device waiting to be paired announces: its mDNS instance name and TXT entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NearbyAnnouncement {
    pub name: String,
    pub txt: Vec<NearbyTxt>,
}

/// A waiting device the pairing screen lists: [`crate::Syncing::pair_take`] takes its `link`, came
/// [`crate::Came::Nearby`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NearbyDevice {
    /// The mDNS service it was found as.
    pub service: String,
    pub name: String,
    pub link: String,
}

/// The platform's mDNS for pairing. Calls come on any thread, never while sync holds its state.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait Nearby: Send + Sync {
    /// Announces `waiting` as a [`SERVICE`], in place of what was announced before; None withdraws it.
    fn announce(&self, waiting: Option<NearbyAnnouncement>);
    /// Looks for [`SERVICE`]s or stops; what it finds goes to [`crate::Syncing::nearby_found`] and
    /// [`crate::Syncing::nearby_lost`].
    fn browse(&self, on: bool);
}

/// A keyed hash of `user` under `secret`, short: devices without the secret neither match it nor learn
/// the name.
fn tag(user: &str, secret: &str) -> String {
    let (prk, _) = Hkdf::<Sha256>::extract(Some(secret.as_bytes()), format!("nori pair account\0{user}").as_bytes());
    prk.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The tags of the server accounts in `servers`: a server profile with a user and a password or key. A
/// jam guest's profile is no account.
pub(crate) fn account_tags(servers: &[SavedServer]) -> Vec<String> {
    let mut tags: Vec<String> = servers
        .iter()
        .filter(|s| s.kind == ServerKind::Subsonic && !s.user.is_empty() && !nori_remote::is_guest_key(&s.api_key))
        .filter_map(|s| {
            let secret = if s.api_key.is_empty() { &s.password } else { &s.api_key };
            (!secret.is_empty()).then(|| tag(&s.user, secret))
        })
        .collect();
    tags.sort();
    tags.dedup();
    tags.truncate(TAGS_MOST);
    tags
}

/// The announcement of code `link` for accounts `tags`; None without an account, as no device could
/// list it.
pub(crate) fn announcement(link: &str, tags: &[String]) -> Option<NearbyAnnouncement> {
    if tags.is_empty() {
        return None;
    }
    let mut txt = vec![NearbyTxt { key: "t".into(), value: tags.join(",") }];
    let chars: Vec<char> = link.chars().collect();
    txt.extend(chars.chunks(CHUNK).enumerate().map(|(i, c)| NearbyTxt { key: format!("c{i}"), value: c.iter().collect() }));
    // The instance's name, from its code: the same code announces under the same name.
    let name = format!("nori-pair-{}", &tag(link, "nori pair name")[..12]);
    Some(NearbyAnnouncement { name, txt })
}

/// The code an announcement carries when one of its accounts is among `ours`.
pub(crate) fn code(txt: &[NearbyTxt], ours: &[String]) -> Option<String> {
    let get = |k: &str| txt.iter().find(|t| t.key == k).map(|t| t.value.as_str());
    let shared = get("t")?.split(',').any(|t| ours.iter().any(|o| o == t));
    if !shared {
        return None;
    }
    let link: String = (0..).map_while(|i| get(&format!("c{i}"))).collect();
    (!link.is_empty()).then_some(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(user: &str, password: &str) -> SavedServer {
        SavedServer { user: user.into(), password: password.into(), url: "https://music.example".into(), ..Default::default() }
    }

    #[test]
    fn an_announcement_reaches_only_a_device_with_one_of_its_accounts() {
        let link = format!("nori://pair#v=1&{}", "x".repeat(450));
        let ann = announcement(&link, &account_tags(&[server("ann", "pw"), server("work", "pw2")])).unwrap();
        assert!(ann.txt.iter().all(|t| t.value.len() <= CHUNK), "each entry fits mDNS");
        assert_eq!(code(&ann.txt, &account_tags(&[server("ann", "pw")])), Some(link.clone()));
        assert_eq!(code(&ann.txt, &account_tags(&[server("ann", "old")])), None, "another password");
        assert_eq!(code(&ann.txt, &account_tags(&[server("bob", "pw")])), None, "another user");
        assert_eq!(announcement(&link, &[]), None, "nothing to announce without an account");
        let guest = SavedServer { api_key: "nori-jam-k1".into(), ..server("ann", "") };
        let plugin = SavedServer { kind: ServerKind::Plugin { id: "dev.example".into() }, ..server("ann", "pw") };
        assert!(account_tags(&[guest, plugin]).is_empty(), "a jam guest and a plugin have no server account");
        assert_eq!(announcement(&link, &account_tags(&[server("ann", "pw")])).map(|a| a.name), Some(ann.name), "one code, one name");
    }
}
