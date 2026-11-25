//! A paired device in its group's room: it says who it is, tells the others its profiles and takes
//! theirs, keeps the group's copy on the server, and notices devices arriving and leaving. Everything
//! it says is sealed with the group key; the server passes it on unread.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use nori_settings::settings::SavedServer;
use nori_settings::settings_store::Settings;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::book::{Changes, MergeOffer};
use crate::group::{Group, KeepCopy, Kept, ResetNotice, Seen};
use crate::keys::{GroupKey, RoomName, Sealed};
use crate::profile::{Here, NeededPlugin, Plays, SignInCheck};
use crate::room::{Heard, Member, Refusal, Relay, RoomKind, To};

/// The version of what devices say to each other. A change older devices cannot read raises
/// [`READ_BY`] too.
pub(crate) const VERSION: u32 = 1;
/// The oldest version that reads what this one says.
const READ_BY: u32 = 1;

/// Who a device is, as it tells the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Card {
    device: String,
    name: String,
    plays: Plays,
    version: u32,
}

/// What a group's devices say to each other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Said {
    /// Who the device is: `answer` to another's hello, `new` when it was just paired.
    Hello { card: Card, answer: bool, new: bool },
    /// Profiles and markers, and the kept copy's choice: `whole` when it is everything the device holds.
    Profiles { changes: Changes, keep: KeepCopy, whole: bool },
    /// The device leaves the group.
    Bye,
    /// Remote control's, for the devices with synced profile `profile` open; sync passes it on unread.
    Remote { profile: String, message: Value },
}

/// What a device says, with the versions that can read it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    v: u32,
    read_by: u32,
    said: Value,
}

/// The group's kept copy.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Copy {
    v: u32,
    read_by: u32,
    changes: Changes,
    keep: KeepCopy,
}

/// What a client shows of what happened in the group.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SyncEvent {
    /// "New device paired: `name`".
    NewDevice { name: String },
    /// `name` left the group: it was paired into another, or its pairing was reset.
    DeviceLeft { name: String },
    /// A device of the group said what only a newer nori reads: this one needs an update.
    UpdateThis,
    /// `name` runs a nori too old to read this one: it needs an update.
    UpdateOther { name: String },
    Refused { reason: Refusal },
}

/// What remote control hears in the group room.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteHeard {
    /// Device `device` (its group id) said `message` for synced profile `profile`.
    Said { device: String, profile: String, message: Value },
    /// Device `device` left the room.
    Left { device: String },
}

/// What a call did, for the client to carry out.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// The profiles as they are now, when sync changed them: the client keeps this list.
    pub servers: Option<Vec<SavedServer>>,
    /// Plugins to install for the profiles.
    pub install: Vec<NeededPlugin>,
    pub events: Vec<SyncEvent>,
    pub remote: Vec<RemoteHeard>,
}

/// The member a device of the group reached directly stands for while its messages are handled: no
/// server numbers it.
const DIRECT: Member = Member(u32::MAX);

/// The relay a direct exchange is handled over: what is said to [`DIRECT`] is its answer, and the rest
/// goes to the server as usual.
struct Direct<'a> {
    server: &'a dyn Relay,
    answer: RefCell<Vec<Vec<u8>>>,
}

impl Relay for Direct<'_> {
    fn join(&self, room: &RoomName, kind: RoomKind) {
        self.server.join(room, kind);
    }
    fn leave(&self, room: &RoomName) {
        self.server.leave(room);
    }
    fn send(&self, room: &RoomName, to: To, payload: Vec<u8>) {
        match to {
            To::Member(DIRECT) => self.answer.borrow_mut().push(payload),
            to => self.server.send(room, to, payload),
        }
    }
    fn ask_time(&self, room: &RoomName) {
        self.server.ask_time(room);
    }
    fn put_copy(&self, room: &RoomName, secret: [u8; 32], blob: Vec<u8>) {
        self.server.put_copy(room, secret, blob);
    }
    fn get_copy(&self, room: &RoomName) {
        self.server.get_copy(room);
    }
    fn delete_copy(&self, room: &RoomName, secret: [u8; 32]) {
        self.server.delete_copy(room, secret);
    }
}

