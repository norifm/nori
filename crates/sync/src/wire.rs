//! The sync server's frames, version 1 (the server project's PROTOCOL.md): what a member sends, what the
//! server answers, its close codes, and the address of a room. Integers are big-endian; times are µs
//! since the Unix epoch on the server's clock.

use crate::keys::RoomName;
use crate::room::{Refusal, RoomKind};

/// The protocol version this nori speaks.
pub const VERSION: u8 = 1;

/// A frame a member sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Up<'a> {
    /// To every other member.
    SendAll(&'a [u8]),
    /// To one member.
    SendTo(u16, &'a [u8]),
    /// The member's own clock, µs, returned unchanged with the server's.
    Time(i64),
    KeptGet,
    KeptPut(&'a [u8; 32], &'a [u8]),
    KeptDelete(&'a [u8; 32]),
}

impl Up<'_> {
    /// The frame's tag, which a `done` or `refused` names.
    fn tag(&self) -> u8 {
        match self {
            Up::SendAll(_) => 0x01,
            Up::SendTo(..) => 0x02,
            Up::Time(_) => 0x03,
            Up::KeptGet => 0x04,
            Up::KeptPut(..) => 0x05,
            Up::KeptDelete(_) => 0x06,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.tag()];
        match self {
            Up::SendAll(payload) => out.extend_from_slice(payload),
            Up::SendTo(member, payload) => {
                out.extend_from_slice(&member.to_be_bytes());
                out.extend_from_slice(payload);
            }
            Up::Time(t1) => out.extend_from_slice(&t1.to_be_bytes()),
            Up::KeptGet => {}
            Up::KeptPut(secret, blob) => {
                out.extend_from_slice(&secret[..]);
                out.extend_from_slice(blob);
            }
            Up::KeptDelete(secret) => out.extend_from_slice(&secret[..]),
        }
        out
    }
}

/// The first frame of an admitted connection: who it is, the room's others and the server's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    pub you: u16,
    pub max_frame: u32,
    pub messages_per_minute: u32,
    pub bytes_per_minute: u32,
    pub max_members: u16,
    /// When a meeting room closes, µs; None for other rooms.
    pub closes_at: Option<i64>,
    pub members: Vec<u16>,
}

/// A frame the server sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Down {
    Welcome(Welcome),
    Joined(u16),
    Left(u16),
    /// `alone`: sent to this member only.
    Message { from: u16, alone: bool, payload: Vec<u8> },
    /// `t1` as sent, `t2` when the request arrived, `t3` when the answer left, µs.
    Time { t1: i64, t2: i64, t3: i64 },
    Kept(Option<Vec<u8>>),
    /// A kept-copy put or delete went through.
    Done(Asked),
    Refused { asked: Asked, code: Code, retry_after_ms: u32 },
}

/// The request a `done` or `refused` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    SendAll,
    SendTo,
    Time,
    KeptGet,
    KeptPut,
    KeptDelete,
}

impl Asked {
    fn of(tag: u8) -> Option<Asked> {
        Some(match tag {
            0x01 => Asked::SendAll,
            0x02 => Asked::SendTo,
            0x03 => Asked::Time,
            0x04 => Asked::KeptGet,
            0x05 => Asked::KeptPut,
            0x06 => Asked::KeptDelete,
            _ => return None,
        })
    }
}

/// A frame from the server that this version does not read: a bug on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable(pub &'static str);

/// Reads the front of a frame's body.
struct Body<'a>(&'a [u8]);

