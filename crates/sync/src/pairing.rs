//! Pairing a new device into a group. The new device shows a code: the server's address, a meeting
//! room, its one-time lock and its name, nothing secret, so a photographed or forwarded code is
//! useless. A device of the group takes the code, says what the user confirms, and seals the group's
//! key to the lock in the meeting room. The code dies after five minutes, by the server's clock.

use std::sync::Arc;
use std::time::Duration;

use nori_settings::settings::SavedServer;
use nori_settings::settings_store::Settings;
use serde::{Deserialize, Serialize};

use crate::book::Book;
use crate::group::{new_device_id, Group, KeepCopy, Kept};
use crate::keys::{b64, unb64, GroupKey, Lock, LockKey, RoomName};
use crate::profile::{guest, Plays};
use crate::room::{Heard, Refusal, Relay, RoomKind, To};
use crate::session::Session;
use crate::words::words;

const LINK: &str = "nori://pair#";
/// How long a code lives.
pub const CODE_MS: i64 = 5 * 60 * 1000;
/// The version of the code and of the key sent through it.
const VERSION: u32 = 1;

/// Why pairing refused an input or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PairingRefused {
    /// Not a pairing code.
    NotACode,
    /// Made by a newer nori: this one needs an update.
    Newer,
    /// Its five minutes are over.
    Expired,
    /// The new device is already in this device's group.
    AlreadyPaired,
    /// The new device left the meeting before the key reached it.
    Gone,
    /// What arrived was not sealed to this device's lock for this meeting: changed on the way, or meant
    /// for another. The new device goes on waiting.
    Tampered,
    /// A key after the one that paired the device, a replay.
    Spent,
    /// The sync server refused.
    Server { reason: Refusal },
}

/// How the code reached the device with the profiles. Only the camera needs no check of the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Came {
    Camera,
    /// A pasted link.
    Link,
    /// A waiting device listed in the pairing screen.
    Nearby,
}

/// What the device with the profiles asks the user to confirm before it pairs.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Confirmation {
    /// The new device's name.
    pub name: String,
    /// The profiles it receives that it can play.
    pub profiles: u32,
    /// When the new device leaves a group: that group's other devices.
    pub leaves: Option<u32>,
    /// The three words to compare with the new device's screen; None when the code came by camera.
    pub words: Option<Vec<String>>,
    /// The group's first pairing: ask whether to keep an encrypted copy on the server.
    pub asks_keep_copy: bool,
}

/// A pairing code, as the new device shows it.
#[derive(Debug, Clone, PartialEq)]
struct Code {
    server: String,
    meeting: RoomName,
    lock: LockKey,
    name: String,
    plays: Plays,
    /// The server's time, Unix ms, when it dies.
    expires_ms: i64,
    /// When the new device is in a group: that group's other devices, and a tag only a holder of the
    /// group's key makes for this lock.
    group: Option<(u32, String)>,
}

impl Code {
    /// The link, which is also the QR code's text.
    fn link(&self) -> String {
        let plays = match self.plays {
            Plays::Servers => "s",
            Plays::ServersAndPlugins => "a",
        };
        let mut link = format!("{LINK}v={VERSION}&s={}&r={}&l={}&n={}&p={plays}&e={}", b64(self.server.as_bytes()), self.meeting.as_str(), self.lock.text(), b64(self.name.as_bytes()), self.expires_ms);
        if let Some((others, tag)) = &self.group {
            link.push_str(&format!("&g={others}&t={tag}"));
        }
        link
    }

    fn parse(text: &str) -> Result<Code, PairingRefused> {
        use PairingRefused::NotACode;
        let fragment = text.trim().strip_prefix(LINK).ok_or(NotACode)?;
        let field = |k: &str| fragment.split('&').filter_map(|kv| kv.split_once('=')).find(|(key, _)| *key == k).map(|(_, v)| v);
        let version: u32 = field("v").and_then(|v| v.parse().ok()).ok_or(NotACode)?;
        if version > VERSION {
            return Err(PairingRefused::Newer);
        }
        let text_of = |k| field(k).and_then(unb64).and_then(|b| String::from_utf8(b).ok());
        let server = text_of("s").filter(|s| s.starts_with("https://") || s.starts_with("http://")).ok_or(NotACode)?;
        let plays = match field("p") {
            Some("s") => Plays::Servers,
            Some("a") => Plays::ServersAndPlugins,
            _ => return Err(NotACode),
        };
        let group = match (field("g").map(str::parse), field("t")) {
            (None, None) => None,
            (Some(Ok(others)), Some(tag)) => Some((others, tag.to_string())),
            _ => return Err(NotACode),
        };
        Ok(Code {
            server,
            meeting: field("r").and_then(RoomName::parse).ok_or(NotACode)?,
            lock: field("l").and_then(LockKey::from_text).ok_or(NotACode)?,
            name: text_of("n").filter(|n| !n.trim().is_empty()).ok_or(NotACode)?,
            plays,
            expires_ms: field("e").and_then(|e| e.parse().ok()).ok_or(NotACode)?,
            group,
        })
    }
}

