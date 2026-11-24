//! A sync server in memory, for tests: rooms, members, the kept copy and the server's clock, which the
//! test moves. Each device's link keeps what it heard until the test takes it, so devices run in
//! turns, with no thread and no sleeping.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::keys::RoomName;
use crate::room::{Asked, Heard, Member, Refusal, Relay, RoomKind, To};

/// How long a meeting room stays open.
pub const MEETING_MS: i64 = 5 * 60 * 1000;
/// The largest message the server passes.
const MESSAGE_BYTES: usize = 64 * 1024;

struct Room {
    kind: RoomKind,
    opened_ms: i64,
    /// Each member and the link it is.
    members: Vec<(Member, usize)>,
}

#[derive(Default)]
struct State {
    now_ms: i64,
    rooms: BTreeMap<RoomName, Room>,
    copies: BTreeMap<RoomName, ([u8; 32], Vec<u8>)>,
    inboxes: Vec<VecDeque<Heard>>,
    next_member: u32,
    wall: bool,
}

impl State {
    fn tell(&mut self, link: usize, heard: Heard) {
        self.inboxes[link].push_back(heard);
    }

    fn refuse(&mut self, link: usize, asked: Asked, reason: Refusal) {
        self.tell(link, Heard::Refused { asked, reason });
    }

    /// `link` leaves `room`; the others hear it, and an empty room closes.
    fn leave(&mut self, room: &RoomName, link: usize) {
        let Some(r) = self.rooms.get_mut(room) else { return };
        let Some(at) = r.members.iter().position(|(_, l)| *l == link) else { return };
        let (member, _) = r.members.remove(at);
        let others: Vec<usize> = r.members.iter().map(|(_, l)| *l).collect();
        if others.is_empty() {
            self.rooms.remove(room);
        }
        for other in others {
            self.tell(other, Heard::Left { room: room.clone(), member });
        }
    }
}

/// The server.
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<State>>);

/// One device's connection to the [`Hub`].
pub struct HubLink {
    hub: Arc<Mutex<State>>,
    link: usize,
}

impl Hub {
    /// A server whose clock reads `now_ms`.
    pub fn new(now_ms: i64) -> Hub {
        Hub(Arc::new(Mutex::new(State { now_ms, ..State::default() })))
    }

    pub fn link(&self) -> HubLink {
        let mut s = self.0.lock().unwrap();
        s.inboxes.push(VecDeque::new());
        HubLink { hub: self.0.clone(), link: s.inboxes.len() - 1 }
    }

    /// Moves the clock on by `ms`; meeting rooms whose time ran out close.
    pub fn advance(&self, ms: i64) {
        let mut s = self.0.lock().unwrap();
        s.now_ms += ms;
        let now = s.now_ms;
        let over: Vec<RoomName> = s.rooms.iter().filter(|(_, r)| r.kind == RoomKind::Meeting && now - r.opened_ms >= MEETING_MS).map(|(n, _)| n.clone()).collect();
        for name in over {
            let room = s.rooms.remove(&name).expect("listed");
            for (_, link) in room.members {
                s.tell(link, Heard::Closed { room: name.clone() });
            }
        }
    }

    /// The daily wall: while up, every request is refused.
    pub fn wall(&self, up: bool) {
        self.0.lock().unwrap().wall = up;
    }

    /// The kept copy of `room`, as stored.
    pub fn copy(&self, room: &RoomName) -> Option<Vec<u8>> {
        self.0.lock().unwrap().copies.get(room).map(|(_, blob)| blob.clone())
    }

    /// How many members `room` has.
    pub fn members(&self, room: &RoomName) -> usize {
        self.0.lock().unwrap().rooms.get(room).map_or(0, |r| r.members.len())
    }
}

impl HubLink {
    /// What this device heard since it last asked.
    pub fn heard(&self) -> Vec<Heard> {
        self.hub.lock().unwrap().inboxes[self.link].drain(..).collect()
    }

    /// The connection drops: it leaves every room.
    pub fn drop_connection(&self) {
        let mut s = self.hub.lock().unwrap();
        let rooms: Vec<RoomName> = s.rooms.iter().filter(|(_, r)| r.members.iter().any(|(_, l)| *l == self.link)).map(|(n, _)| n.clone()).collect();
        for room in rooms {
            s.leave(&room, self.link);
        }
    }

    /// The state, unless the wall refuses `asked`.
    fn open(&self, asked: Asked) -> Option<std::sync::MutexGuard<'_, State>> {
        let mut s = self.hub.lock().unwrap();
        if s.wall {
            s.refuse(self.link, asked, Refusal::DailyWall);
            return None;
        }
        Some(s)
    }
}

impl Relay for HubLink {
    fn join(&self, room: &RoomName, kind: RoomKind) {
        let Some(mut s) = self.open(Asked::Join) else { return };
        let now = s.now_ms;
        let member = Member(s.next_member);
        let r = s.rooms.entry(room.clone()).or_insert(Room { kind, opened_ms: now, members: Vec::new() });
        if r.kind == RoomKind::Meeting && r.members.len() >= 2 {
            return s.refuse(self.link, Asked::Join, Refusal::Limit);
        }
        let others = r.members.clone();
        r.members.push((member, self.link));
        s.next_member += 1;
        s.tell(self.link, Heard::Joined { room: room.clone(), me: member, members: others.iter().map(|(m, _)| *m).collect() });
        for (_, link) in others {
            s.tell(link, Heard::Arrived { room: room.clone(), member });
        }
    }