/// One paired device.
pub struct Session {
    kept: Kept,
    store: Arc<Settings>,
    /// The members that said who they are.
    members: BTreeMap<Member, Card>,
    /// Just paired: its hellos say so while this session lasts, to the devices that reach the room late too.
    new: bool,
}

impl Session {
    /// The group `store` keeps; None on a device that never paired, which then does nothing at all.
    pub fn open(store: Arc<Settings>) -> Option<Session> {
        Some(Session::with(Kept::load(&store)?, store, false))
    }

    pub(crate) fn with(kept: Kept, store: Arc<Settings>, new: bool) -> Session {
        Session { kept, store, members: BTreeMap::new(), new }
    }

    pub(crate) fn save(&self) {
        self.kept.save(&self.store);
    }

    pub(crate) fn group(&self) -> &Group {
        &self.kept.group
    }

    pub(crate) fn key(&self) -> &GroupKey {
        &self.group().key
    }

    pub(crate) fn room(&self) -> RoomName {
        self.group().key.room()
    }

    /// The sync server the group uses: the client connects its [`Relay`] there.
    pub fn server(&self) -> &str {
        &self.group().server
    }

    /// Joins the group's room; the device then says who it is and catches up.
    pub fn connect(&self, relay: &dyn Relay) {
        relay.join(&self.room(), RoomKind::Group);
    }

    /// The group's other devices, with when each was last seen.
    pub fn devices(&self) -> Vec<Seen> {
        let online = |s: &Seen| self.members.values().any(|c| c.device == s.device);
        self.group().seen.iter().map(|s| Seen { online: online(s), ..s.clone() }).collect()
    }

    /// Whether the group keeps a copy on the server.
    pub fn keeps_copy(&self) -> bool {
        self.group().keep.on
    }

    /// Whether profile `local` syncs: removing it removes it from every paired device, and turning its
    /// sync off asks whether to remove it there.
    pub fn synced(&self, local: &str) -> bool {
        self.kept.book.synced(local)
    }

    pub fn offers(&self, servers: &[SavedServer]) -> Vec<MergeOffer> {
        self.kept.book.offers(servers)
    }

    /// Profile `local`'s sync identity, the same on every device, while it syncs.
    pub(crate) fn sync_id(&self, local: &str) -> Option<&str> {
        self.kept.book.sync_id(local)
    }

    /// Remote control's `message` for synced profile `profile`, to device `to` (its group id) or every
    /// other one; false when `to` is not in the room. Two clients over one app's data are one device of
    /// the group: each of its connections hears what is for it.
    pub(crate) fn remote_say(&self, relay: &dyn Relay, to: Option<&str>, profile: &str, message: Value) -> bool {
        let said = Said::Remote { profile: profile.into(), message };
        let Some(device) = to else {
            self.say(relay, To::Everyone, &said);
            return true;
        };
        let there: Vec<Member> = self.members.iter().filter(|(_, c)| c.device == device).map(|(m, _)| *m).collect();
        for member in &there {
            self.say(relay, To::Member(*member), &said);
        }
        !there.is_empty()
    }

    pub fn reset_notice(&self) -> ResetNotice {
        ResetNotice { others: self.group().seen.len() as u32, signed_in: self.kept.book.signed_in_elsewhere() }
    }

    fn card(&self, here: &Here) -> Card {
        Card { device: self.group().device.clone(), name: self.group().name.clone(), plays: here.plays, version: VERSION }
    }

    fn say(&self, relay: &dyn Relay, to: To, said: &Said) {
        relay.send(&self.room(), to, self.sealed(said));
    }