/// The group's key and what goes with it, sealed to the new device's lock.
#[derive(Serialize, Deserialize)]
struct Welcome {
    v: u32,
    key: String,
    server: String,
    keep: KeepCopy,
}

/// What a step of pairing on the new device came to.
pub enum JoinStep {
    /// Show this code: `link` is also the QR code's text. The words show beside it.
    Code { link: String, words: Vec<String> },
    /// The key arrived: the device is in the group ([`Welcomed::join`]).
    Paired(Welcomed),
    /// [`PairingRefused::Tampered`] and [`PairingRefused::Spent`] leave the device waiting; the rest end
    /// this pairing.
    Refused(PairingRefused),
}

/// The new device's side: it makes its lock, picks a meeting room, shows its code and waits there.
pub struct Joining {
    lock: Lock,
    meeting: RoomName,
    server: String,
    name: String,
    plays: Plays,
    group: Option<(u32, String)>,
    /// This device's monotonic clock when it started.
    started: Duration,
    joined: bool,
    time_ms: Option<i64>,
    paired: bool,
}

impl Joining {
    /// Starts pairing this device, `name`, into a group on `server` (the sync server setting's). It
    /// leaves `previous`, the group it is in, once paired.
    pub fn start(relay: &dyn Relay, server: String, name: String, plays: Plays, previous: Option<&Session>, now: Duration) -> Joining {
        let lock = Lock::new();
        let group = previous.map(|s| (s.devices().len() as u32, s.key().tag(lock.key())));
        let joining = Joining { lock, meeting: RoomName::random(), server, name, plays, group, started: now, joined: false, time_ms: None, paired: false };
        relay.join(&joining.meeting, RoomKind::Meeting);
        relay.ask_time();
        joining
    }

    /// What the server said; `now` is this device's monotonic clock.
    pub fn heard(&mut self, relay: &dyn Relay, heard: Heard, now: Duration) -> Option<JoinStep> {
        match heard {
            Heard::Joined { room, .. } if room == self.meeting => {
                self.joined = true;
                self.code()
            }
            Heard::Time { ms } if self.time_ms.is_none() => {
                self.time_ms = Some(ms);
                self.code()
            }
            Heard::Closed { room } if room == self.meeting && !self.paired => Some(JoinStep::Refused(PairingRefused::Expired)),
            Heard::Refused { reason, .. } if !self.paired => {
                relay.leave(&self.meeting);
                Some(JoinStep::Refused(PairingRefused::Server { reason }))
            }
            Heard::Message { room, payload, .. } if room == self.meeting => Some(self.welcome(relay, &payload, now)),
            _ => None,
        }
    }

    /// The code, once the device is in the meeting room and knows the server's time.
    fn code(&self) -> Option<JoinStep> {
        let time_ms = self.time_ms.filter(|_| self.joined)?;
        let code = Code { server: self.server.clone(), meeting: self.meeting.clone(), lock: self.lock.key().clone(), name: self.name.clone(), plays: self.plays, expires_ms: time_ms + CODE_MS, group: self.group.clone() };
        Some(JoinStep::Code { link: code.link(), words: words(self.lock.key()) })
    }

    fn welcome(&mut self, relay: &dyn Relay, payload: &[u8], now: Duration) -> JoinStep {
        if self.paired {
            return JoinStep::Refused(PairingRefused::Spent);
        }
        if now.saturating_sub(self.started) > Duration::from_millis(CODE_MS as u64) {
            relay.leave(&self.meeting);
            return JoinStep::Refused(PairingRefused::Expired);
        }
        let Some(welcome) = self.lock.open(payload, &self.meeting).and_then(|p| serde_json::from_slice::<Welcome>(&p).ok()) else {
            return JoinStep::Refused(PairingRefused::Tampered);
        };
        if welcome.v > VERSION {
            return JoinStep::Refused(PairingRefused::Newer);
        }
        let Some(key) = GroupKey::from_text(&welcome.key) else { return JoinStep::Refused(PairingRefused::Tampered) };
        self.paired = true;
        relay.leave(&self.meeting);
        let group = Group { key, server: welcome.server, device: new_device_id(), name: self.name.clone(), seen: Vec::new(), keep: welcome.keep };
        JoinStep::Paired(Welcomed { group })
    }
}

/// A new device's way into its group.
pub struct Welcomed {
    group: Group,
}

impl Welcomed {
    /// The device's session in its new group, kept in `store`; it leaves `previous`, telling that
    /// group's devices through `relay`, and keeps its profiles' sync. The client connects the session.
    pub fn join(self, previous: Option<(Session, &dyn Relay)>, store: Arc<Settings>) -> Session {
        let book = previous.map(|(s, relay)| s.leave(relay).book).unwrap_or_default();
        let session = Session::with(Kept { group: self.group, book }, store, true);
        session.save();
        session
    }
}