impl<'a> Body<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Unreadable> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(Unreadable("a frame cut short"))?;
        self.0 = rest;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8, Unreadable> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, Unreadable> {
        self.take().map(u16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, Unreadable> {
        self.take().map(u32::from_be_bytes)
    }

    fn i64(&mut self) -> Result<i64, Unreadable> {
        self.take().map(i64::from_be_bytes)
    }

    fn rest(self) -> Vec<u8> {
        self.0.to_vec()
    }

    fn end(self) -> Result<(), Unreadable> {
        if self.0.is_empty() { Ok(()) } else { Err(Unreadable("a frame too long")) }
    }
}

impl Down {
    pub fn decode(frame: &[u8]) -> Result<Down, Unreadable> {
        let (&tag, body) = frame.split_first().ok_or(Unreadable("an empty frame"))?;
        let mut b = Body(body);
        let down = match tag {
            0x80 => {
                if b.u8()? != VERSION {
                    return Err(Unreadable("a welcome of another version"));
                }
                // The kind is the one the connection asked for: a group, a meeting or a jam room.
                if !(1..=3).contains(&b.u8()?) {
                    return Err(Unreadable("an unknown kind of room"));
                }
                let (you, max_frame, messages_per_minute, bytes_per_minute, max_members) = (b.u16()?, b.u32()?, b.u32()?, b.u32()?, b.u16()?);
                let closes_at = Some(b.i64()?).filter(|&t| t != 0);
                let count = b.u16()?;
                let members = (0..count).map(|_| b.u16()).collect::<Result<_, _>>()?;
                b.end()?;
                return Ok(Down::Welcome(Welcome { you, max_frame, messages_per_minute, bytes_per_minute, max_members, closes_at, members }));
            }
            0x81 => Down::Joined(b.u16()?),
            0x82 => Down::Left(b.u16()?),
            0x83 => {
                let from = b.u16()?;
                let alone = match b.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(Unreadable("a message to no one known")),
                };
                return Ok(Down::Message { from, alone, payload: b.rest() });
            }
            0x84 => Down::Time { t1: b.i64()?, t2: b.i64()?, t3: b.i64()? },
            0x85 => {
                return match b.u8()? {
                    0 => b.end().map(|_| Down::Kept(None)),
                    1 => Ok(Down::Kept(Some(b.rest()))),
                    _ => Err(Unreadable("a kept copy neither there nor not")),
                };
            }
            0x86 => Down::Done(Asked::of(b.u8()?).ok_or(Unreadable("done for no request"))?),
            0x87 => {
                let asked = Asked::of(b.u8()?).ok_or(Unreadable("a refusal of no request"))?;
                let code = Code::of(b.u16()?).ok_or(Unreadable("an unknown refusal"))?;
                Down::Refused { asked, code, retry_after_ms: b.u32()? }
            }
            _ => return Err(Unreadable("an unknown frame")),
        };
        b.end()?;
        Ok(down)
    }
}

/// Why the server refused: a close code, or a `refused` frame's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    BadRequest,
    VersionUnsupported,
    WrongKind,
    RoomFull,
    RoomClosed,
    TooManyRooms,
    DailyLimit,
    Unavailable,
    TooLarge,
    TooFast,
    WrongSecret,
    NoMember,
}

impl Code {
    pub fn of(code: u16) -> Option<Code> {
        Some(match code {
            4000 => Code::BadRequest,
            4001 => Code::VersionUnsupported,
            4002 => Code::WrongKind,
            4003 => Code::RoomFull,
            4004 => Code::RoomClosed,
            4005 => Code::TooManyRooms,
            4006 => Code::DailyLimit,
            4007 => Code::Unavailable,
            4008 => Code::TooLarge,
            4009 => Code::TooFast,
            4010 => Code::WrongSecret,
            4011 => Code::NoMember,
            _ => return None,
        })
    }

    /// What the client shows; None for a meeting room's end and a member gone, which are no refusal.
    pub fn refusal(self) -> Option<Refusal> {
        Some(match self {
            Code::BadRequest | Code::WrongKind => Refusal::Invalid,
            Code::VersionUnsupported => Refusal::Version,
            Code::RoomFull | Code::TooManyRooms | Code::TooLarge | Code::TooFast => Refusal::Limit,
            Code::DailyLimit => Refusal::DailyWall,
            Code::Unavailable => Refusal::Unavailable,
            Code::WrongSecret => Refusal::WrongSecret,
            Code::RoomClosed | Code::NoMember => return None,
        })
    }
}