    fn sealed(&self, said: &Said) -> Vec<u8> {
        let read_by = if matches!(said, Said::Hello { .. }) { 1 } else { READ_BY };
        let envelope = Envelope { v: VERSION, read_by, said: serde_json::to_value(said).expect("what is said serializes") };
        let plain = serde_json::to_vec(&envelope).expect("an envelope serializes");
        self.group().key.seal(Sealed::Message, &plain)
    }

    /// What this device says first to a device of the group it reaches directly, on one network: who
    /// it is. The other answers with who it is and everything it holds.
    pub(crate) fn direct_hello(&self, here: &Here) -> Vec<u8> {
        self.sealed(&Said::Hello { card: self.card(here), answer: false, new: false })
    }

    /// What a device of the group reached directly said, handled as from a member of the room, with
    /// what this device says back to it; what the client is to do. What the server is to hear goes to
    /// `server`.
    pub(crate) fn direct(&mut self, server: &dyn Relay, payloads: Vec<Vec<u8>>, servers: &[SavedServer], here: &Here, now_ms: i64) -> (Outcome, Vec<Vec<u8>>) {
        let direct = Direct { server, answer: RefCell::default() };
        let (mut out, mut list) = (Outcome::default(), servers.to_vec());
        for payload in payloads {
            let o = self.message(&direct, DIRECT, &payload, &list, here, now_ms);
            if let Some(changed) = o.servers {
                list = changed.clone();
                out.servers = Some(changed);
            }
            out.install.extend(o.install);
            out.events.extend(o.events);
        }
        self.members.remove(&DIRECT);
        (out, direct.answer.into_inner())
    }

    /// Tells every member that said who it is `changes`, each without the sign-ins it cannot use.
    fn tell(&self, relay: &dyn Relay, changes: &Changes) {
        let fitted = |plays: Plays| changes.clone().fitted(plays);
        let keep = self.group().keep.clone();
        if self.members.values().all(|c| fitted(c.plays) == *changes) {
            return self.say(relay, To::Everyone, &Said::Profiles { changes: changes.clone(), keep, whole: false });
        }
        for (member, card) in &self.members {
            self.say(relay, To::Member(*member), &Said::Profiles { changes: fitted(card.plays), keep: keep.clone(), whole: false });
        }
    }

    fn put_copy(&self, relay: &dyn Relay) {
        let copy = Copy { v: VERSION, read_by: READ_BY, changes: self.kept.book.state(), keep: self.group().keep.clone() };
        let key = &self.group().key;
        relay.put_copy(&self.room(), key.copy_secret(), key.seal(Sealed::Copy, &serde_json::to_vec(&copy).expect("a copy serializes")));
    }

    /// After this device's own changes: tells the others and the kept copy.
    fn spread(&mut self, relay: &dyn Relay, changes: &Changes) {
        if changes.is_empty() {
            return;
        }
        self.tell(relay, changes);
        if self.keeps_copy() {
            self.put_copy(relay);
        }
        self.save();
    }

    /// The profiles in `servers` changed here (added, edited, removed): the group hears of it.
    pub fn changed(&mut self, relay: &dyn Relay, servers: &[SavedServer], here: &Here, now_ms: i64) {
        let by = self.group().device.clone();
        let changes = self.kept.book.notice(servers, here, &by, now_ms);
        self.spread(relay, &changes);
    }

    /// Takes `theirs` into `servers` and the group's choice; what changed for the client.
    fn take(&mut self, theirs: Changes, keep: KeepCopy, servers: &[SavedServer], here: &Here, now_ms: i64, relay: &dyn Relay) -> Outcome {
        self.changed(relay, servers, here, now_ms);
        let theirs = theirs.fitted(here.plays);
        let mut list = servers.to_vec();
        let taken = self.kept.book.take(theirs, &mut list, here);
        self.kept.book.seen(&keep.stamp);
        if keep.stamp > self.group().keep.stamp {
            self.kept.group.keep = keep;
        }
        self.save();
        Outcome { servers: taken.changed.then_some(list), install: taken.install, ..Outcome::default() }
    }

