//! The group as a device keeps it: the key, the server, this device's name, the devices it has seen and
//! the kept copy's choice. It lives in the app database beside the saved profiles, whose sign-ins it
//! guards as well as they are guarded (the app's own storage on each platform), and never in a backup.

use nori_settings::settings_store::Settings;
use serde::{Deserialize, Serialize};

use crate::book::Book;
use crate::keys::{b64, random, GroupKey};
use crate::profile::Stamp;

/// Where the group is kept: the app database's `app_kv`, which no backup holds.
const KEPT: &str = "syncGroup";

/// A device of the group as this one last saw it. A device names itself, so the list informs and does
/// not prove.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Seen {
    /// Its id in the group.
    pub device: String,
    pub name: String,
    /// Unix ms.
    pub last_ms: i64,
    /// In the group's room now.
    #[serde(skip)]
    pub online: bool,
}

/// Puts `devices` in the order a list shows them: those in the group's room first, then the most recently
/// seen.
pub(crate) fn in_order(devices: &mut [Seen]) {
    devices.sort_by(|a, b| b.online.cmp(&a.online).then(b.last_ms.cmp(&a.last_ms)));
}

/// The group's choice to keep a copy on the server; it syncs, the newer choice winning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct KeepCopy {
    pub on: bool,
    pub stamp: Stamp,
}

mod key_text {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::keys::GroupKey;

    pub fn serialize<S: Serializer>(key: &GroupKey, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&key.text())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<GroupKey, D::Error> {
        GroupKey::from_text(&String::deserialize(d)?).ok_or_else(|| serde::de::Error::custom("not a group key"))
    }
}

/// The group and this device's place in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Group {
    #[serde(with = "key_text")]
    pub key: GroupKey,
    /// The sync server every device of the group uses.
    pub server: String,
    /// This device's id in the group, and its name.
    pub device: String,
    pub name: String,
    pub seen: Vec<Seen>,
    pub keep: KeepCopy,
}

impl Group {
    /// `device` was seen now as `name`.
    pub(crate) fn saw(&mut self, device: &str, name: &str, now_ms: i64) {
        match self.seen.iter_mut().find(|s| s.device == device) {
            Some(s) => (s.name, s.last_ms) = (name.to_string(), now_ms),
            None => self.seen.push(Seen { device: device.to_string(), name: name.to_string(), last_ms: now_ms, online: false }),
        }
    }
}

pub(crate) fn new_device_id() -> String {
    b64(&random::<16>())
}

/// The group and the profile sync, kept together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Kept {
    pub group: Group,
    pub book: Book,
}

impl Kept {
    /// The group kept in `settings`' database; None on a device that never paired.
    pub(crate) fn load(settings: &Settings) -> Option<Kept> {
        serde_json::from_str(&settings.app_value(KEPT)?).ok()
    }

    pub(crate) fn save(&self, settings: &Settings) {
        settings.keep_app_value(KEPT, serde_json::to_string(self).expect("the group serializes"));
    }
}

/// Whether this device belongs to a group.
pub fn paired(settings: &Settings) -> bool {
    settings.app_value(KEPT).is_some()
}

/// What Reset pairing does, for its confirmation and the screen after it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ResetNotice {
    /// The other devices that keep their data and stop syncing with this one.
    pub others: u32,
    /// This device's profiles whose sign-ins the other devices hold: change those passwords if one was
    /// lost.
    pub signed_in: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_in_the_room_come_first_then_the_most_recently_seen() {
        let seen = |device: &str, last_ms: i64, online: bool| Seen { device: device.into(), name: device.into(), last_ms, online };
        let mut devices = vec![seen("iPad", 1_000, false), seen("MacBook", 5_000, false), seen("PC", 2_000, true)];
        in_order(&mut devices);
        assert_eq!(devices.iter().map(|d| d.device.as_str()).collect::<Vec<_>>(), ["PC", "MacBook", "iPad"]);
    }
}
