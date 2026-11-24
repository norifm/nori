//! The sync server as this crate sees it: rooms that pass opaque messages between their members, the
//! server's time, and a group room's kept copy. A client hands a [`Relay`] to one server in, and gives
//! back what it hears as [`Heard`], in order. The wire encoding is the transport's.

use crate::keys::RoomName;

/// A member of a room: the short number the server gave its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Member(pub u32);

/// What a room is for. The server holds each to its limits: a meeting room has two members and closes
/// after about five minutes; only a group room keeps a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomKind {
    /// A person's devices, long-lived.
    Group,
    /// One pairing: the new device and the one that lets it in.
    Meeting,
}

/// Who a message goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    /// Every other member.
    Everyone,
    Member(Member),
}

/// Why the server refused a request. A refusal is a normal state: the client goes on without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Refusal {
    /// A limit: the room is full, or too many messages, bytes or rooms.
    Limit,
    /// The server's daily wall: it refuses everything until midnight UTC.
    DailyWall,
    /// The server does not speak this nori's protocol version.
    Version,
    /// A kept copy's put or delete with another write secret than the first put's.
    WrongSecret,
}

/// The request a refusal answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    Join,
    Send,
    Time,
    PutCopy,
    GetCopy,
    DeleteCopy,
}

/// What the server said, handed back to the crate in the order it arrived.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// This connection joined `room` as `me`; `members` are the others there.
    Joined { room: RoomName, me: Member, members: Vec<Member> },
    Arrived { room: RoomName, member: Member },
    Left { room: RoomName, member: Member },
    /// The server closed `room` (a meeting room's time ran out).
    Closed { room: RoomName },
    /// A payload as `from` sent it.
    Message { room: RoomName, from: Member, payload: Vec<u8> },
    /// The server's clock, Unix ms.
    Time { ms: i64 },
    /// The kept copy of `room`; None when it has none.
    Copy { room: RoomName, blob: Option<Vec<u8>> },
    Refused { asked: Asked, reason: Refusal },
}

/// One connection to one sync server. Every call returns at once; answers come back as [`Heard`].
pub trait Relay {
    fn join(&self, room: &RoomName, kind: RoomKind);
    fn leave(&self, room: &RoomName);
    fn send(&self, room: &RoomName, to: To, payload: Vec<u8>);
    fn ask_time(&self);
    /// Replaces the kept copy of group room `room`; the first put sets `secret`, later ones present it.
    fn put_copy(&self, room: &RoomName, secret: [u8; 32], blob: Vec<u8>);
    fn get_copy(&self, room: &RoomName);
    fn delete_copy(&self, room: &RoomName, secret: [u8; 32]);
}