    /// Hands the session what the server said; what the client is to do follows.
    pub fn heard(&mut self, relay: &dyn Relay, heard: Heard, servers: &[SavedServer], here: &Here, now_ms: i64) -> Outcome {
        let room = self.room();
        match heard {
            Heard::Joined { room: r, .. } if r == room => {
                self.say(relay, To::Everyone, &Said::Hello { card: self.card(here), answer: false, new: self.new });
                if self.keeps_copy() {
                    relay.get_copy(&room);
                }
                Outcome::default()
            }
            Heard::Left { room: r, member } if r == room => {
                let Some(card) = self.members.remove(&member) else { return Outcome::default() };
                self.kept.group.saw(&card.device, &card.name, now_ms);
                self.save();
                Outcome { remote: vec![RemoteHeard::Left { device: card.device }], ..Outcome::default() }
            }
            Heard::Message { room: r, from, payload } if r == room => self.message(relay, from, &payload, servers, here, now_ms),
            Heard::Copy { room: r, blob } if r == room => self.copy(relay, blob, servers, here, now_ms),
            Heard::Refused { room: r, reason, .. } if r == room => Outcome { events: vec![SyncEvent::Refused { reason }], ..Outcome::default() },
            _ => Outcome::default(),
        }
    }

    fn message(&mut self, relay: &dyn Relay, from: Member, payload: &[u8], servers: &[SavedServer], here: &Here, now_ms: i64) -> Outcome {
        let Some(envelope) = self.group().key.open(Sealed::Message, payload).and_then(|p| serde_json::from_slice::<Envelope>(&p).ok()) else { return Outcome::default() };
        if envelope.read_by > VERSION {
            return Outcome { events: vec![SyncEvent::UpdateThis], ..Outcome::default() };
        }
        // A kind of message a newer nori added, which this one may pass over.
        let Ok(said) = serde_json::from_value::<Said>(envelope.said) else { return Outcome::default() };
        let mut out = Outcome::default();
        match said {
            Said::Hello { card, answer, new } => {
                let news = new && !self.group().seen.iter().any(|s| s.device == card.device);
                self.kept.group.saw(&card.device, &card.name, now_ms);
                if news {
                    out.events.push(SyncEvent::NewDevice { name: card.name.clone() });
                }
                if card.version < READ_BY {
                    out.events.push(SyncEvent::UpdateOther { name: card.name.clone() });
                }
                if !answer {
                    self.say(relay, To::Member(from), &Said::Hello { card: self.card(here), answer: true, new: self.new });
                }
                let whole = self.kept.book.state().fitted(card.plays);
                self.say(relay, To::Member(from), &Said::Profiles { changes: whole, keep: self.group().keep.clone(), whole: true });
                self.members.insert(from, card);
                self.save();
            }
            Said::Profiles { changes, keep, whole } => {
                let (said, their_keep) = (changes.clone(), keep.stamp.clone());
                out = self.take(changes, keep, servers, here, now_ms, relay);
                // What this device holds newer than the sender, or what a whole state lacked, goes back.
                let back = self.kept.book.answer(&said, whole);
                if let Some(card) = self.members.get(&from).filter(|_| !back.is_empty() || self.group().keep.stamp > their_keep) {
                    self.say(relay, To::Member(from), &Said::Profiles { changes: back.fitted(card.plays), keep: self.group().keep.clone(), whole: false });
                }
                if out.servers.is_some() && self.keeps_copy() {
                    self.put_copy(relay);
                }
            }
            Said::Bye => {
                if let Some(card) = self.members.remove(&from) {
                    self.kept.group.seen.retain(|s| s.device != card.device);
                    self.save();
                    out.remote.push(RemoteHeard::Left { device: card.device.clone() });
                    out.events.push(SyncEvent::DeviceLeft { name: card.name });
                }
            }
            Said::Remote { profile, message } => {
                if let Some(card) = self.members.get(&from) {
                    out.remote.push(RemoteHeard::Said { device: card.device.clone(), profile, message });
                }
            }
        }
        out
    }