/// The address of `room` on `server` (`https://…`, or `http://…` for a local one, with a path prefix if
/// it has one); None for an address that is neither.
pub fn room_url(server: &str, room: &RoomName, kind: RoomKind) -> Option<String> {
    let (scheme, rest) = server.trim().split_once("://")?;
    let scheme = match scheme {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    let kind = match kind {
        RoomKind::Group => "group",
        RoomKind::Meeting => "meeting",
    };
    Some(format!("{scheme}://{}/room/{}?v={VERSION}&kind={kind}", rest.trim_end_matches('/'), room.as_str()))
}

/// The server's side, for tests that stand in for it.
#[cfg(any(test, feature = "testing"))]
impl Down {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let tag_of = |asked: &Asked| match asked {
            Asked::SendAll => 0x01,
            Asked::SendTo => 0x02,
            Asked::Time => 0x03,
            Asked::KeptGet => 0x04,
            Asked::KeptPut => 0x05,
            Asked::KeptDelete => 0x06,
        };
        match self {
            Down::Welcome(w) => {
                let mut out = vec![0x80, VERSION, if w.closes_at.is_some() { 2 } else { 1 }];
                out.extend_from_slice(&w.you.to_be_bytes());
                out.extend_from_slice(&w.max_frame.to_be_bytes());
                out.extend_from_slice(&w.messages_per_minute.to_be_bytes());
                out.extend_from_slice(&w.bytes_per_minute.to_be_bytes());
                out.extend_from_slice(&w.max_members.to_be_bytes());
                out.extend_from_slice(&w.closes_at.unwrap_or(0).to_be_bytes());
                out.extend_from_slice(&(w.members.len() as u16).to_be_bytes());
                w.members.iter().for_each(|m| out.extend_from_slice(&m.to_be_bytes()));
                out
            }
            Down::Joined(m) => [&[0x81][..], &m.to_be_bytes()].concat(),
            Down::Left(m) => [&[0x82][..], &m.to_be_bytes()].concat(),
            Down::Message { from, alone, payload } => [&[0x83][..], &from.to_be_bytes(), &[*alone as u8], payload].concat(),
            Down::Time { t1, t2, t3 } => [&[0x84][..], &t1.to_be_bytes(), &t2.to_be_bytes(), &t3.to_be_bytes()].concat(),
            Down::Kept(None) => vec![0x85, 0],
            Down::Kept(Some(blob)) => [&[0x85, 1][..], blob].concat(),
            Down::Done(asked) => vec![0x86, tag_of(asked)],
            Down::Refused { asked, code, retry_after_ms } => [&[0x87, tag_of(asked)][..], &code.number().to_be_bytes(), &retry_after_ms.to_be_bytes()].concat(),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl Code {
    pub(crate) fn number(self) -> u16 {
        (4000..=4011).find(|&n| Code::of(n) == Some(self)).expect("every code has its number")
    }
}

/// What a member sent, read as the server reads it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn read_up(frame: &[u8]) -> Up<'_> {
    let (&tag, body) = frame.split_first().expect("a frame");
    match tag {
        0x01 => Up::SendAll(body),
        0x02 => Up::SendTo(u16::from_be_bytes([body[0], body[1]]), &body[2..]),
        0x03 => Up::Time(i64::from_be_bytes(body.try_into().expect("8 bytes"))),
        0x04 => Up::KeptGet,
        0x05 => Up::KeptPut(body[..32].try_into().expect("a secret"), &body[32..]),
        0x06 => Up::KeptDelete(body.try_into().expect("a secret")),
        _ => panic!("no such frame"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_member_sends_is_these_bytes() {
        let secret = [7; 32];
        let cases: [(Up, Vec<u8>); 6] = [
            (Up::SendAll(b"hi"), vec![0x01, b'h', b'i']),
            (Up::SendTo(0x0102, b"x"), vec![0x02, 0x01, 0x02, b'x']),
            (Up::Time(-2), vec![0x03, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]),
            (Up::KeptGet, vec![0x04]),
            (Up::KeptPut(&secret, b"b"), [&[0x05][..], &secret, b"b"].concat()),
            (Up::KeptDelete(&secret), [&[0x06][..], &secret].concat()),
        ];
        for (up, bytes) in cases {
            assert_eq!(up.encode(), bytes, "{up:?}");
            assert_eq!(read_up(&bytes), up, "the tests' server reads it so");
        }
        assert_eq!(Up::SendAll(b"").encode(), [0x01], "an empty payload");
    }

    #[test]
    fn what_the_server_sends_reads_from_these_bytes() {
        let welcome = [
            &[0x80, 1, 2, 0x00, 0x05][..],
            &65_536u32.to_be_bytes(),
            &240u32.to_be_bytes(),
            &4_194_304u32.to_be_bytes(),
            &2u16.to_be_bytes(),
            &1_700_000_000_000_000i64.to_be_bytes(),
            &[0x00, 0x02, 0x00, 0x01, 0x00, 0x03],
        ]
        .concat();
        let cases: [(Vec<u8>, Down); 9] = [
            (welcome, Down::Welcome(Welcome { you: 5, max_frame: 65_536, messages_per_minute: 240, bytes_per_minute: 4_194_304, max_members: 2, closes_at: Some(1_700_000_000_000_000), members: vec![1, 3] })),
            (vec![0x81, 0x00, 0x07], Down::Joined(7)),
            (vec![0x82, 0xff, 0xff], Down::Left(65_535)),
            (vec![0x83, 0x00, 0x02, 0x01, b'o', b'k'], Down::Message { from: 2, alone: true, payload: b"ok".to_vec() }),
            ([&[0x84][..], &1i64.to_be_bytes(), &2i64.to_be_bytes(), &3i64.to_be_bytes()].concat(), Down::Time { t1: 1, t2: 2, t3: 3 }),
            (vec![0x85, 0x00], Down::Kept(None)),
            (vec![0x85, 0x01, b'b'], Down::Kept(Some(b"b".to_vec()))),
            (vec![0x86, 0x06], Down::Done(Asked::KeptDelete)),
            (vec![0x87, 0x02, 0x0f, 0xa9, 0x00, 0x00, 0x03, 0xe8], Down::Refused { asked: Asked::SendTo, code: Code::TooFast, retry_after_ms: 1000 }),
        ];
        for (bytes, down) in cases {
            assert_eq!(Down::decode(&bytes), Ok(down.clone()), "{bytes:02x?}");
            assert_eq!(down.encode(), bytes, "the tests' server says it so");
        }
        let group = [&[0x80, 1, 1, 0, 1][..], &[0; 4 * 3 + 2], &0i64.to_be_bytes(), &[0, 0]].concat();
        assert!(matches!(Down::decode(&group), Ok(Down::Welcome(Welcome { closes_at: None, .. }))), "no closing time but a meeting room's");
    }

    #[test]
    fn frames_this_version_does_not_read_are_refused() {
        for (bytes, why) in [
            (vec![], "empty"),
            (vec![0x90], "an unknown tag"),
            (vec![0x81, 0x00], "cut short"),
            (vec![0x81, 0x00, 0x01, 0x00], "too long"),
            (vec![0x83, 0x00, 0x01, 0x02], "to an unknown someone"),
            (vec![0x85, 0x00, b'x'], "nothing kept, with a blob"),
            (vec![0x86, 0x09], "done for an unknown request"),
            ([&[0x80, 1, 4][..], &[0; 22]].concat(), "a welcome to an unknown kind of room"),
            (vec![0x87, 0x01, 0x0f, 0x00, 0, 0, 0, 0], "an unknown refusal code"),
            ([&[0x80, 2, 1][..], &[0; 22]].concat(), "a welcome of version 2"),
        ] {
            assert!(Down::decode(&bytes).is_err(), "{why}");
        }
    }

    #[test]
    fn close_codes_read_as_the_refusal_a_client_shows() {
        let all: Vec<(u16, Option<Refusal>)> = (4000..=4011).map(|c| (c, Code::of(c).and_then(Code::refusal))).collect();
        assert_eq!(all, [
            (4000, Some(Refusal::Invalid)),
            (4001, Some(Refusal::Version)),
            (4002, Some(Refusal::Invalid)),
            (4003, Some(Refusal::Limit)),
            (4004, None),
            (4005, Some(Refusal::Limit)),
            (4006, Some(Refusal::DailyWall)),
            (4007, Some(Refusal::Unavailable)),
            (4008, Some(Refusal::Limit)),
            (4009, Some(Refusal::Limit)),
            (4010, Some(Refusal::WrongSecret)),
            (4011, None),
        ]);
        assert_eq!(Code::of(1001), None, "a standard close is no refusal");
    }

    #[test]
    fn a_room_s_address_follows_the_server_s() {
        let room = RoomName::parse(&"A".repeat(43)).unwrap();
        let a = "A".repeat(43);
        assert_eq!(room_url("https://sync.norifm.com", &room, RoomKind::Group).unwrap(), format!("wss://sync.norifm.com/room/{a}?v=1&kind=group"));
        assert_eq!(room_url("http://127.0.0.1:8080/", &room, RoomKind::Meeting).unwrap(), format!("ws://127.0.0.1:8080/room/{a}?v=1&kind=meeting"));
        assert_eq!(room_url("https://example.com/nori/sync", &room, RoomKind::Group).unwrap(), format!("wss://example.com/nori/sync/room/{a}?v=1&kind=group"));
        assert_eq!(room_url("ftp://example.com", &room, RoomKind::Group), None);
    }
}