    fn leave(&self, room: &RoomName) {
        self.hub.lock().unwrap().leave(room, self.link);
    }

    fn send(&self, room: &RoomName, to: To, payload: Vec<u8>) {
        let Some(mut s) = self.open(Asked::Send) else { return };
        if payload.len() > MESSAGE_BYTES {
            return s.refuse(self.link, Asked::Send, Refusal::Limit);
        }
        let Some(r) = s.rooms.get(room) else { return };
        let Some(from) = r.members.iter().find(|(_, l)| *l == self.link).map(|(m, _)| *m) else { return };
        let targets: Vec<usize> = r.members.iter().filter(|(m, l)| *l != self.link && (to == To::Everyone || to == To::Member(*m))).map(|(_, l)| *l).collect();
        for link in targets {
            s.tell(link, Heard::Message { room: room.clone(), from, payload: payload.clone() });
        }
    }

    fn ask_time(&self) {
        let Some(mut s) = self.open(Asked::Time) else { return };
        let ms = s.now_ms;
        s.tell(self.link, Heard::Time { ms });
    }

    fn put_copy(&self, room: &RoomName, secret: [u8; 32], blob: Vec<u8>) {
        let Some(mut s) = self.open(Asked::PutCopy) else { return };
        if s.rooms.get(room).is_some_and(|r| r.kind != RoomKind::Group) || blob.len() > MESSAGE_BYTES {
            return s.refuse(self.link, Asked::PutCopy, Refusal::Limit);
        }
        if s.copies.get(room).is_some_and(|(set, _)| *set != secret) {
            return s.refuse(self.link, Asked::PutCopy, Refusal::WrongSecret);
        }
        s.copies.insert(room.clone(), (secret, blob));
    }

    fn get_copy(&self, room: &RoomName) {
        let Some(mut s) = self.open(Asked::GetCopy) else { return };
        let blob = s.copies.get(room).map(|(_, blob)| blob.clone());
        s.tell(self.link, Heard::Copy { room: room.clone(), blob });
    }

    fn delete_copy(&self, room: &RoomName, secret: [u8; 32]) {
        let Some(mut s) = self.open(Asked::DeleteCopy) else { return };
        match s.copies.get(room) {
            Some((set, _)) if *set != secret => s.refuse(self.link, Asked::DeleteCopy, Refusal::WrongSecret),
            _ => {
                s.copies.remove(room);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_hear_each_other_and_a_meeting_room_holds_two_for_five_minutes() {
        let hub = Hub::new(0);
        let (a, b, c) = (hub.link(), hub.link(), hub.link());
        let room = RoomName::random();
        a.join(&room, RoomKind::Meeting);
        b.join(&room, RoomKind::Meeting);
        c.join(&room, RoomKind::Meeting);
        assert_eq!(a.heard(), [Heard::Joined { room: room.clone(), me: Member(0), members: vec![] }, Heard::Arrived { room: room.clone(), member: Member(1) }]);
        assert_eq!(b.heard(), [Heard::Joined { room: room.clone(), me: Member(1), members: vec![Member(0)] }]);
        assert_eq!(c.heard(), [Heard::Refused { asked: Asked::Join, reason: Refusal::Limit }]);
        b.send(&room, To::Everyone, b"hi".to_vec());
        assert_eq!(a.heard(), [Heard::Message { room: room.clone(), from: Member(1), payload: b"hi".to_vec() }]);
        hub.advance(MEETING_MS);
        assert_eq!(a.heard(), [Heard::Closed { room: room.clone() }]);
        assert_eq!(hub.members(&room), 0);
    }

    #[test]
    fn the_first_put_sets_the_copy_secret_and_the_wall_refuses_everything() {
        let hub = Hub::new(7);
        let a = hub.link();
        let room = RoomName::random();
        a.join(&room, RoomKind::Group);
        a.put_copy(&room, [1; 32], b"one".to_vec());
        a.put_copy(&room, [2; 32], b"two".to_vec());
        a.delete_copy(&room, [2; 32]);
        a.get_copy(&room);
        let heard = a.heard();
        assert_eq!(heard[1..], [
            Heard::Refused { asked: Asked::PutCopy, reason: Refusal::WrongSecret },
            Heard::Refused { asked: Asked::DeleteCopy, reason: Refusal::WrongSecret },
            Heard::Copy { room: room.clone(), blob: Some(b"one".to_vec()) },
        ]);
        hub.wall(true);
        a.ask_time();
        assert_eq!(a.heard(), [Heard::Refused { asked: Asked::Time, reason: Refusal::DailyWall }]);
        hub.wall(false);
        a.ask_time();
        a.delete_copy(&room, [1; 32]);
        a.get_copy(&room);
        assert_eq!(a.heard(), [Heard::Time { ms: 7 }, Heard::Copy { room, blob: None }]);
    }
}