/// What a step of pairing on the device with the profiles came to.
#[derive(Debug, Clone, PartialEq)]
pub enum AdmitStep {
    /// Ask the user this; [`Admitting::admit`] or [`Admitting::admit_first`] follows a yes.
    Confirm(Confirmation),
    /// The key went to the new device.
    Sent,
    Refused(PairingRefused),
}

#[derive(Debug, Clone, PartialEq)]
enum AdmitState {
    /// Asking the server's time, to tell a dead code.
    Checking,
    /// Waiting for the user.
    Asking,
    /// The sealed key, waiting to be sent in the meeting room.
    Sending(Vec<u8>),
    Done,
}

/// The side of the device with the profiles: it takes a code, says what to confirm, and sends the key.
pub struct Admitting {
    code: Code,
    came: Came,
    state: AdmitState,
}

impl Admitting {
    /// Takes `text`, a scanned code, a pasted link or a nearby device's announcement; `own` is this
    /// device's group.
    pub fn read(text: &str, came: Came, own: Option<&Session>) -> Result<Admitting, PairingRefused> {
        let code = Code::parse(text)?;
        if let (Some(own), Some((_, tag))) = (own, &code.group) {
            if own.key().tag(&code.lock) == *tag {
                return Err(PairingRefused::AlreadyPaired);
            }
        }
        Ok(Admitting { code, came, state: AdmitState::Checking })
    }

    /// The server the meeting is on: the client connects a [`Relay`] there.
    pub fn server(&self) -> &str {
        &self.code.server
    }

    /// Asks the server's time; [`Admitting::heard`] answers with what to confirm.
    pub fn check(&self, relay: &dyn Relay) {
        relay.ask_time();
    }

    fn confirmation(&self, own: Option<&Session>, servers: &[SavedServer]) -> Confirmation {
        Confirmation {
            name: self.code.name.clone(),
            profiles: servers.iter().filter(|s| !s.sync_off && !guest(s) && self.code.plays.plays(&s.kind)).count() as u32,
            leaves: self.code.group.as_ref().map(|(others, _)| *others),
            words: (self.came != Came::Camera).then(|| words(&self.code.lock)),
            asks_keep_copy: own.is_none(),
        }
    }

    /// What the server said; `own` and `servers` are this device's group and profiles.
    pub fn heard(&mut self, relay: &dyn Relay, heard: Heard, own: Option<&Session>, servers: &[SavedServer]) -> Option<AdmitStep> {
        match (heard, &self.state) {
            (Heard::Time { ms }, AdmitState::Checking) if ms >= self.code.expires_ms => {
                self.state = AdmitState::Done;
                Some(AdmitStep::Refused(PairingRefused::Expired))
            }
            (Heard::Time { .. }, AdmitState::Checking) => {
                self.state = AdmitState::Asking;
                Some(AdmitStep::Confirm(self.confirmation(own, servers)))
            }
            (Heard::Joined { room, members, .. }, AdmitState::Sending(sealed)) if room == self.code.meeting => {
                if !members.is_empty() {
                    relay.send(&room, To::Everyone, sealed.clone());
                }
                relay.leave(&room);
                self.state = AdmitState::Done;
                Some(if members.is_empty() { AdmitStep::Refused(PairingRefused::Gone) } else { AdmitStep::Sent })
            }
            (Heard::Refused { reason, .. }, state) if *state != AdmitState::Done => {
                self.state = AdmitState::Done;
                Some(AdmitStep::Refused(PairingRefused::Server { reason }))
            }
            _ => None,
        }
    }

    fn send(&mut self, relay: &dyn Relay, group: &Group) {
        let welcome = Welcome { v: VERSION, key: group.key.text(), server: group.server.clone(), keep: group.keep.clone() };
        let plain = serde_json::to_vec(&welcome).expect("a welcome serializes");
        let Some(sealed) = self.code.lock.seal(&plain, &self.code.meeting) else {
            self.state = AdmitState::Done;
            return;
        };
        self.state = AdmitState::Sending(sealed);
        relay.join(&self.code.meeting, RoomKind::Meeting);
    }

    /// The user confirmed: `own`'s key goes to the new device.
    pub fn admit(&mut self, relay: &dyn Relay, own: &Session) {
        if self.state == AdmitState::Asking {
            self.send(relay, own.group());
        }
    }

    /// The user confirmed the group's first pairing: this device, `name`, makes the group on the code's
    /// server, keeping a copy there when `keep_copy`, and sends its key. The session is the client's to
    /// keep and connect.
    pub fn admit_first(&mut self, relay: &dyn Relay, name: String, keep_copy: bool, store: Arc<Settings>, now_ms: i64) -> Option<Session> {
        if self.state != AdmitState::Asking {
            return None;
        }
        let device = new_device_id();
        let mut book = Book::default();
        let keep = KeepCopy { on: keep_copy, stamp: book.next(&device, now_ms) };
        let group = Group { key: GroupKey::new(), server: self.code.server.clone(), device, name, seen: Vec::new(), keep };
        self.send(relay, &group);
        let session = Session::with(Kept { group, book }, store, false);
        session.save();
        Some(session)
    }
}

#[cfg(test)]
mod tests;
