use std::sync::atomic::AtomicU64;

use super::*;
use crate::wire::{read_up, Welcome};

const S: i64 = 1_000_000;
/// The Unix time when the tests' clock reads 0: 2023-11-14 22:13:20 UTC.
const UNIX: i64 = 1_700_000_000 * S;

/// What the link asked of the sockets.
#[derive(Debug, Clone, PartialEq)]
enum Asked {
    Open(SocketId, String),
    Send(SocketId, Vec<u8>),
    Ping(SocketId),
    Close(SocketId),
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Asked>>);

impl Recorded {
    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

impl Sockets for Recorded {
    fn open(&self, id: SocketId, url: String) {
        self.0.lock().unwrap().push(Asked::Open(id, url));
    }
    fn send(&self, id: SocketId, frame: Vec<u8>) {
        self.0.lock().unwrap().push(Asked::Send(id, frame));
    }
    fn ping(&self, id: SocketId) {
        self.0.lock().unwrap().push(Asked::Ping(id));
    }
    fn close(&self, id: SocketId) {
        self.0.lock().unwrap().push(Asked::Close(id));
    }
}

fn link() -> (Link, Arc<Recorded>) {
    let sockets = Arc::new(Recorded::default());
    (Link::new("https://sync.example".into(), sockets.clone(), Arc::new(AtomicU64::new(0)), 0, UNIX), sockets)
}

fn welcome(you: u16, members: Vec<u16>) -> Vec<u8> {
    Down::Welcome(Welcome { you, max_frame: 65_536, messages_per_minute: 240, bytes_per_minute: 4 << 20, max_members: 20, closes_at: None, members }).encode()
}

fn room() -> RoomName {
    RoomName::parse(&"g".repeat(43)).unwrap()
}

/// The socket the link last opened.
fn opened(asked: &[Asked]) -> SocketId {
    asked.iter().rev().find_map(|a| if let Asked::Open(id, _) = a { Some(*id) } else { None }).expect("a socket opened")
}

#[test]
fn a_room_is_a_socket_of_its_own_and_what_is_sent_before_its_welcome_waits_for_it() {
    let (link, sockets) = link();
    let room = room();
    link.join(&room, RoomKind::Group);
    link.send(&room, To::Everyone, b"hello".to_vec());
    link.get_copy(&room);
    let asked = sockets.take();
    assert_eq!(asked, [Asked::Open(0, format!("wss://sync.example/room/{}?v=1&kind=group", room.as_str()))], "nothing sent before the welcome");
    link.frame(0, &welcome(4, vec![2]));
    assert_eq!(link.heard(), [Heard::Joined { room: room.clone(), me: Member(4), members: vec![Member(2)] }]);
    assert_eq!(sockets.take(), [Asked::Send(0, Up::SendAll(b"hello").encode()), Asked::Send(0, Up::KeptGet.encode())]);
    link.send(&room, To::Member(Member(2)), b"you".to_vec());
    assert_eq!(sockets.take(), [Asked::Send(0, Up::SendTo(2, b"you").encode())], "open, it goes at once");
    for (down, heard) in [
        (Down::Joined(9), Heard::Arrived { room: room.clone(), member: Member(9) }),
        (Down::Message { from: 9, alone: true, payload: b"hi".to_vec() }, Heard::Message { room: room.clone(), from: Member(9), payload: b"hi".to_vec() }),
        (Down::Kept(Some(b"copy".to_vec())), Heard::Copy { room: room.clone(), blob: Some(b"copy".to_vec()) }),
        (Down::Refused { asked: crate::wire::Asked::KeptPut, code: Code::WrongSecret, retry_after_ms: 0 }, Heard::Refused { room: room.clone(), asked: room::Asked::PutCopy, reason: Refusal::WrongSecret }),
        (Down::Left(9), Heard::Left { room: room.clone(), member: Member(9) }),
    ] {
        link.frame(0, &down.encode());
        assert_eq!(link.heard(), [heard], "{down:?}");
    }
    for quiet in [Down::Done(crate::wire::Asked::KeptPut), Down::Refused { asked: crate::wire::Asked::SendTo, code: Code::NoMember, retry_after_ms: 0 }] {
        link.frame(0, &quiet.encode());
        assert_eq!(link.heard(), [], "{quiet:?} needs nothing: the member's leaving is told apart");
    }
    link.leave(&room);
    assert_eq!(sockets.take(), [Asked::Close(0)]);
    link.frame(0, &Down::Joined(3).encode());
    assert_eq!((link.heard(), link.room(&room)), (vec![], None), "a socket let go is heard no more");
}

#[test]
fn the_server_s_time_is_read_from_the_way_there_and_back() {
    let (link, sockets) = link();
    let room = room();
    link.join(&room, RoomKind::Meeting);
    link.ask_time(&room);
    link.at(300_000);
    link.frame(0, &welcome(1, vec![]));
    let sent = sockets.take().into_iter().find_map(|a| if let Asked::Send(_, f) = a { Some(f) } else { None }).unwrap();
    let Up::Time(t1) = read_up(&sent) else { panic!("a time request") };
    assert_eq!(t1, UNIX + 300_000, "stamped as it left, after the welcome");
    // The server's clock is 2 s ahead; the way there and back takes 40 ms.
    let server = t1 + 20_000 + 2 * S;
    link.at(340_000);
    link.frame(0, &Down::Time { t1, t2: server, t3: server }.encode());
    assert_eq!(link.heard().pop(), Some(Heard::Time { room: room.clone(), ms: (UNIX + 340_000 + 2 * S) / 1000, rtt_us: 40_000 }));
    link.ask_time(&room);
    let Some(Asked::Send(_, f)) = sockets.take().pop() else { panic!("sent at once") };
    let Up::Time(t1) = read_up(&f) else { panic!("a time request") };
    link.at(380_000);
    let server = t1 + 20_000 + 2 * S;
    link.frame(0, &Down::Time { t1, t2: server, t3: server }.encode());
    assert_eq!(link.heard(), [Heard::Time { room, ms: (UNIX + 380_000 + 2 * S) / 1000, rtt_us: 40_000 }]);
}

#[test]
fn a_dropped_connection_comes_back_after_a_random_short_wait_that_grows_while_connecting_fails() {
    let (link, sockets) = link();
    let room = room();
    link.join(&room, RoomKind::Group);
    link.frame(0, &welcome(1, vec![2, 3]));
    link.heard();
    link.ended(0, Ended::Dropped);
    sockets.take();
    assert_eq!(link.heard(), [Heard::Left { room: room.clone(), member: Member(2) }, Heard::Left { room: room.clone(), member: Member(3) }], "no refusal: a drop is quiet");
    let mut now = 0;
    for failures in 0..10 {
        let Some(RoomState::Waiting { until, refused: None }) = link.room(&room) else { panic!("waiting") };
        let (low, high) = ((RETRY_US.0 << failures.min(8)).min(RETRY_MAX_US), (RETRY_US.1 << failures.min(8)).min(RETRY_MAX_US));
        assert!((low..=high).contains(&(until - now)), "after {failures} failures: {} s", (until - now) / S);
        assert_eq!(link.due_at(), Some(until));
        link.at(until - 1);
        link.tick();
        assert_eq!(sockets.take(), [], "not before its time");
        now = until;
        link.at(now);
        link.tick();
        let id = opened(&sockets.take());
        link.ended(id, Ended::Dropped);
    }
    let id = {
        link.at(link.due_at().unwrap());
        link.tick();
        opened(&sockets.take())
    };
    link.frame(id, &welcome(7, vec![]));
    link.ended(id, Ended::Closed { code: 1001 });
    let Some(RoomState::Waiting { until, .. }) = link.room(&room) else { panic!("waiting") };
    assert!(until - link.now() <= RETRY_US.1, "a welcome starts the waits afresh");
}

#[test]
fn a_refusal_is_a_state_with_the_time_to_try_again() {
    let room = room();
    let wait = |code: u16| {
        let (link, _) = link();
        link.join(&room, RoomKind::Group);
        link.ended(0, Ended::Closed { code });
        let heard = link.heard();
        (link.room(&room), heard)
    };
    let (state, heard) = wait(4006);
    let Some(RoomState::Waiting { until, refused: Some(Refusal::DailyWall) }) = state else { panic!("{state:?}") };
    // The tests' clock starts at 22:13:20 UTC: midnight is 6,400 s later.
    assert!((6_400 * S..=6_400 * S + WALL_SPREAD_US).contains(&until), "after midnight UTC: {} s", until / S);
    assert_eq!(heard, [Heard::Refused { room: room.clone(), asked: room::Asked::Join, reason: Refusal::DailyWall }]);
    let Some(RoomState::Waiting { until, refused: Some(Refusal::Limit) }) = wait(4005).0 else { panic!("too many rooms") };
    assert!(until >= ROOMS_WAIT_US, "a minute at least");
    assert!(matches!(wait(4003).0, Some(RoomState::Waiting { refused: Some(Refusal::Limit), .. })), "a full room");
    assert_eq!(wait(4001).0, Some(RoomState::Stopped(Refusal::Version)), "not tried again");
    assert_eq!(wait(4002).0, Some(RoomState::Stopped(Refusal::Invalid)));
    let (state, heard) = wait(4004);
    assert_eq!((state, heard), (None, vec![Heard::Closed { room: room.clone() }]), "a meeting room's end");
    let (link, _) = link();
    link.join(&room, RoomKind::Group);
    link.ended(0, Ended::Status { status: 503 });
    assert!(matches!(link.room(&room), Some(RoomState::Waiting { refused: Some(Refusal::Unavailable), .. })), "the server could not run");
    assert_eq!(link.due_at().map(|d| d <= RETRY_US.1), Some(true));
}

#[test]
fn a_frame_this_version_cannot_read_stops_the_room() {
    let (link, sockets) = link();
    let room = room();
    link.join(&room, RoomKind::Group);
    link.frame(0, &welcome(1, vec![2]));
    link.heard();
    sockets.take();
    link.frame(0, &[0x99]);
    assert_eq!(link.heard(), [Heard::Left { room: room.clone(), member: Member(2) }, Heard::Refused { room: room.clone(), asked: room::Asked::Join, reason: Refusal::Invalid }]);
    assert_eq!((sockets.take(), link.room(&room), link.due_at()), (vec![Asked::Close(0)], Some(RoomState::Stopped(Refusal::Invalid)), None));
}

#[test]
fn a_keep_alive_goes_out_after_fifty_seconds_without_traffic() {
    let (link, sockets) = link();
    let room = room();
    link.join(&room, RoomKind::Group);
    assert_eq!(link.ping_at(), None, "nothing to keep up before the welcome");
    link.at(S);
    link.frame(0, &welcome(1, vec![]));
    sockets.take();
    assert_eq!(link.ping_at(), Some(S + PING_AFTER_US));
    link.at(30 * S);
    link.send(&room, To::Everyone, b"x".to_vec());
    assert_eq!(link.ping_at(), Some(30 * S + PING_AFTER_US), "traffic puts it off");
    link.at(79 * S);
    link.keep_alive();
    assert_eq!(sockets.take().len(), 1, "the frame sent before, only");
    link.at(80 * S);
    link.keep_alive();
    assert_eq!(sockets.take(), [Asked::Ping(0)]);
    assert_eq!(link.ping_at(), Some(130 * S));
    link.leave_all();
    assert_eq!(link.ping_at(), None);
}
