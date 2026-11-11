//! Devices paired with this one over the LAN. A profile without a server account pairs device by device:
//! one device shows its code (lan.rs's [`nori_remote::lan::PairCode`]), the other sends it back to that device's door,
//! and both keep a key of their own for the pair. An account's devices need none: they sign with the
//! account's proof. A browser opens no door, so it only signs with the keys kept; pairing is native.

use nori_remote::wire::DeviceKind;
use nori_settings::settings_store::Settings;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::db;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{pairing_name, pairing_valid, PairedDevice};
#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::door_access;

/// The `app_kv` key of the devices paired with this one.
const PAIRED: &str = "remotePaired";

/// A paired device as the app database keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Kept {
    id: String,
    name: String,
    kind: DeviceKind,
    key: String,
    seen_ms: i64,
}

fn kept(settings: &Settings) -> Vec<Kept> {
    settings.app_value(PAIRED).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// Changes the paired devices kept, read and written under the database's lock; None when it is not open.
fn edit_kept<R>(settings: &Settings, edit: impl FnOnce(&mut Vec<Kept>) -> R) -> Option<R> {
    let db = settings.app_db()?;
    let db = db.lock();
    let read: Option<String> = db.query_row("SELECT value FROM app_kv WHERE key=?1", [PAIRED], |r| r.get(0)).ok();
    let mut list: Vec<Kept> = read.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    let out = edit(&mut list);
    let json = serde_json::to_string(&list).ok()?;
    db.execute("INSERT OR REPLACE INTO app_kv(key,value) VALUES(?1,?2)", [PAIRED, &json]).ok()?;
    Some(out)
}

/// The key every device of the account (or a LAN jam) signs with; None where devices pair one by one.
pub(super) fn shared_key(client: &Client) -> Option<String> {
    if let Some((_, secret)) = client.core.lan_jam.read().as_ref() {
        return Some(secret.clone());
    }
    client.core.account.read().as_ref().map(|account| account.proof().to_string())
}

/// The key this device signs its requests to device `peer`'s door with.
pub(super) fn lan_key(client: &Client, peer: &str) -> Option<String> {
    shared_key(client).or_else(|| kept(&client.core.session.settings).into_iter().find(|k| k.id == peer).map(|k| k.key))
}

/// Whether device `id` is paired with this one.
pub(super) fn is_paired(client: &Client, id: &str) -> bool {
    kept(&client.core.session.settings).iter().any(|k| k.id == id)
}

/// Device `id` was found on this network now.
pub(super) fn seen(client: &Client, id: &str) {
    edit_kept(&client.core.session.settings, |list| list.iter_mut().filter(|k| k.id == id).for_each(|k| k.seen_ms = db::now_ms()));
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::Arc;

    use nori_remote::lan::{self, PairError};
    use nori_remote::wire::{DeviceKind, Member};
    use nori_settings::settings_store::Settings;

    use super::{edit_kept, kept, shared_key, Kept};
    use crate::remote::Remote;
    use crate::client::Client;
    use crate::db;
    use crate::transport::{Exchange, FailureKind, TransportError, TransportResponse};

    /// How long pairing waits for the other device's door.
    const PAIR_TIMEOUT_MS: u32 = 8_000;

    /// The `app_kv` key of the invite this device's code carries.
    const INVITE: &str = "remoteLanSecret";

    /// A device paired with this one, as a list of them shows it.
    #[derive(Debug, Clone, PartialEq)]
    #[cfg_attr(feature = "ffi", derive(uniffi::Record))]
    pub struct PairedDevice {
        pub id: String,
        pub name: String,
        pub kind: DeviceKind,
        /// Its door is on this network now (known while devices are looked for).
        pub nearby: bool,
        /// Whole days since it was last found on this network, or paired.
        pub seen_days: u32,
    }

    const DAY_MS: i64 = 86_400_000;

    impl Kept {
        fn shown(self, nearby: bool, now_ms: i64) -> PairedDevice {
            let seen_days = ((now_ms - self.seen_ms).max(0) / DAY_MS) as u32;
            PairedDevice { id: self.id, name: self.name, kind: self.kind, nearby, seen_days }
        }
    }

    /// The invite this device's code carries, made the first time it is asked for.
    fn invite(settings: &Settings) -> Option<String> {
        if let Some(invite) = settings.app_value(INVITE) {
            return Some(invite);
        }
        new_invite(settings)
    }

    /// A new invite in place of the last: a code is good for one device.
    fn new_invite(settings: &Settings) -> Option<String> {
        let invite = lan::new_secret().ok()?;
        let db = settings.app_db()?;
        db.lock().execute("INSERT OR REPLACE INTO app_kv(key,value) VALUES(?1,?2)", [INVITE, &invite]).ok()?;
        Some(invite)
    }

    /// What this device's door admits: an account's devices by its proof, a jam guest's host by its
    /// member key, else the devices paired with it.
    pub(in crate::remote) fn door_access(client: &Client) -> Option<lan::Access> {
        if let Some(secret) = shared_key(client) {
            return Some(lan::Access::Account(secret));
        }
        let settings = &client.core.session.settings;
        let invite = invite(settings)?;
        Some(lan::Access::Paired { invite, keys: kept(settings).into_iter().map(|k| (k.id, k.key)).collect() })
    }

    /// The other device as its door answered a pairing; why it did not pair otherwise.
    fn answered(got: Result<TransportResponse, TransportError>) -> Result<Member, PairError> {
        match got {
            Err(TransportError::Failed { kind: FailureKind::Timeout | FailureKind::Interrupted, .. }) => Err(PairError::TimedOut),
            Err(_) => Err(PairError::Unreachable),
            Ok(r) if r.status == 401 || r.status == 403 => Err(PairError::Refused),
            // Anything else answering there is not a nori door: the device is not where its code says.
            Ok(r) => serde_json::from_slice::<Member>(&r.body).ok().filter(|m| r.status == 200 && !m.id.is_empty()).ok_or(PairError::Unreachable),
        }
    }

    /// Whether `link` is a device pairing code, separate from a jam invite.
    #[cfg_attr(feature = "ffi", uniffi::export)]
    pub fn pairing_valid(link: String) -> bool {
        lan::parse_pairing(&link).is_some()
    }

    /// The name of the device whose pairing code `link` is, for asking whether to pair with it.
    #[cfg_attr(feature = "ffi", uniffi::export)]
    pub fn pairing_name(link: String) -> Option<String> {
        lan::parse_pairing(&link).map(|code| code.name).filter(|name| !name.is_empty())
    }

    #[cfg_attr(feature = "ffi", uniffi::export)]
    impl Remote {
        /// This device's pairing code, for another device to scan or paste: None where this profile does not
        /// pair (an account's, a jam guest's) or its door is closed (remote control off).
        pub fn pairing_link(&self) -> Option<String> {
            if !self.pairs() {
                return None;
            }
            let door = self.door_address()?;
            let invite = invite(&self.client.core.session.settings)?;
            Some(lan::pairing_link(&lan::PairCode { invite, id: self.id.clone(), name: self.me.name.clone(), door }))
        }

        /// The devices paired with this one, the nearby ones first, then by when each was last seen.
        pub fn paired(&self) -> Vec<PairedDevice> {
            let near: Vec<String> = self.inner.lock().peers.iter().filter(|p| !p.lost).map(|p| p.member.id.clone()).collect();
            let mut list = kept(&self.client.core.session.settings);
            list.sort_by_key(|k| (!near.contains(&k.id), -k.seen_ms));
            let now = db::now_ms();
            list.into_iter().map(|k| { let nearby = near.contains(&k.id); k.shown(nearby, now) }).collect()
        }

        /// Pairs with the device whose code `link` is: sends the code back to that device's door, which
        /// answers with itself. Both then list the other and control each other on this network.
        pub async fn pair(self: Arc<Self>, link: String) -> Result<PairedDevice, PairError> {
            let code = lan::parse_pairing(&link).ok_or(PairError::NotACode)?;
            if !self.pairs() {
                return Err(PairError::Unavailable);
            }
            if code.id == self.id {
                return Err(PairError::Own);
            }
            let nonce = self.nonce().ok_or(PairError::Unavailable)?;
            let me = serde_json::to_string(&Member { id: self.id.clone(), name: self.me.name.clone(), kind: self.me.kind, state: None }).unwrap_or_default();
            let target = lan::signed(&code.invite, "POST", &format!("/rest/noriRemote.pair?nonce={nonce}"), me.as_bytes(), db::now_ms());
            let url = format!("http://{}{target}", code.door);
            let other = answered(self.client.transport.send(Exchange { url, json: Some(me), timeout_ms: PAIR_TIMEOUT_MS, ..Default::default() }).await)?;
            let now = db::now_ms();
            let device = Kept { id: other.id, name: other.name, kind: other.kind, key: lan::pair_key(&code.invite, &nonce), seen_ms: now };
            self.keep_pair(device.clone());
            Ok(device.shown(false, now))
        }

        /// Forgets paired device `id`: it no longer controls this one, nor this one it. It is told so when
        /// it is nearby, and forgets this one too.
        pub fn forget(self: Arc<Self>, id: String) {
            let settings = &self.client.core.session.settings;
            let Some(Some(gone)) = edit_kept(settings, |list| list.iter().position(|k| k.id == id).map(|at| list.remove(at))) else { return };
            let base = {
                let mut i = self.inner.lock();
                let base = i.peers.iter().find(|p| p.member.id == id && !p.lost).map(|p| p.base());
                i.peers.retain(|p| p.member.id != id);
                if i.active.mirror().is_some_and(|m| m.id == id) {
                    i.active.here();
                }
                base
            };
            self.reopen_access();
            if let Some(base) = base {
                let url = format!("{base}{}", lan::signed(&gone.key, "POST", &format!("/rest/noriRemote.unpair?dev={}", self.id), b"", db::now_ms()));
                let client = self.client.clone();
                self.spawn(async move {
                    let _ = client.transport.send(Exchange { url, json: Some(String::new()), timeout_ms: PAIR_TIMEOUT_MS, ..Default::default() }).await;
                });
            }
            self.follow_active();
            self.shown.changed();
        }
    }

    impl Remote {
        /// Whether this profile pairs device by device: one without a server account, not a jam guest's.
        fn pairs(&self) -> bool {
            self.me.kind != DeviceKind::Guest && shared_key(&self.client).is_none() && self.client.core.serverless()
        }

        /// Keeps a device paired now, shows it, and looks for doors again: the paired device's door, seen
        /// before it was paired, was passed over.
        fn keep_pair(self: &Arc<Self>, device: Kept) {
            let settings = &self.client.core.session.settings;
            edit_kept(settings, |list| {
                list.retain(|k| k.id != device.id);
                list.push(device);
            });
            self.reopen_access();
            self.look_again();
            self.shown.changed();
        }

        /// A pairing made or ended at this device's door.
        pub(in crate::remote) fn door_paired(self: &Arc<Self>, change: lan::PairChange) {
            match change {
                lan::PairChange::Paired { id, name, kind, key } => {
                    // The code shown is used: the next device pairs with a new one.
                    new_invite(&self.client.core.session.settings);
                    self.keep_pair(Kept { id, name, kind, key, seen_ms: db::now_ms() });
                }
                lan::PairChange::Forgot { id } => {
                    edit_kept(&self.client.core.session.settings, |list| list.retain(|k| k.id != id));
                    self.inner.lock().peers.retain(|p| p.member.id != id);
                    self.reopen_access();
                    self.follow_active();
                    self.shown.changed();
                }
            }
        }

        /// The door admits what the paired list says now.
        fn reopen_access(&self) {
                if let (Some(door), Some(access)) = (&self.inner.lock().door, door_access(&self.client)) {
                door.set_access(access);
            }
        }

        /// Where this device's door is, as `host:port` on its network; None while it is closed.
        fn door_address(&self) -> Option<String> {
            let port = self.inner.lock().door.as_ref()?.port();
            Some(match crate::remote::lan_ip()? {
                std::net::IpAddr::V6(ip) => format!("[{ip}]:{port}"),
                ip => format!("{ip}:{port}"),
            })
        }

        fn nonce(&self) -> Option<String> {
            lan::new_secret().ok()
        }

        /// Reports every door again: those found before a pairing were passed over.
        fn look_again(&self) {
            let browsing = self.inner.lock().browsing();
            if let (Some(discovery), true) = (&self.discovery, browsing) {
                discovery.browse(false);
                discovery.browse(true);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn last_seen_counts_whole_days() {
            let kept = |seen_ms| Kept { id: "d".into(), name: "D".into(), kind: DeviceKind::Phone, key: String::new(), seen_ms };
            let now = 10 * DAY_MS;
            for (seen, days) in [(now, 0), (now - DAY_MS + 1, 0), (now - DAY_MS, 1), (now - 3 * DAY_MS - 5, 3), (now + 5, 0)] {
                assert_eq!(kept(seen).shown(false, now).seen_days, days, "seen at {seen}");
            }
        }

        #[test]
        fn a_door_answer_says_why_pairing_failed() {
            let failed = |kind| Err(TransportError::Failed { kind, detail: None });
            let answer = |status, body: &str| Ok(TransportResponse { status, body: body.as_bytes().to_vec(), headers: Vec::new() });
            let desk = r#"{"id":"desk","name":"Desk","kind":"desktop"}"#;
            let cases = [
                (answer(200, desk), Ok(Member { id: "desk".into(), name: "Desk".into(), kind: DeviceKind::Desktop, state: None })),
                (failed(FailureKind::Connect), Err(PairError::Unreachable)),
                (failed(FailureKind::NoRoute), Err(PairError::Unreachable)),
                (failed(FailureKind::UnknownHost), Err(PairError::Unreachable)),
                (failed(FailureKind::Cleartext), Err(PairError::Unreachable)),
                (failed(FailureKind::Timeout), Err(PairError::TimedOut)),
                (failed(FailureKind::Interrupted), Err(PairError::TimedOut)),
                (answer(401, "{}"), Err(PairError::Refused)),
                (answer(403, "{}"), Err(PairError::Refused)),
                (answer(404, "{}"), Err(PairError::Unreachable)),
                (answer(200, "<html>"), Err(PairError::Unreachable)),
                (answer(200, "{}"), Err(PairError::Unreachable)),
                (answer(500, desk), Err(PairError::Unreachable)),
            ];
            for (got, want) in cases {
                let said = format!("{got:?}");
                assert_eq!(answered(got), want, "{said}");
            }
        }
    }
}