    /// The kept copy arrived: older than what is here, it changes nothing, and what is here newer goes
    /// back into it.
    fn copy(&mut self, relay: &dyn Relay, blob: Option<Vec<u8>>, servers: &[SavedServer], here: &Here, now_ms: i64) -> Outcome {
        let copy = blob.and_then(|b| self.group().key.open(Sealed::Copy, &b)).and_then(|p| serde_json::from_slice::<Copy>(&p).ok());
        let Some(copy) = copy else {
            if self.keeps_copy() {
                self.put_copy(relay);
            }
            return Outcome::default();
        };
        if copy.read_by > VERSION {
            return Outcome { events: vec![SyncEvent::UpdateThis], ..Outcome::default() };
        }
        let behind = !self.kept.book.answer(&copy.changes, true).is_empty() || copy.keep.stamp < self.group().keep.stamp;
        let out = self.take(copy.changes, copy.keep, servers, here, now_ms, relay);
        if behind && self.keeps_copy() {
            self.put_copy(relay);
        }
        out
    }

    /// Turns the kept copy on or off for the whole group; off deletes it from the server.
    pub fn keep_copy(&mut self, relay: &dyn Relay, on: bool, now_ms: i64) {
        if on == self.keeps_copy() {
            return;
        }
        let stamp = self.kept.book.next(&self.group().device.clone(), now_ms);
        self.kept.group.keep = KeepCopy { on, stamp };
        self.say(relay, To::Everyone, &Said::Profiles { changes: Changes::default(), keep: self.group().keep.clone(), whole: false });
        if on {
            self.put_copy(relay);
        } else {
            relay.delete_copy(&self.room(), self.group().key.copy_secret());
        }
        self.save();
    }

    /// Sync this profile off for `local`; with `elsewhere`, it is removed from the other devices too.
    /// The client then sets the profile's `sync_off`.
    pub fn sync_off(&mut self, relay: &dyn Relay, local: &str, elsewhere: bool, now_ms: i64) {
        let by = self.group().device.clone();
        let changes = self.kept.book.sync_off(local, elsewhere, &by, now_ms);
        self.spread(relay, &changes);
        self.save();
    }

    /// Merges profile `drop` into `keep` on every device; this device's profiles as they are after.
    pub fn merge(&mut self, relay: &dyn Relay, keep: &str, drop: &str, servers: &[SavedServer], now_ms: i64) -> Vec<SavedServer> {
        let by = self.group().device.clone();
        let mut list = servers.to_vec();
        let changes = self.kept.book.merge(keep, drop, &mut list, &by, now_ms);
        self.spread(relay, &changes);
        list
    }

    /// Profiles `a` and `b` are not one: they are offered for merging no more.
    pub fn keep_apart(&mut self, a: &str, b: &str) {
        self.kept.book.keep_apart(a, b);
        self.save();
    }

    /// A sign-in on this device to `account` (the provider's stable identifier) for profile `local`:
    /// accepted only for the account the profile syncs with.
    pub fn signed_in(&mut self, relay: &dyn Relay, local: &str, account: Option<String>, now_ms: i64) -> SignInCheck {
        let by = self.group().device.clone();
        let (check, changes) = self.kept.book.signed_in(local, account, &by, now_ms);
        self.spread(relay, &changes);
        check
    }

    /// Leaves the group, telling its devices.
    pub(crate) fn leave(self, relay: &dyn Relay) -> Kept {
        self.say(relay, To::Everyone, &Said::Bye);
        relay.leave(&self.room());
        self.kept
    }

    /// Reset pairing: this device leaves the group and starts a new one of its own, with a new key. The
    /// other devices keep their data and stop syncing with it.
    pub fn reset(&mut self, relay: &dyn Relay) {
        self.say(relay, To::Everyone, &Said::Bye);
        relay.leave(&self.room());
        self.kept.group.key = GroupKey::new();
        self.kept.group.seen.clear();
        self.members.clear();
        self.save();
        self.connect(relay);
    }
}

#[cfg(test)]
mod tests;
