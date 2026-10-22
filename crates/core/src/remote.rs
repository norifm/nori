//! Remote control and jams over a client (nori-remote has the protocol). A [`Remote`] exists only while
//! remote control or jams are switched on, or the profile is a jam guest's; it polls the relay only
//! while it serves (this device is controllable), watches (a device picker or jam screen is open),
//! mirrors the account's active device or hosts a jam, and holds each poll up to [`HOLD_MS`], so a quiet
//! device wakes about once a minute. Nearby devices are reached through their LAN door instead of the relay.
//!
//! One device of the account plays at a time: the active one. This one is it until a transfer moves the
//! playback elsewhere ([`Remote::pick`], or another device's transfer); it then mirrors that device
//! ([`Remote::active`]) until playback comes back, a transfer moves it on, or the device goes. While it
//! mirrors, it keeps learning how the device's clock stands to its own (nori-remote's clock.rs): a burst
//! of time exchanges, then one every quarter minute, so the playhead shown is the one heard there.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::ops::Range;
use std::sync::Arc;
use futures_util::future::{select, Either};
use nori_tasks::{Notify, Tasks};
use nori_time::Duration;

use nori_remote::clock::{self, ClockSync};
use nori_remote::device::{admit, is_jam, Sender};
pub use nori_remote::device::{Controls, Reach};
use nori_remote::jam::{By, Jam};
use nori_remote::lan;
#[cfg(not(target_arch = "wasm32"))]
use nori_remote::lan::Door;
use nori_player::engine::Plan;
use nori_player::transitions::engine_plan;
use nori_player::types::TransitionPlan;
use nori_remote::wire::{Along, Answer, Body, DeviceKind, DeviceState, Entry, JamLyrics, JamMember, JamState, Member, Mix, Obeyed, Op, Outgoing, Pending, Refusal, Role, Room, HOLD_MS};
use parking_lot::Mutex;
use serde::Deserialize;

use crate::client::Client;
use crate::transport::{self, Exchange, NetError, Transport};
use crate::{api, db, Param, Song};
use nori_settings::settings_store::Settings;

/// The frames, for clients that speak them (the terminal and desktop host).
pub use nori_remote::wire;
/// The invite's QR code, for clients that draw it themselves.
pub use nori_remote::{is_guest_key, is_home_only, is_invite, parse_invite, position_now, qr_code, QrCode};

/// Songs before and after the current one a published state lists.
const ENTRIES_BEFORE: usize = 10;
const ENTRIES_AFTER: usize = 40;

/// Songs a controller asks for at a time beyond that window, and the most a device answers with.
const PAGE: u32 = 100;
const PAGE_MAX: u32 = 200;

/// Back this far into a song, a controller's previous restarts it rather than going to the song before
/// (as the player's own rule does, before the device's next state says what it did).
const PREVIOUS_RESTARTS_MS: i64 = 3_000;

/// After a failed poll, the relay is asked again this much later, twice as late after each further
/// failure up to [`RETRY_MAX_MS`] (or at once when the network is back).
const RETRY_MS: u64 = 30_000;
const RETRY_MAX_MS: u64 = 300_000;

/// The longest [`Remote::stop`] waits for the sends queued before it (leaving the relay among them).
#[cfg(not(target_arch = "wasm32"))]
const SENT_WAIT_MS: u64 = 3_000;

/// A held poll's own timeout: the hold plus the way there and back.
const POLL_TIMEOUT_MS: u32 = HOLD_MS + 15_000;

/// A published position this close to where the last one runs on to is not sent again.
const POSITION_SLACK_MS: i64 = 5;

/// The relay's clock learned this close to as last published is not sent again, µs.
const SERVER_SLACK_US: i64 = 1_000;

/// Time exchanges of a burst go out this far apart: more than most round trips through a relay, so an
/// answer does not wait behind the one before.
const BURST_GAP_MS: u64 = 250;

/// How long a command sent to the mirrored device shows as foreseen while the device does not say it
/// carried it out: an older device never says, and one that could not hear it falls back to its state.
const FORESEEN_US: i64 = 3_000_000;

/// What the platform's player does for the remote control.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait RemotePlayer: Send + Sync {
    /// Carries out an op the core admitted: never a jam op, a transfer or [`Op::Clear`] (the core sends
    /// that as one [`Op::Remove`] a song).
    fn apply(&self, op: Op);
}

/// Told whenever the devices, their states, the jam or a refusal changed; the platform reads them again.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait RemoteShown: Send + Sync {
    fn changed(&self);
    /// The account's library favorites changed on another device.
    fn marks_changed(&self);
    /// The jam this guest was in ended (its host ended it, or the relay no longer knows it), once: the
    /// platform leaves it as after [`Remote::jam_leave`]. `host`: its host's name, if seen.
    fn jam_ended(&self, host: Option<String>);
}

/// The platform's mDNS (NsdManager on Android, mdns-sd on the desktop).
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait Discovery: Send + Sync {
    /// Announces this device's door ([`nori_remote::lan::SERVICE`]), or withdraws it (None).
    fn announce(&self, door: Option<Announcement>);
    /// Looks for other doors while on, reporting them to [`Remote::lan_found`] and [`Remote::lan_lost`].
    fn browse(&self, on: bool);
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Announcement {
    /// The service instance name.
    pub name: String,
    pub port: u16,
    pub txt: Vec<Param>,
}

/// This device as others list it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RemoteMe {
    pub name: String,
    pub kind: DeviceKind,
}

/// The platform player's side of a published state; the queue's side is the core's.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Playing {
    pub playing: bool,
    /// Waiting for the song's bytes while it should play.
    pub buffering: bool,
    pub mixing: bool,
    /// The queue entry mixed out of, even after the incoming song becomes current.
    pub mix_from: Option<u32>,
    pub mix_position_ms: i64,
    pub mix_rate: f32,
    pub position_ms: i64,
    /// Song ms per real ms: the speed times a mix's tempo (the engine's `Status::pace`).
    pub rate: f32,
    /// The list index heard: the queue's own current song moves only once the player says it arrived.
    pub index: Option<u32>,
    /// The media volume, 0 to 100, when it can be set.
    pub volume: Option<u8>,
}

impl Default for Playing {
    fn default() -> Self {
        Playing { playing: false, buffering: false,
            mixing: false,
            mix_from: None,
            mix_position_ms: 0,
            mix_rate: 1.0, position_ms: 0, rate: 1.0, index: None, volume: None }
    }
}

/// Another device of the account, as a picker lists it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub state: Option<DeviceState>,
    /// How long ago `state` arrived (`nori_remote::position_now`).
    pub age_ms: i64,
    /// Reached on this network rather than through the relay.
    pub nearby: bool,
    /// Its answer to the last command sent to it, when it refused.
    pub refused: Option<Refusal>,
}

/// The account's active device while it is another one: what this device mirrors in its own player.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Mirror {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// The queue in play order, as far as it is known here: the whole of it once its pages are in.
    pub rows: Vec<MirrorRow>,
    /// The row playing.
    pub at: Option<u32>,
    /// Songs in the device's queue.
    pub len: u32,
    /// The queue's revision, for [`Op::Jump`], [`Op::Remove`] and [`Op::Move`].
    pub rev: u64,
    pub playing: bool,
    pub buffering: bool,
    pub mixing: bool,
    /// Where the song was at `at_us`; it runs on at `rate` from there while `playing`.
    pub position_ms: i64,
    /// Song ms per real ms there; zero while paused or waiting for audio.
    pub rate: f64,
    /// When the device's listener heard `position_ms`, on this device's clock (`nori_remote::clock::now_us`,
    /// Android's `SystemClock.elapsedRealtimeNanos` / 1000).
    pub at_us: i64,
    pub shuffle: bool,
    pub repeat: u8,
    /// The device's volume, 0 to 100, when it can be set.
    pub volume: Option<u8>,
    /// Its answer to the last command sent to it, when it refused.
    pub refused: Option<Refusal>,
}

impl Mirror {
    /// Where the song is at `now_us` on this device's clock, within the song.
    pub fn position_at(&self, now_us: i64) -> i64 {
        let pace = if self.playing { self.rate } else { 0.0 };
        let duration = self.at.and_then(|a| self.rows.get(a as usize)).map_or(0, |r| r.song.duration);
        nori_remote::run_on(self.position_ms, (now_us - self.at_us) as f64 / 1000.0, pace, duration)
    }

    /// Where the song is now: what the device's listener hears at this moment.
    pub fn position_now(&self) -> i64 {
        self.position_at(clock::now_us())
    }

    /// How long ago the device's listener heard `position_ms`, ms.
    pub fn heard_ago_ms(&self) -> i64 {
        (clock::now_us() - self.at_us) / 1000
    }

    /// `press` as a command for the device; None when there is nothing to ask.
    pub fn op(&self, press: Press) -> Option<Op> {
        Some(match press {
            Press::Play => Op::Play,
            Press::Pause => Op::Pause,
            Press::Toggle if self.playing => Op::Pause,
            Press::Toggle => Op::Play,
            Press::Next => Op::Next,
            Press::Previous => Op::Previous,
            Press::Seek { ms } => Op::Seek { ms: ms.max(0) },
            Press::Jump { index } => Op::Jump { index, rev: self.rev },
            Press::Remove { index } => Op::Remove { index, rev: self.rev },
            Press::Move { from, to } => Op::Move { from, to, rev: self.rev },
            Press::Clear => Op::Clear,
            Press::Shuffle { on } => Op::Shuffle { on },
            Press::Repeat { mode } => Op::Repeat { mode },
            Press::Volume { level } => {
                let percent = (level.clamp(0.0, 1.0) * 100.0).round() as u8;
                self.volume.filter(|&now| now != percent)?;
                Op::Volume { percent }
            }
        })
    }
}

/// [`Mirror::op`]: what a press of the player's controls asks of the mirrored device.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn mirror_op(mirror: Mirror, press: Press) -> Option<Op> {
    mirror.op(press)
}

/// What a client's player asks of the music.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Press {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek { ms: i64 },
    /// The song at this list index (a queue row's).
    Jump { index: u32 },
    Shuffle { on: bool },
    /// Off 0, one 1, all 2 (`nori_player::playlist`).
    Repeat { mode: u8 },
    /// 0 to 1.
    Volume { level: f32 },
    /// The song at this list index leaves the queue.
    Remove { index: u32 },
    /// The song at list index `from` moves to `to`.
    Move { from: u32, to: u32 },
    /// Everything after the song playing leaves the queue.
    Clear,
}

/// A song of a mirrored queue, and the list index commands name it by.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MirrorRow {
    pub index: u32,
    pub song: Song,
}

/// A jam this device hosts or is a guest in.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct JamView {
    pub hosting: bool,
    pub remote_songs: bool,
    /// The invite link (and QR code) while hosting.
    pub link: Option<String>,
    /// This device's member id.
    pub you: String,
    pub members: Vec<JamMember>,
    pub pending: Vec<Pending>,
    /// The host's playback and queue.
    pub queue: Option<DeviceState>,
    pub age_ms: i64,
    /// The host's answer to this guest's last request, when it refused.
    pub refused: Option<Refusal>,
    /// The host lets its guests listen along.
    pub along: bool,
    /// This guest's listening along.
    pub listening: Listening,
}

impl JamView {
    /// The jam's host's name.
    pub fn host(&self) -> &str {
        self.members.iter().find(|m| m.role == Role::Host).map_or("", |m| m.name.as_str())
    }

    /// Who listens: every member but the host.
    pub fn listeners(&self) -> impl Iterator<Item = &JamMember> {
        self.members.iter().filter(|m| m.role != Role::Host)
    }

    /// The requests this device shows: every one waiting while hosting; a guest's own, which wait for the
    /// host.
    pub fn asks(&self) -> impl Iterator<Item = &Pending> {
        self.pending.iter().filter(|p| self.hosting || p.from == self.you)
    }
}

/// Whether a jam guest plays the host's music along with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Listening {
    /// Only the jam is shown here.
    Watching,
    /// This device plays what the host plays, in step.
    Playing,
    /// Asked for, but the host does not let its guests now.
    HostOff,
    /// Asked for, but the server does not let jam guests stream.
    ServerOff,
}

/// Where starting a jam on this device stands ([`Remote::jam_start`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum JamStart {
    /// It can be started.
    Offered,
    /// Asked of the relay, not open yet.
    Starting,
    /// This device hosts one.
    Hosting,
    /// Jams are off, the server has no relay, or this is a guest's profile.
    Unavailable,
}

/// A jam guest's player controls: what each reaches by its role, and what its play button shows.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct JamControls {
    pub controls: Controls,

    /// Song actions request approval from the host.
    pub asks: bool,
    /// The play button shows playing: the jam's playback where play and pause reach the jam, else this
    /// device's.
    pub playing: bool,
    /// This guest paused its own listening while the jam plays on; play joins it again where it is.
    pub paused_here: bool,
}

/// The host's playback as a guest listening along plays it: the host's queue around its song, the place
/// heard there (`ms` at `at_us`, this device's clock) moving at `rate`, its speed and pitch, and the
/// transition out of the song playing as the host planned it.
#[derive(Debug, Clone, PartialEq)]
pub struct Lead {
    pub songs: Vec<Song>,
    pub index: usize,
    pub ms: f64,
    pub at_us: i64,
    /// The same moment on the host's own clock: it moves only with the host's words, not as its clock
    /// is learned here, so a jump there is told apart from a better reading of its clock.
    pub there_us: i64,
    pub rate: f64,
    pub playing: bool,
    pub speed: f32,
    pub pitch: f32,
    /// (outgoing song id, its plan).
    pub mix: Option<(String, Plan)>,
    pub mix_place: Option<(usize, f64, f64)>,
}

impl Lead {
    /// How long before now (this device's clock) `ms` was heard there.
    pub fn ago_us(&self) -> i64 {
        clock::now_us() - self.at_us
    }
}

/// What plays along with a jam's host on this device (the platform's player).
pub trait Follower: Send + Sync {
    /// The host's playback changed, or there is none to follow any more.
    fn lead(&self, lead: Option<Lead>);
}

/// What a guest's profile signs in with.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct JamPass {
    pub url: String,
    pub api_key: String,
}

/// Whether a link is a device pairing code, separate from a jam invite.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn pairing_valid(link: String) -> bool { lan::parse_pairing(&link).is_some() }

/// The temporary guest profile contains only a scoped room credential.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn jam_profile(pass: JamPass, id: String, name: String) -> crate::settings::SavedServer {
    let kind = if lan::guest_secret(&pass.api_key).is_some() { crate::settings::ServerKind::YouTubeMusic } else { crate::settings::ServerKind::Subsonic };
    crate::settings::SavedServer { id, name, kind, url: pass.url, api_key: pass.api_key, ..Default::default() }
}

/// The profile open before a jam was joined, opened again on leaving it (an app value).
const BEFORE_JAM: &str = "host.beforeJam";

/// A jam is being joined: the profile in use, an account's (not a jam's guest profile), is the one leaving
/// the jam returns to ([`jam_return`]).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn jam_joining(settings: &crate::settings_store::Settings) {
    let Some(prefs) = settings.current() else { return };
    if prefs.active_server().is_some_and(|s| !nori_remote::is_guest_key(&s.api_key)) {
        settings.keep_app_value(BEFORE_JAM, prefs.active_server_id.clone());
    }
}

/// `list` once the jam is left: its guest profiles gone, and the profile open before the jam active again,
/// else the one used last (the list is in the order they were opened), else none (the login).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn jam_return(settings: &crate::settings_store::Settings, list: crate::settings::ServerList) -> crate::settings::ServerList {
    let servers: Vec<_> = list.servers.into_iter().filter(|s| !nori_remote::is_guest_key(&s.api_key)).collect();
    let before = settings.app_value(BEFORE_JAM).filter(|id| servers.iter().any(|s| &s.id == id));
    let active_server_id = before.or_else(|| servers.last().map(|s| s.id.clone())).unwrap_or_default();
    crate::settings::ServerList { servers, active_server_id }
}

/// Where a command came from, and so how to answer it.
#[derive(Debug, Clone)]
enum Via {
    /// The relay; the room is a jam's (None: the account's).
    Relay(Option<String>),
    /// This device's door.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "a browser opens no door"))]
    Door,
    /// A member authenticated with this door’s scoped jam credential.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "a browser opens no door"))]
    LanJam,
    /// The door of a nearby device this one watches.
    Peer(String),
}

/// Whose clock the time keeper learns.
#[derive(Debug, Clone, PartialEq)]
enum Timed {
    /// Another device's, reached through the jam room `room` (None: the account's, or its door).
    Device { id: String, room: Option<String> },
    /// The relay's (`nori/time`).
    Server,
}

/// Where a send goes: the relay, or a nearby door's address.
#[derive(Debug, Clone)]
enum Link {
    Relay,
    Lan(String),
}

enum Out {
    Send(Link, Box<Outgoing>),
    /// A GET to the relay (leaving, closing a jam, sending a member out).
    Get(String),
}

impl Out {
    fn send(link: Link, outgoing: Outgoing) -> Out {
        Out::Send(link, Box::new(outgoing))
    }
}

/// What shows the mirrored device on this one, and so how closely it is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Sight {
    /// The app on screen: its playhead to the millisecond (the seek bar, lyrics), so its clock is learned.
    #[default]
    Screen,
    /// Only a notification: its song and whether it plays, not its clock.
    Notification,
    /// Nothing: a paused device is not followed (its poll ends) until it is in sight again.
    Nothing,
}

/// Whether the server relays remote control and jams (octo-fiesta's `noriRemote.*`), as asked when the
/// remote is made for a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RelaySupport {
    /// Not known yet (not asked, or the server was unreachable).
    #[default]
    Unknown,
    Supported,
    /// Plain Navidrome, or an octo-fiesta without the hub: nearby devices only, no jams. Asked again only
    /// when a device picker opens.
    Unsupported,
}

/// What a relay request's answer says of the relay; None when the network, not the server, answered.
fn support(got: &Result<Vec<u8>, NetError>) -> Option<RelaySupport> {
    match got {
        Ok(body) => Some(if answer(body).is_some() { RelaySupport::Supported } else { RelaySupport::Unsupported }),
        Err(NetError::Http { status: 404 | 405 } | NetError::Api { .. } | NetError::Parse { .. }) => Some(RelaySupport::Unsupported),
        Err(_) => None,
    }
}

struct Peer {
    library_marks: bool,
    service: String,
    /// Where its door was seen (mDNS tells each of the device's addresses), the one answering first.
    bases: Vec<String>,
    /// Polls failed in a row, each at the next address.
    failed: usize,
    member: Member,
    /// When its state last changed, on this device's clock.
    received: i64,
    since: Option<u64>,
    polling: bool,
}

struct Hosted {
    jam: Jam,
    link: String,
    /// Guests may listen along.
    along: bool,
}

/// A guest listening along: how the host's clock stands to this one's, and the lead last given.
#[derive(Default)]
struct Listen {
    clock: ClockSync,
    given: Option<Lead>,
}

/// What this device knows of the active device it mirrors.
struct Mirrored {
    id: String,
    /// Its state as it last arrived.
    heard: Option<DeviceState>,
    /// When `heard` arrived, on this device's clock.
    arrived: i64,
    /// That state with this device's own commands since then foreseen in it.
    shown: Option<DeviceState>,
    /// When `shown`'s position was right, on this device's clock: as it arrived, or was foreseen here.
    at: i64,
    /// The same moment on the device's clock, as it said (None: foreseen here, or an older device).
    device_at: Option<i64>,
    /// How the device's clock stands to this one's.
    clock: ClockSync,
    /// The queue by turn, at `shown`'s revision, as its pages arrive.
    pages: Vec<Option<Entry>>,
    /// The turn the page asked for and not answered yet starts at.
    asking: Option<u32>,
    /// Songs this device removed there, and the list index each had, for [`Remote::put_back`].
    taken: Vec<(Song, u32)>,
    /// This device's commands the device has not said it carried out yet, oldest first.
    foreseen: Vec<Foreseen>,
}

/// A command sent to the mirrored device, as it is expected to come out.
struct Foreseen {
    /// The command's id.
    id: u64,
    /// Deadline on this device's clock; accepted position changes wait for the player.
    until: Option<i64>,
    change: Change,
}

/// What a command changes in the mirrored device's state.
#[derive(Clone)]
enum Change {
    /// Plays or pauses at `ms`, as of `at` (this device's clock).
    Playing { playing: bool, ms: i64, at: i64 },
    /// Goes to list index `index` at `ms`, as of `at`.
    Place { index: Option<u32>, ms: i64, at: i64 },
    Shuffle(bool),
    Repeat(u8),
    Volume(u8),
    Star { id: String, on: bool },
}

impl Change {
    /// Whether the device's `state` shows what this changed. A device says it carried a command out in
    /// the next state it publishes, which may be before its player has played or paused.
    fn shown_in(&self, state: &DeviceState) -> bool {
        match *self {
            Change::Playing { playing, .. } => state.playing == playing,
            Change::Place { index, .. } => state.index == index,
            _ => true,
        }
    }
}

impl Mirrored {
    fn new(id: String, now: i64) -> Mirrored {
        Mirrored { id, heard: None, arrived: 0, shown: None, at: now, device_at: None, clock: ClockSync::default(), pages: Vec::new(), asking: None, taken: Vec::new(), foreseen: Vec::new() }
    }

    /// A state arrived from the device (`at`: when, on this device's clock). It ends the foresight of
    /// every command of this device's (`me`) it says it carried out and shows; the rest still shows over it. False
    /// when it is the one already heard.
    fn heard(&mut self, state: &DeviceState, at: i64, me: &str) -> bool {
        if let Some(last) = &self.heard {
            if state.instance == last.instance {
                if state.seq < last.seq || state == last {
                    return false;
                }
            } else {
                if state.instance < last.instance {
                    return false;
                }
                self.clock = ClockSync::default();
                self.foreseen.clear();
                self.pages.clear();
                self.asking = None;
                self.shown = None;
            }
        }
        if let Some(done) = state.obeyed.iter().find(|o| o.from == me) {
            let confirmed = self.foreseen.iter().filter(|f| f.id <= done.id && f.change.shown_in(state));
            let playing = confirmed.clone().filter(|f| matches!(f.change, Change::Playing { .. })).map(|f| f.id).max();
            let place = confirmed.filter(|f| matches!(f.change, Change::Place { .. })).map(|f| f.id).max();
            self.foreseen.retain(|f| f.id > done.id || match f.change {
                Change::Playing { .. } => playing.is_none_or(|id| f.id > id),
                Change::Place { .. } => place.is_none_or(|id| f.id > id),
                _ => !f.change.shown_in(state),
            });
            for f in self.foreseen.iter_mut().filter(|f| f.id <= done.id && matches!(f.change, Change::Place { .. })) {
                f.until = None;
            }
        }
        let rev = self.shown.as_ref().map(|s| (s.rev, s.len));
        if rev != Some((state.rev, state.len)) {
            self.pages = vec![None; state.len as usize];
            self.asking = None;
        }
        for e in &state.entries {
            if let Some(slot @ None) = self.pages.get_mut(e.turn as usize) {
                *slot = Some(e.clone());
            }
        }
        self.heard = Some(state.clone());
        self.arrived = at;
        self.show();
        true
    }

    /// `shown`: the state heard, with the commands foreseen over it.
    fn show(&mut self) {
        self.shown = self.heard.clone();
        self.at = self.arrived;
        self.device_at = self.heard.as_ref().and_then(|s| s.at_us);
        for change in self.foreseen.iter().map(|f| f.change.clone()).collect::<Vec<_>>() {
            self.put(&change);
        }
    }

    /// Drops a refused command or an unanswered command past its deadline; whether anything shown changed.
    fn forget(&mut self, id: Option<u64>, now: i64) -> bool {
        let before = self.foreseen.len();
        self.foreseen.retain(|f| Some(f.id) != id && f.until.is_none_or(|until| until > now));
        let changed = self.foreseen.len() != before;
        if changed {
            self.show();
        }
        changed
    }

    /// When the oldest unanswered command expires.
    fn foreseen_until(&self) -> Option<i64> {
        self.foreseen.iter().filter_map(|f| f.until).min()
    }

    fn put(&mut self, change: &Change) {
        let Some(st) = &mut self.shown else { return };
        let (index, ms, at) = match *change {
            Change::Playing { playing, ms, at } => {
                st.playing = playing;
                st.buffering &= playing;
                (st.index, ms, at)
            }
            Change::Place { index, ms, at } => (index, ms, at),
            Change::Shuffle(on) => return st.shuffle = on,
            Change::Repeat(mode) => return st.repeat = mode,
            Change::Volume(percent) => return st.volume = Some(percent),
            Change::Star { ref id, on } => {
                for e in st.entries.iter_mut().chain(self.pages.iter_mut().flatten()).filter(|e| e.id == *id) {
                    e.starred = on;
                }
                return;
            }
        };
        st.index = index;
        st.position_ms = ms;
        self.at = at;
        self.device_at = None;
    }

    /// Whether the device plays as shown here: its playhead runs on, so its clock matters.
    fn playing(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.playing)
    }

    /// When `shown`'s position was right, on this device's clock: the device's own word, once its clock
    /// is known here.
    fn shown_at(&self) -> i64 {
        match (self.device_at, self.clock.offset_at(self.at)) {
            (Some(there), Some(offset)) => there - offset,
            _ => self.at,
        }
    }

    /// A page of the queue arrived.
    fn page(&mut self, rev: u64, from: u32, entries: Vec<Entry>) {
        if self.shown.as_ref().map(|s| s.rev) != Some(rev) {
            return;
        }
        self.asking = None;
        for (k, e) in entries.into_iter().enumerate() {
            if let Some(slot) = self.pages.get_mut(from as usize + k) {
                *slot = Some(e);
            }
        }
    }

    /// The page to ask for next: where the first song not known yet is, unless one is being asked for.
    fn wanted(&self) -> Option<u32> {
        if self.asking.is_some() {
            return None;
        }
        self.pages.iter().position(Option::is_none).map(|p| p as u32)
    }

    /// The queue in play order: every page once all are in, the published window (newer) over them.
    fn rows(&self) -> Vec<Entry> {
        let Some(st) = &self.shown else { return Vec::new() };
        if self.pages.is_empty() || self.pages.iter().any(Option::is_none) {
            return st.entries.clone();
        }
        let mut all: Vec<Entry> = self.pages.iter().flatten().cloned().collect();
        for e in &st.entries {
            if let Some(slot) = all.get_mut(e.turn as usize).filter(|slot| slot.index == e.index) {
                *slot = e.clone();
            }
        }
        all
    }

    /// Song `id`'s star as [`Mirrored::rows`] has it, without copying them; None when it is not there.
    fn starred(&self, id: &str) -> Option<bool> {
        let st = self.shown.as_ref()?;
        if self.pages.is_empty() || self.pages.iter().any(Option::is_none) {
            return st.entries.iter().find(|e| e.id == id).map(|e| e.starred);
        }
        let newer = |turn: usize, page: &Entry| st.entries.iter().find(|e| e.turn as usize == turn && e.index == page.index);
        self.pages.iter().flatten().enumerate().map(|(turn, e)| newer(turn, e).unwrap_or(e)).find(|e| e.id == id).map(|e| e.starred)
    }

    /// Command `id`, `op`, as expected: until the player reports it, it is refused, or an unanswered
    /// command expires.
    fn foresee(&mut self, id: u64, op: &Op, here: i64) {
        let rows = self.rows();
        let at = self.shown_at();
        let Some(st) = &self.shown else { return };
        let now = nori_remote::position_now(st, (here - at) / 1000);
        let at = rows.iter().position(|e| Some(e.index) == st.index);
        let to = |k: Option<usize>| k.and_then(|k| rows.get(k)).map(|e| e.index);
        let place = |index: Option<u32>, ms: i64| Change::Place { index, ms, at: here };
        let change = match *op {
            Op::Play => Change::Playing { playing: true, ms: now, at: here },
            Op::Pause => Change::Playing { playing: false, ms: now, at: here },
            Op::Seek { ms } => place(st.index, ms),
            Op::Next => match to(at.map(|a| a + 1)).or_else(|| (st.repeat != 0).then(|| to(Some(0))).flatten()) {
                Some(i) => place(Some(i), 0),
                None => return,
            },
            Op::Previous if now > PREVIOUS_RESTARTS_MS => place(st.index, 0),
            Op::Previous => place(to(at.and_then(|a| a.checked_sub(1))).or(st.index), 0),
            Op::Jump { index, .. } => place(Some(index), 0),
            Op::Shuffle { on } => Change::Shuffle(on),
            Op::Repeat { mode } => Change::Repeat(mode),
            Op::Volume { percent } => Change::Volume(percent),
            Op::Star { ref id, on } => Change::Star { id: id.clone(), on },
            _ => return,
        };
        self.put(&change);
        self.foreseen.push(Foreseen { id, until: Some(here + FORESEEN_US), change });
    }

    fn view(&self, name: String, kind: DeviceKind, refused: Option<Refusal>) -> Option<Mirror> {
        let st = self.shown.as_ref()?;
        let rows = self.rows();
        Some(Mirror {
            id: self.id.clone(),
            name,
            kind,
            at: rows.iter().position(|e| Some(e.index) == st.index).map(|p| p as u32),
            rows: rows.iter().map(|e| MirrorRow { index: e.index, song: e.song() }).collect(),
            len: st.len,
            rev: st.rev,
            playing: st.playing,
            buffering: st.buffering,

            mixing: st.mixing,
            position_ms: st.position_ms,
            rate: nori_remote::pace(st),
            at_us: self.shown_at(),
            shuffle: st.shuffle,
            repeat: st.repeat,
            volume: st.volume,
            refused,
        })
    }
}

struct PendingTransfer {
    from: String,
    id: u64,
    rev: u64,
}

#[derive(Default)]
struct Inner {
    serving: bool,
    watching: bool,
    /// A foreground app is looking for playback already active on the account.
    discovering: bool,
    relay: RelaySupport,
    /// Bumped to end the running relay poller (each runs while its generation is current).
    generation: u64,
    relay_polling: bool,
    /// Times the relay was asked whether it relays, and those its answers covered: its poller runs until
    /// it answered the last.
    probes_asked: u64,
    probes_answered: u64,
    /// The last relay poll failed: the devices it listed may have gone since, and are not shown.
    relay_down: bool,
    /// What the relay's last answer said of it, as logged.
    relay_said: Option<String>,
    /// Bumped to end the nearby doors' pollers.
    lan_generation: u64,
    since: Option<u64>,
    you: String,
    rooms: Vec<Room>,
    /// When each other member's state last changed here, on this device's clock.
    received: HashMap<String, i64>,
    /// When each other device was last heard playing, on this device's clock.
    heard_playing: HashMap<String, i64>,
    /// When music last played here, on this device's clock.
    played_last: Option<i64>,
    peers: Vec<Peer>,
    hosted: Option<Hosted>,
    playing: Playing,
    /// When `playing` was said, on this device's clock: its position runs on from then.
    playing_at: Option<i64>,
    /// The device this one last handed its playback to, until it plays again.
    handed_to: Option<String>,
    /// The last command carried out from each controller, as published states say it.
    obeyed: Vec<Obeyed>,
    /// The account's active device, while it is another one.
    mirror: Option<Mirrored>,
    /// The queue asked for after this device took its controls back.
    taking: Option<PendingTransfer>,
    /// A task lets the mirrored device's foresights go as they end ([`Remote::end_foresight`]).
    ending_foresight: bool,
    /// Bumped to end the running time keeper (each runs while its generation is current).
    timing: u64,
    published: Option<(DeviceState, i64)>,
    /// The last published state's [`DeviceState::seq`].
    seq: u64,
    next_id: u64,
    refused: HashMap<String, Refusal>,
    #[cfg(not(target_arch = "wasm32"))]
    door: Option<Door>,
    /// What shows the mirrored device here ([`Remote::sight`]).
    sight: Sight,
    /// [`Remote::stop`] ran: nothing starts again.
    stopped: bool,
    /// The relay lets jam guests stream the host's queue ([`Answer::along`]).
    relay_along: bool,
    /// The relay tells its time ([`Answer::time`]), and how its clock stands to this one's.
    relay_time: bool,
    server_clock: ClockSync,
    /// This guest listens along.
    listen: Option<Listen>,
    /// The name of the host of the jam this guest is in, as last seen.
    jam_host: Option<String>,
    /// This guest's jam ended or was left: the relay is asked nothing more.
    jam_over: bool,
    /// A jam asked of the relay and not opened yet, by its attempt ([`Remote::jam_open`]); ending the jam
    /// takes it, and the jam it opens is closed again.
    opening: Option<u64>,
}

impl Peer {
    fn base(&self) -> String {
        self.bases[0].clone()
    }
}

impl Inner {
    fn browsing(&self) -> bool {
        self.watching || self.discovering || (self.serving && self.sight == Sight::Screen && !self.stopped)
    }

    fn probing(&self) -> bool {
        self.probes_asked != self.probes_answered
    }

    fn wants_relay(&self) -> bool {
        self.probing() || self.relay != RelaySupport::Unsupported && !self.jam_over && (self.serving || self.watching || self.discovering || self.taking.is_some() || self.follows_mirror() || self.hosted.is_some() || self.listen.is_some())
    }

    /// Whether the mirrored device is followed: while something here shows it, or it plays.
    fn follows_mirror(&self) -> bool {
        self.mirror.as_ref().is_some_and(|m| self.sight != Sight::Nothing || m.playing())
    }

    /// Whether the door of nearby device `id` is followed: every one while a picker is open, else only
    /// the one mirrored.
    fn follows_peer(&self, id: &str) -> bool {
        self.watching || self.discovering || self.taking.as_ref().is_some_and(|t| t.from == id) || (self.follows_mirror() && self.mirror.as_ref().is_some_and(|m| m.id == id))
    }

    /// Ends the relay poller; its held poll is given up once [`Remote::cancel`] is notified.
    fn end_relay_poll(&mut self) {
        self.generation += 1;
        self.relay_polling = false;
    }

    /// Device `id`'s last state and when it arrived; a nearby device's door says it first.
    fn state_of(&self, id: &str) -> Option<(&DeviceState, Option<i64>)> {
        if let Some(p) = self.peers.iter().find(|p| p.member.id == id) {
            return p.member.state.as_ref().map(|s| (s, Some(p.received)));
        }
        let m = self.rooms.iter().filter(|r| !r.jam).flat_map(|r| &r.members).find(|m| m.id == id)?;
        m.state.as_ref().map(|s| (s, self.received.get(id).copied()))
    }

    /// Whether device `id` is still there to be followed, as [`Remote::devices`] lists it: heard from nearby,
    /// or through the relay while it answers.
    fn listed(&self, id: &str) -> bool {
        let far = self.rooms.iter().filter(|r| !r.jam && !self.relay_down).flat_map(|r| &r.members);
        self.peers.iter().map(|p| &p.member).chain(far).any(|m| m.id == id && m.state.is_some())
    }

    /// Whether device `id` plays, as last heard.
    fn plays(&self, id: &str) -> bool {
        self.state_of(id).is_some_and(|(s, _)| s.playing)
    }

    /// Whether device `id`, picked while this one is the active device, is the active one rather than
    /// this one: this one does not play, and it plays, or it has a song and was heard playing since this
    /// one last played (or neither played since this one started).
    fn leads(&self, id: &str) -> bool {
        if self.playing.playing {
            return false;
        }
        let has_song = self.state_of(id).is_some_and(|(s, _)| s.index.is_some());
        self.plays(id) || (has_song && self.heard_playing.get(id).copied() >= self.played_last)
    }

    /// Device `id` as it is listed.
    fn member(&self, id: &str) -> Option<&Member> {
        self.peers.iter().map(|p| &p.member).chain(self.rooms.iter().filter(|r| !r.jam).flat_map(|r| &r.members)).find(|m| m.id == id)
    }

    /// Device `id`'s name and kind, as it is listed.
    fn who(&self, id: &str) -> Option<(String, DeviceKind)> {
        self.member(id).map(|m| (m.name.clone(), m.kind))
    }

    fn jam_room(&self) -> Option<&str> {
        self.hosted.as_ref().map(|h| h.jam.room.as_str())
    }

    /// The jam this device is a guest in, and its host.
    fn joined(&self) -> Option<(&Room, Option<&Member>)> {
        let room = self.rooms.iter().find(|r| r.jam && Some(r.room.as_str()) != self.jam_room())?;
        Some((room, room.members.iter().find(|m| m.state.as_ref().is_some_and(|s| s.jam.is_some()))))
    }

    /// Whose clock is learned now: the mirrored device's; the relay's while hosting a jam whose guests
    /// listen along, or listening along to a host that knows it; else that host's, through the jam's room.
    fn timed(&self) -> Option<Timed> {
        if let Some(m) = &self.mirror {
            return Some(Timed::Device { id: m.id.clone(), room: None });
        }
        if self.hosts_along() {
            return self.relay_time.then_some(Timed::Server);
        }
        self.listen.as_ref()?;
        let (room, host) = self.joined()?;
        let host = host?;
        if self.relay_time && host.state.as_ref().is_some_and(|s| s.server_us.is_some()) {
            return Some(Timed::Server);
        }
        Some(Timed::Device { id: host.id.clone(), room: Some(room.room.clone()) })
    }

    /// Whether another clock is to be learned now: a guest listening along plays by its host's, the
    /// screen on or off; a host by the relay's for its guests.
    fn wants_time(&self) -> bool {
        self.listen.is_some() || (self.sight == Sight::Screen && self.mirror.as_ref().is_some_and(Mirrored::playing)) || (self.hosts_along() && self.relay_time)
    }

    /// A jam is hosted here or being started: it plays on this device only.
    fn jams(&self) -> bool {
        self.hosted.is_some() || self.opening.is_some()
    }

    /// This device hosts a jam whose guests may listen along.
    fn hosts_along(&self) -> bool {
        self.hosted.as_ref().is_some_and(|h| h.along)
    }

    /// Where this guest's listening along stands.
    fn listening(&self) -> Listening {
        let host_lets = self.joined().and_then(|(_, h)| h?.state.as_ref()?.jam.as_ref()?.along.as_ref()).is_some();
        match &self.listen {
            None => Listening::Watching,
            Some(_) if !self.relay_along => Listening::ServerOff,
            Some(_) if !host_lets => Listening::HostOff,
            Some(_) => Listening::Playing,
        }
    }

    /// This guest's player controls by its role in the jam it is in; None while hosting or in no jam.
    fn jam_controls(&self) -> Option<JamControls> {
        if self.hosted.is_some() {
            return None;
        }
        let st = self.joined()?.1?.state.as_ref()?;
        let role = st.jam.as_ref()?.members.iter().find(|m| m.id == self.you).map_or(Role::Guest, |m| m.role);
        let listening = self.listening() == Listening::Playing;
        let controls = Controls::of(role, listening);
        let (there, here) = (st.playing, self.playing.playing);
        let paused_here = listening && there && !here;
        let playing = !paused_here && if controls.play_pause == Reach::Jam { there } else { here };
        Some(JamControls { controls,
            asks: role == Role::Guest,
            playing, paused_here })
    }

    /// When the jam's host heard `st`'s place, on this device's clock, once its clock is known here
    /// (listening along): through the relay's clock when both know it, else as learned from the host.
    fn heard_at(&self, st: &DeviceState, now: i64) -> Option<i64> {
        let l = self.listen.as_ref()?;
        let server = st.server_us.zip(self.server_clock.settled_offset_at(now)).map(|(host, here)| host - here);
        Some(st.at_us? + server.or_else(|| l.clock.settled_offset_at(now).map(|o| -o))?)
    }

    /// The host's playback as this guest plays along with it; None while it should not, or the host's
    /// clock is not known yet.
    fn lead(&self, now: i64) -> Option<Lead> {
        self.listen.as_ref().filter(|_| self.listening() == Listening::Playing)?;
        let (_, host) = self.joined()?;
        let st = host?.state.as_ref()?;
        let along = st.jam.as_ref()?.along.as_ref()?;
        let at = self.heard_at(st, now)?;
        let index = st.entries.iter().position(|e| Some(e.index) == st.index)?;
        let mix = along.mix.as_ref().and_then(|m| plan_of(m, &st.entries));

        let mix_place = mix.as_ref().and(along.mix.as_ref()).and_then(|m| {
            Some((
                st.entries.iter().position(|e| e.index == m.from)?,
                m.position_ms? as f64,
                m.rate? as f64,
            ))
        });
        Some(Lead {
            songs: st.entries.iter().map(Entry::song).collect(),
            index,
            ms: st.position_ms as f64,
            at_us: at,
            there_us: st.at_us?,
            rate: nori_remote::rate(st),
            playing: st.playing && !st.buffering,
            speed: along.speed,
            pitch: along.pitch,
            mix,
            mix_place,
        })
    }

    fn age(&self, id: &str, now: i64) -> i64 {
        self.received.get(id).map_or(0, |t| (now - t) / 1000)
    }

    fn since_played_ms(&self, now_us: i64) -> i64 {
        self.playing_at.map_or(0, |t| (now_us - t) / 1000)
    }

    /// Where this device's song is now, run on from when the platform last said.
    fn position_now(&self, now: i64) -> i64 {
        let elapsed = self.since_played_ms(now);
        if self.playing.playing { self.playing.position_ms + (elapsed as f64 * self.playing.rate as f64) as i64 } else { self.playing.position_ms }
    }

    /// Where this device's song is at `now_us`, held to the end of `state`'s song.
    fn position_at_of(&self, state: &DeviceState, now_us: i64) -> i64 {
        nori_remote::position_now(&DeviceState { position_ms: self.playing.position_ms, ..state.clone() }, self.since_played_ms(now_us))
    }
}

/// Remote control and jams for one client. See the module.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Remote {
    client: Arc<Client>,
    id: String,
    /// Keeps `id` this remote's until it stops.
    id_lock: Mutex<IdLock>,
    instance: Option<u64>,
    me: RemoteMe,
    player: Arc<dyn RemotePlayer>,
    shown: Arc<dyn RemoteShown>,
    discovery: Option<Arc<dyn Discovery>>,
    inner: Mutex<Inner>,
    /// Where the pollers, the time keeper and the sender run.
    tasks: Arc<dyn Tasks>,
    /// Wakes a relay poller waiting to try again.
    retry: Notify,
    /// Wakes the time keeper and the foresights' end when what they wait for may have changed.
    timing: Notify,
    /// Wakes the requests waiting for an answer to check they are still wanted.
    cancel: Notify,
    /// None once stopped: the sender ends with what was queued.
    out: Mutex<Option<Outbox>>,
    /// The tasks [`Remote::stop`] waits for.
    running: Arc<Running>,
    /// What plays along with a jam's host here.
    follower: Mutex<Option<Arc<dyn Follower>>>,
}

/// The app's key for the one id every client over an app database had before each kind of client had
/// its own: the first account client to claim an id takes it over.
const SHARED_DEVICE_ID: &str = "remoteDeviceId";

/// The lock file a remote holds for its device id while it runs (none in a browser).
#[cfg(not(target_arch = "wasm32"))]
type IdLock = Option<std::fs::File>;
#[cfg(target_arch = "wasm32")]
type IdLock = Option<()>;

/// `kind` as the wire names it ("desktop").
fn kind_name(kind: DeviceKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Device ids are kept in the app database, one for each copy of a kind of client running at once.
/// The id of the first copy of `kind` over `settings`' app database that no running remote holds
/// (`remoteDeviceId.desktop.0`, `.1`, ...), made the first time, and its lock. A fresh id while the
/// database is not open; the first copy's when no lock file can be made.
fn claim_device_id(settings: &Settings, kind: DeviceKind) -> (String, IdLock) {
    let Some(db) = settings.app_db() else { return (nori_remote::new_id(), None) };
    let name = kind_name(kind);
    let path = db.lock().path().filter(|p| !p.is_empty()).map(str::to_string);
    let (key, lock) = (0..)
        .map(|copy| format!("{SHARED_DEVICE_ID}.{name}.{copy}"))
        .find_map(|key| id_lock(path.as_deref(), &key).map(|lock| (key, lock)))
        .expect("a copy no remote holds");
    let id = kept_device_id(&db.lock(), &key, kind != DeviceKind::Guest).unwrap_or_else(|e| {
        crate::alog::info(&format!("remote: could not keep the device id: {e}"));
        nori_remote::new_id()
    });
    (id, lock)
}

/// The lock of id `key` next to the app database at `db`; None while another remote holds it.
#[cfg(not(target_arch = "wasm32"))]
fn id_lock(db: Option<&str>, key: &str) -> Option<IdLock> {
    let Some(db) = db else { return Some(None) };
    let opened = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(format!("{db}.{key}.lock"));
    let locked = opened.and_then(|file| match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    });
    match locked {
        Ok(None) => None,
        Ok(lock) => Some(lock),
        Err(e) => {
            crate::alog::info(&format!("remote: no lock for the device id: {e}"));
            Some(None)
        }
    }
}

/// A browser runs one copy of a client over its database.
#[cfg(target_arch = "wasm32")]
fn id_lock(_: Option<&str>, _: &str) -> Option<IdLock> {
    Some(None)
}

/// The id kept at `key`: made the first time, or the shared one taken over when `shared` allows.
fn kept_device_id(c: &rusqlite::Connection, key: &str, shared: bool) -> rusqlite::Result<String> {
    use rusqlite::OptionalExtension;
    let tx = c.unchecked_transaction()?;
    if let Some(id) = tx.query_row("SELECT value FROM app_kv WHERE key=?1 AND value<>''", [key], |r| r.get::<_, String>(0)).optional()? {
        return Ok(id);
    }
    let taken = if shared { tx.query_row("DELETE FROM app_kv WHERE key=?1 AND value<>'' RETURNING value", [SHARED_DEVICE_ID], |r| r.get::<_, String>(0)).optional()? } else { None };
    let id = taken.unwrap_or_else(nori_remote::new_id);
    tx.execute("INSERT OR REPLACE INTO app_kv(key, value) VALUES(?1, ?2)", [key, id.as_str()])?;
    tx.commit()?;
    Ok(id)
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Remote {
    /// A remote whose work runs on a thread of its own (in a browser, on the page's event loop).
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new(client: Arc<Client>, me: RemoteMe, player: Arc<dyn RemotePlayer>, shown: Arc<dyn RemoteShown>, discovery: Option<Arc<dyn Discovery>>) -> Arc<Remote> {
        #[cfg(not(target_arch = "wasm32"))]
        let tasks = Arc::new(nori_tasks::Native::new("nori-remote").expect("a thread for the remote"));
        #[cfg(target_arch = "wasm32")]
        let tasks = Arc::new(nori_tasks::Local);
        Remote::with_tasks(client, me, player, shown, discovery, tasks)
    }

    /// The QR/code used to pair this device's YouTube profiles with its other devices.
    pub fn pairing_link(&self) -> Option<String> {
        if self.me.kind == DeviceKind::Guest || self.client.core.kind() != crate::settings::ServerKind::YouTubeMusic { return None; }
        lan_secret(&self.client).map(|secret| lan::pairing_link(&secret))
    }

    /// Accepts a pairing code explicitly chosen by the user, then reannounces the door.
    pub fn pair(self: Arc<Self>, link: String) -> bool {
        let Some(secret) = lan::parse_pairing(&link) else { return false };
        if self.pairing_link().is_none() { return false; }
        if !keep_pairing(&self.client.core.session.settings, &secret) { return false; }
        let announce = {
            let mut i = self.inner.lock();
            i.peers.clear();
            i.lan_generation += 1;
            i.mirror = None;
            i.taking = None;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(door) = &i.door { door.pair_secret(lan_secret(&self.client).expect("pairing secret")); }
            #[cfg(not(target_arch = "wasm32"))]
            let announce = i.door.as_ref().map(|door| self.announcement(door.port()));
            #[cfg(target_arch = "wasm32")]
            let announce = None;
            announce
        };
        if let Some(discovery) = &self.discovery { discovery.announce(announce); }
        self.shown.changed();
        true
    }

    /// Forgets all paired devices by rotating this device's pairing credential.
    pub fn reset_pairing(self: Arc<Self>) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(secret) = lan::new_secret() { return self.pair(lan::pairing_link(&secret)); }
        false
    }

    /// Current library favorites, including changes received from nearby devices.
    pub fn star_marks(&self) -> crate::stars::StarMarks {
        self.client.core.stars.lock().clone()
    }

    /// Whether the server relays remote control and jams; the screens offer jams and far devices only then.
    pub fn relay(&self) -> RelaySupport {
        self.inner.lock().relay
    }

    /// This device's id, as other devices and the relay know it.
    pub fn id(&self) -> String {
        self.id.clone()
    }

    /// This device as others list it.
    pub fn me(&self) -> RemoteMe {
        self.me.clone()
    }

    /// Makes this device controllable (its playback service is up) or not.
    pub fn serve(self: Arc<Self>, on: bool) {
        let (announce, browse) = {
            let mut i = self.inner.lock();
            if i.serving == on {
                return;
            }
            let before = i.browsing();
            i.serving = on;
            i.published = None;
            // A held poll saying this device serves says so again as it ends: it is asked anew.
            if !on && i.relay_polling {
                i.end_relay_poll();
                self.cancel.notify();
            }
            (self.reopen_door(&mut i, on), (before != i.browsing()).then_some(i.browsing()))
        };
        if let Some(d) = &self.discovery {
            d.announce(announce);
            if let Some(on) = browse { d.browse(on); }
        }
        if on {
            self.publish();
        } else {
            self.leave();
        }
        self.keep_polling();
    }

    /// A device picker or jam screen is open: other devices' states are followed while on. Opened while
    /// the server was found without the relay, it is asked again (it may have gained one since).
    pub fn watch(self: Arc<Self>, on: bool) {
        let (probe, browse) = {
            let mut i = self.inner.lock();
            if i.watching == on {
                return;
            }
            let before = i.browsing();
            i.watching = on;
            let browse = i.browsing();
            if !i.watching && !i.discovering {
                i.lan_generation += 1;
                i.peers.iter_mut().for_each(|p| p.polling = false);
            }
            (on && i.relay == RelaySupport::Unsupported, (before != browse).then_some(browse))
        };
        if let (Some(d), Some(on)) = (&self.discovery, browse) {
            d.browse(on);
        }
        if probe {
            self.probe();
        }
        self.keep_polling();
    }

    /// Looks for the account's playing device while this app is in front.
    pub fn discover_active(self: Arc<Self>, on: bool) {
        let browse = {
            let mut i = self.inner.lock();
            let on = on && self.me.kind != DeviceKind::Guest && !i.stopped && i.sight == Sight::Screen && i.mirror.is_none() && i.taking.is_none() && !i.playing.playing && !i.jams();
            if i.discovering == on {
                return;
            }
            let before = i.browsing();
            i.discovering = on;
            if on {
                i.end_relay_poll();
                self.cancel.notify();
            }
            let browse = i.browsing();
            if !i.watching && !i.discovering {
                i.lan_generation += 1;
                i.peers.iter_mut().for_each(|p| p.polling = false);
            }
            (before != browse).then_some(browse)
        };
        if let (Some(d), Some(on)) = (&self.discovery, browse) {
            d.browse(on);
        }
        self.follow_active();
        self.keep_polling();
    }

    /// What shows the mirrored device here; [`Sight::Screen`] until said otherwise.
    pub fn sight(self: Arc<Self>, sight: Sight) {
        let browse = {
            let mut i = self.inner.lock();
            if i.sight == sight {
                return;
            }
            let before = i.browsing();
            i.sight = sight;
            (before != i.browsing()).then_some(i.browsing())
        };
        if sight != Sight::Screen {
            self.clone().discover_active(false);
        }
        if let (Some(d), Some(on)) = (&self.discovery, browse) {
            d.browse(on);
        }
        self.timing.notify();
        self.keep_polling();
    }

    /// The platform's playback changed (play, pause, seek, another song, the queue, the volume). Music
    /// starting here makes this the active device again.
    pub fn played(self: Arc<Self>, playing: Playing) {

        // The report belongs to call entry, before waiting for the session lock.
        let now = self.now();
        let (publish, started, shown, cancelled) = {
            let mut i = self.inner.lock();
            let started = playing.playing && !i.playing.playing;
            // A guest listening along paused or played here: its controls say so.
            let shown = i.listen.is_some() && playing.playing != i.playing.playing;
            if started {
                i.handed_to = None;
            }

            if playing.playing || i.playing.playing {
                i.played_last = Some(now);
            }
            let moved_here = started || (!i.playing.playing && (playing.index != i.playing.index || (playing.position_ms - i.playing.position_ms).abs() > POSITION_SLACK_MS));
            let cancelled = moved_here && i.taking.take().is_some();
            i.playing = playing;
            i.playing_at = Some(now);
            (i.serving || i.hosted.is_some(), started && i.mirror.is_some(), shown, cancelled)
        };
        if playing.playing {
            self.clone().discover_active(false);
        }
        if started {
            self.set_active(None);
        }
        if publish {
            self.publish();
        }
        if cancelled {
            self.keep_polling();
        }
        if shown {
            self.shown.changed();
        }
    }

    /// The device is on a network again: a poll that failed is asked again now, not after its wait.
    pub fn network_back(&self) {
        self.retry.notify();
    }

    /// The device's volume moved by itself (its own keys), 0 to 100; None when it cannot be set.
    pub fn volume_changed(self: Arc<Self>, volume: Option<u8>) {
        let publish = {
            let mut i = self.inner.lock();
            if i.playing.volume == volume {
                return;
            }
            i.playing.volume = volume;
            i.serving || i.hosted.is_some()
        };
        if publish {
            self.publish();
        }
    }

    /// The account's active device while it is another one, as this device mirrors it; None while this
    /// one is the active device.
    pub fn active(&self) -> Option<Mirror> {
        let i = self.inner.lock();
        let m = i.mirror.as_ref()?;
        let (name, kind) = i.who(&m.id)?;
        m.view(name, kind, i.refused.get(&m.id).copied())
    }

    /// Whether [`Remote::active`] has a device, without building its mirror.
    pub fn mirroring(&self) -> bool {
        let i = self.inner.lock();
        i.mirror.as_ref().is_some_and(|m| m.shown.is_some() && i.member(&m.id).is_some())
    }

    /// Moves the playback to `device`, or here (None): the active device hands its queue over and
    /// pauses. A device that is the active one already (it plays, or played since this one did) is only
    /// followed, as is any device while nothing is queued here.
    pub fn pick(self: Arc<Self>, device: Option<String>) {
        self.clone().discover_active(false);
        let cancelled = self.inner.lock().taking.take().is_some();
        let (active, idle, plays, leads) = {
            let i = self.inner.lock();
            if device.is_some() && i.jams() {
                return;
            }
            let m = i.mirror.as_ref();
            let (plays, leads) = device.as_deref().map_or((false, false), |d| (i.plays(d), i.leads(d)));
            (m.map(|m| m.id.clone()), m.and_then(|m| m.shown.as_ref()).is_some_and(|s| s.index.is_none()), plays, leads)
        };
        let here = self.client.core.session.playlist(|p| p.current().is_some());
        match (active, device) {
            (None, None) => {}
            (Some(_), None) if idle => self.set_active(None),
            (Some(a), None) => self.pull(a),
            (None, Some(d)) if here && !leads => self.clone().hand_over(d),
            (None, Some(d)) => self.set_active(Some(d)),
            (Some(a), Some(d)) if a == d => {}
            (Some(_), Some(d)) if plays => self.set_active(Some(d)),
            (Some(a), Some(d)) => {
                self.clone().send(a, Op::Transfer { to: d.clone(), listen_handoff: false });
                self.set_active(Some(d));
            }
        }
        if cancelled {
            self.keep_polling();
        }
    }

    /// The account's other devices, nearby ones first; through the relay only while it answers.
    pub fn devices(&self) -> Vec<RemoteDevice> {
        let now = self.now();
        let i = self.inner.lock();
        let seen = |m: &Member, age_ms: i64, nearby: bool| RemoteDevice { id: m.id.clone(), name: m.name.clone(), kind: m.kind, state: m.state.clone(), age_ms, nearby, refused: i.refused.get(&m.id).copied() };
        let mut out: Vec<RemoteDevice> = i.peers.iter().filter(|p| p.member.state.is_some()).map(|p| seen(&p.member, (now - p.received) / 1000, true)).collect();
        for room in i.rooms.iter().filter(|r| !r.jam && !i.relay_down) {
            for m in room.members.iter().filter(|m| m.state.is_some()) {
                if !out.iter().any(|d| d.id == m.id) {
                    out.push(seen(m, i.age(&m.id, now), false));
                }
            }
        }
        out
    }

    /// Sends `op` to device `device`; its answer shows in [`Remote::devices`]. Sent to the mirrored
    /// device, it shows in [`Remote::active`] at once, as it is expected to come out.
    pub fn send(self: Arc<Self>, device: String, op: Op) {
        if matches!(&op, Op::Transfer { to, .. } if to == &self.id) {
            self.pull(device);
            return;
        }
        let id = self.next_id();
        let mut foreseen = false;
        let link = {
            let mut i = self.inner.lock();
            if let Some(m) = i.mirror.as_mut().filter(|m| m.id == device) {
                if let Op::Remove { index, .. } = op {
                    if let Some(e) = m.rows().into_iter().find(|e| e.index == index) {
                        m.taken.push((e.song(), index));
                    }
                }
                m.foresee(id, &op, self.now());
                // A play foreseen starts the time keeper, and a foresight ends on time.
                self.timing.notify();
                foreseen = true;
            }
            i.refused.remove(&device);
            let lan = i.peers.iter().find(|p| p.member.id == device).map(|p| Link::Lan(p.base()));
            match lan {
                Some(l) => l,
                None if i.relay == RelaySupport::Unsupported => return,
                None => Link::Relay,
            }
        };
        if foreseen {
            self.end_foresight();
        }
        self.out(Out::send(link, Outgoing { to: Some(device), body: Some(Body::Command { id, op: Box::new(op) }), ..Default::default() }));
        self.shown.changed();
    }

    /// Undoes the removal of song `id` from the mirrored device `device`'s queue: it goes back where it
    /// was there ([`Op::Restore`]). False when this device did not take it out.
    pub fn put_back(self: Arc<Self>, device: String, id: String) -> bool {
        let taken = {
            let mut i = self.inner.lock();
            let Some(m) = i.mirror.as_mut().filter(|m| m.id == device) else { return false };
            let Some(at) = m.taken.iter().rposition(|(s, _)| s.id == id) else { return false };
            m.taken.remove(at)
        };
        let (song, index) = taken;
        self.send(device, Op::Restore { song, index });
        true
    }

    /// A heart pressed for song `id` while another device plays: sent there when its queue has the song
    /// (that device stars it on the server and shows it), so the server hears it once. False when this
    /// device stars it itself: nothing is mirrored, or the song is not in that queue.
    pub fn star_where_playing(self: Arc<Self>, id: String, on: bool) -> bool {
        let device = self.inner.lock().mirror.as_ref().filter(|m| m.rows().iter().any(|e| e.id == id)).map(|m| m.id.clone());
        let Some(device) = device else { return false };
        self.send(device, Op::Star { id, on });
        true
    }

    /// A door the platform's discovery found: service `service` at `host`:`port` with TXT `txt`.
    pub fn lan_found(self: Arc<Self>, service: String, host: String, port: u16, txt: Vec<Param>) {
        let get = |k: &str| txt.iter().find(|p| p.key == k).map(|p| p.value.clone()).unwrap_or_default();
        let Some(tag) = self.account_tag() else { return };
        let id = get("id");
        if get("acct") != tag || id.is_empty() || id == self.id {
            return;
        }
        let kind = serde_json::from_value(serde_json::Value::String(get("kind"))).unwrap_or_default();
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
        let base = format!("http://{host}:{port}");
        {
            let mut i = self.inner.lock();
            // Seen again, maybe at another of its addresses: its poller goes on, and tries that one too.
            if let Some(p) = i.peers.iter_mut().find(|p| p.service == service && p.member.id == id) {
                p.library_marks = get("marks") == "1";
                if !p.bases.contains(&base) {
                    p.bases.push(base);
                }
                return;
            }
            i.peers.retain(|p| p.service != service && p.member.id != id);
            let member = Member { id, name: get("name"), kind, state: None };
            i.peers.push(Peer { library_marks: get("marks") == "1", service, bases: vec![base], failed: 0, member, received: self.now(), since: None, polling: false });
        }
        self.keep_polling();
    }

    /// A door went away.
    pub fn lan_lost(self: Arc<Self>, service: String) {
        self.inner.lock().peers.retain(|p| p.service != service);
        self.follow_active();
        self.shown.changed();
    }

    /// Starts hosting a jam; its invite link. Needs the relay. None when there is nothing to start: a
    /// jam is hosted or being started already, or it was ended before the relay opened it (that one is
    /// closed again at once).
    pub async fn jam_open(self: Arc<Self>) -> Result<Option<String>, NetError> {
        #[derive(Deserialize)]
        struct Opened {
            room: String,
            invite: String,
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.client.core.kind() == crate::settings::ServerKind::YouTubeMusic { return self.open_lan_jam(); }
        if self.relay() == RelaySupport::Unsupported {
            return Err(NetError::Http { status: 404 });
        }
        let attempt = {
            let mut i = self.inner.lock();
            if i.hosted.is_some() || i.opening.is_some() || self.me.kind == DeviceKind::Guest {
                return Ok(None);
            }
            i.next_id += 1;
            i.opening = Some(i.next_id);
            i.next_id
        };
        // A jam plays here: what plays on another device comes back first.
        self.clone().pick(None);
        self.shown.changed();
        let url = self.relay_url("noriRemote.open", &[("dev", self.id.clone()), ("name", self.me.name.clone())]);
        let opened = async {
            let body = transport::get(&*self.client.transport, url, 0).await?;
            serde_json::from_slice::<Opened>(&body).map_err(|e| NetError::Parse { reason: e.to_string() })
        };
        let opened = match opened.await {
            Ok(o) => o,
            Err(e) => {
                let mut i = self.inner.lock();
                if i.opening == Some(attempt) {
                    i.opening = None;
                }
                drop(i);
                self.shown.changed();
                return Err(e);
            }
        };
        let server = self.client.profile.read().url.clone();
        let link = nori_remote::invite_link(&server, &opened.invite);
        {
            let mut i = self.inner.lock();
            if i.opening != Some(attempt) {
                drop(i);
                self.out(Out::Get(self.relay_url("noriRemote.close", &[("room", opened.room)])));
                return Ok(None);
            }
            i.opening = None;
            i.relay = RelaySupport::Supported;
            remember_hosted(&self.client.core.session.settings, &opened.invite);
            i.hosted = Some(Hosted { jam: Jam::new(opened.room, opened.invite, self.id.clone(), self.me.name.clone()), link: link.clone(), along: true });
            i.published = None;
            // A poll held from before listens to the account's room only: polled again, now with the jam's.
            i.end_relay_poll();
            self.cancel.notify();
        }
        self.retime();
        self.publish();
        self.keep_polling();
        self.hear_plans();
        Ok(Some(link))
    }

    /// Lets the jam's guests listen along (play its music on their own devices, in step with this one), or not.
    pub fn jam_along(self: Arc<Self>, on: bool) {
        {
            let mut i = self.inner.lock();
            let Some(h) = i.hosted.as_mut() else { return };
            h.along = on;
        }
        self.retime();
        self.hear_plans();
        self.publish();
    }

    /// Allows members to request external provider songs in the hosted room.
    pub fn jam_remote_songs(self: Arc<Self>, on: bool) {
        {
            let mut i = self.inner.lock();
            let Some(h) = i.hosted.as_mut() else { return };
            h.jam.remote_songs(on);
        }
        self.publish();
    }

    /// Listens along with the jam this guest is in (plays its host's music here, in step), or only shows it.
    pub fn listen(self: Arc<Self>, on: bool) {
        {
            let mut i = self.inner.lock();
            if i.listen.is_some() == on {
                return;
            }
            i.listen = on.then(Listen::default);
        }
        self.retime();
        self.keep_polling();
        self.follow_lead();
    }

    /// Ends the jam this device hosts, or the one it is starting.
    pub fn jam_close(self: Arc<Self>) {
        let room = {
            let mut i = self.inner.lock();
            let starting = i.opening.take().is_some();
            let Some(h) = i.hosted.take() else {
                drop(i);
                if starting {
                    self.shown.changed();
                }
                return;
            };
            // Its room is no one's now; kept, it would read as a jam this device is a guest of.
            i.rooms.retain(|r| r.room != h.jam.room);
            h.jam.room
        };
        self.retime();
        self.hear_plans();
        #[cfg(not(target_arch = "wasm32"))]
        if room == "lan-jam" {
            let mut i = self.inner.lock();
            if let Some(door) = &i.door { door.close_jam(); }
            if !i.serving { i.door = None; }
        }
        if room != "lan-jam" { self.out(Out::Get(self.relay_url("noriRemote.close", &[("room", room)]))); }
        self.keep_polling();
        self.shown.changed();
    }

    /// A jam op from this device: the host's own (accept, decline, promote, send out, add straight in),
    /// or a guest's request or decision, sent to the host.
    pub fn jam_act(self: Arc<Self>, op: Op) {
        let host = {
            let mut i = self.inner.lock();
            if i.hosted.is_none() {
                let to = i.joined().and_then(|(room, host)| Some((room.room.clone(), host?.id.clone())));
                if let Some((_, host)) = &to {
                    i.refused.remove(host);
                }
                to
            } else {
                None
            }
        };
        match host {
            None => {
                let _ = self.carry_out(&Via::Relay(None), &self.id.clone(), self.next_id(), op);
                self.shown.changed();
            }
            Some((room, host)) => {
                let id = self.next_id();
                self.out(Out::send(Link::Relay, Outgoing { room: Some(room), to: Some(host), body: Some(Body::Command { id, op: Box::new(op) }), state: None }));
            }
        }
    }

    /// The jam this device hosts or is a guest in.
    pub fn jam_view(&self) -> Option<JamView> {
        let i = self.inner.lock();
        if let Some(h) = &i.hosted {
            let st = h.jam.state();
            let queue = i.published.as_ref().map(|(s, _)| DeviceState { jam: None, ..s.clone() });
            let age_ms = i.published.as_ref().map_or(0, |(_, at)| (self.now() - at) / 1000);
            return Some(JamView { hosting: true, remote_songs: st.remote_songs, link: Some(h.link.clone()), you: self.id.clone(), members: st.members, pending: st.pending, queue, age_ms, refused: None, along: h.along, listening: Listening::Watching });
        }
        let (_, host) = i.joined()?;
        let state = host.and_then(|h| h.state.clone());
        let jam = state.as_ref().and_then(|s| s.jam.clone()).unwrap_or_default();
        Some(JamView {
            along: jam.along.is_some(),
            remote_songs: jam.remote_songs,
            listening: i.listening(),
            hosting: false,
            link: None,
            you: i.you.clone(),
            members: jam.members,
            pending: jam.pending,
            age_ms: host.map_or(0, |h| i.age(&h.id, self.now())),
            refused: host.and_then(|h| i.refused.get(&h.id).copied()),
            queue: state.map(|s| DeviceState { jam: None, ..s }),
        })
    }

    /// Where starting a jam here stands, as every screen that offers it shows it.
    pub fn jam_start(&self) -> JamStart {
        let i = self.inner.lock();
        let on = self.client.core.session.settings.current().is_some_and(|p| p.remote_control);
        if i.hosted.is_some() {
            JamStart::Hosting
        } else if i.opening.is_some() {
            JamStart::Starting
        } else if on && self.me.kind != DeviceKind::Guest && (i.relay != RelaySupport::Unsupported || self.client.core.kind() == crate::settings::ServerKind::YouTubeMusic) {
            JamStart::Offered
        } else {
            JamStart::Unavailable
        }
    }

    /// This jam guest's player controls by its role (Spotify's Jam): what each reaches, and what the play
    /// button shows. None while hosting or in no jam.
    pub fn jam_controls(&self) -> Option<JamControls> {
        self.inner.lock().jam_controls()
    }

    /// Carries out this jam guest's player control `op` by its role: sent to the host when it reaches the
    /// jam. [`Reach::Here`]: the platform's player carries it out (a guest's pause holds its own listening,
    /// and play joins the jam again where it is). [`Reach::Nowhere`]: nothing is done.
    pub fn jam_press(self: Arc<Self>, op: Op) -> Reach {
        let (reach, to) = {
            let mut i = self.inner.lock();
            let Some(c) = i.jam_controls() else { return Reach::Nowhere };
            let reach = match op {
                // Paused here while the jam plays on: play joins it again, whoever presses it.
                Op::Play if c.paused_here => Reach::Here,
                _ => c.controls.reach(&op),
            };
            let to = i.joined().and_then(|(room, host)| Some((room.room.clone(), host?.id.clone())));
            if let Some((_, host)) = &to {
                i.refused.remove(host);
            }
            (reach, to)
        };
        if reach != Reach::Jam {
            return reach;
        }
        let Some((room, host)) = to else { return Reach::Nowhere };
        let id = self.next_id();
        self.out(Out::send(Link::Relay, Outgoing { room: Some(room), to: Some(host), body: Some(Body::Command { id, op: Box::new(op) }), state: None }));
        reach
    }

    /// Ends remote control here for good: leaves the relay, withdraws the door, ends a hosted jam, stops
    /// mirroring, and ends every task this remote started, a held poll at once. Queued sends still go.
    /// Natively it waits for those tasks and sends, up to [`SENT_WAIT_MS`]: a client quitting next
    /// leaves the device lists at once.
    pub fn stop(self: Arc<Self>) {
        self.clone().serve(false);
        self.clone().discover_active(false);
        self.clone().watch(false);
        self.clone().jam_close();
        {
            let mut i = self.inner.lock();
            i.stopped = true;
            i.mirror = None;
            i.taking = None;
            i.listen = None;
            i.generation += 1;
            i.relay_polling = false;
            i.lan_generation += 1;
            i.timing += 1;
        }
        self.unfollow();
        self.retry.notify();
        self.timing.notify();
        self.cancel.notify();
        // After the tasks: a poller ending may send the leave again.
        let sent = async {
            self.running.ended().await;
            let sends = self.out.lock().take().map(|o| o.0.clone());
            if let Some(s) = sends {
                s.ended().await;
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        self.tasks.block_on(std::pin::pin!(async {
            select(std::pin::pin!(sent), self.tasks.sleep(Duration::from_millis(SENT_WAIT_MS))).await;
        }));
        // A browser cannot wait: its tasks end and its sends go on their own.
        #[cfg(target_arch = "wasm32")]
        drop(sent);
        self.out.lock().take();
        self.id_lock.lock().take();
        self.shown.changed();
    }

    /// Who asked for each song that came in through the jam, by song id, for "added by" on the queue: the
    /// whole of it while hosting (the published window holds only a few songs), the songs the host's state
    /// lists in a guest's; empty with no jam.
    pub fn jam_added(&self) -> HashMap<String, String> {
        let i = self.inner.lock();
        if let Some(h) = &i.hosted {
            return h.jam.added().clone();
        }
        let listed = i.joined().and_then(|(_, host)| host?.state.as_ref());
        listed.map(|s| s.entries.iter().filter_map(|e| Some((e.id.clone(), e.by.clone()?))).collect()).unwrap_or_default()
    }

    /// The jam this device is a guest in, as its player shows it: the host's song, its queue around it and
    /// the playhead, run on from when the host's state arrived here. None while hosting or in no jam.
    pub fn jam_playing(&self) -> Option<Mirror> {
        let i = self.inner.lock();
        if i.hosted.is_some() {
            return None;
        }
        let host = i.joined()?.1?;
        let st = host.state.as_ref()?;
        Some(Mirror {
            id: host.id.clone(),
            name: host.name.clone(),
            kind: host.kind,
            rows: st.entries.iter().map(|e| MirrorRow { index: e.index, song: e.song() }).collect(),
            at: st.entries.iter().position(|e| Some(e.index) == st.index).map(|p| p as u32),
            len: st.len,
            rev: st.rev,
            playing: st.playing,
            buffering: st.buffering,

            mixing: st.mixing,
            position_ms: st.position_ms,
            rate: nori_remote::pace(st),
            at_us: i.heard_at(st, self.now()).or_else(|| i.received.get(&host.id).copied()).unwrap_or_else(|| self.now()),
            shuffle: st.shuffle,
            repeat: st.repeat,
            volume: None,
            refused: None,
        })
    }

    /// Leaves the jam this guest profile is in, its music stopping here at once; the app then drops the
    /// profile at once too. The relay is told on the way, as far as it can be before [`Remote::stop`]
    /// gives up waiting.
    pub fn jam_leave(self: Arc<Self>) {
        self.inner.lock().jam_over = true;
        self.unfollow();
        self.out(Out::Get(self.relay_url("noriRemote.leave", &[])));
    }
}

/// What joining a jam's invite came to.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum JamJoin {
    /// A guest now: what its profile signs in with.
    Joined { pass: JamPass },
    /// The invite is to the jam this device hosts: joining would make it a guest of itself.
    Own,
    /// The jam has ended: nothing changes.
    Ended,
}

/// The app's key for the invites of the jams this device opened, newest last, one a line.
const HOSTED_INVITES: &str = "remoteHostedInvites";

/// Invites [`HOSTED_INVITES`] keeps.
const HOSTED_KEPT: usize = 16;

fn hosted_invites(settings: &Settings) -> Vec<String> {
    settings.app_value(HOSTED_INVITES).map(|v| v.lines().map(str::to_string).collect()).unwrap_or_default()
}

/// Keeps `invite` among the jams this device opened, so it is known as its own once the jam ended too.
fn remember_hosted(settings: &Settings, invite: &str) {
    let mut kept = hosted_invites(settings);
    kept.push(invite.to_string());
    let from = kept.len().saturating_sub(HOSTED_KEPT);
    let Some(db) = settings.app_db() else { return };
    // Written here rather than on the settings' writer: an invite opened at once is to read it.
    let written = db.lock().execute("INSERT OR REPLACE INTO app_kv(key, value) VALUES(?1, ?2)", rusqlite::params![HOSTED_INVITES, kept[from..].join("\n")]);
    if let Err(e) = written {
        crate::alog::info(&format!("{HOSTED_INVITES}: could not write: {e}"));
    }
}

/// What joining `link` comes to without asking the server: [`JamJoin::Own`] for the jam this device
/// (`remote`) hosts, [`JamJoin::Ended`] for another it opened; None for anyone else's.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn own_invite(settings: Arc<Settings>, remote: Option<Arc<Remote>>, link: String) -> Option<JamJoin> {
    let (_, invite) = nori_remote::parse_invite(&link)?;
    if remote.is_some_and(|r| r.inner.lock().hosted.as_ref().is_some_and(|h| h.jam.invite == invite)) {
        return Some(JamJoin::Own);
    }
    hosted_invites(&settings).contains(&invite).then_some(JamJoin::Ended)
}

/// Joins the jam `link` invites to as `name`, unless it is this device's own ([`own_invite`]) or the
/// server no longer knows the invite.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub async fn jam_join(transport: Arc<dyn Transport>, settings: Arc<Settings>, remote: Option<Arc<Remote>>, link: String, name: String) -> Result<JamJoin, NetError> {
    #[derive(Deserialize)]
    struct Joined {
        key: String,
    }
    if let Some(own) = own_invite(settings, remote, link.clone()) {
        return Ok(own);
    }
    let (server, invite) = nori_remote::parse_invite(&link).ok_or_else(|| NetError::Parse { reason: "not a jam invite".into() })?;
    let local = invite.strip_prefix("lan-");
    if local.is_some() && !nori_remote::is_home_only(&server) { return Err(NetError::Parse { reason: "LAN invitation requires a local address".into() }); }
    #[cfg(not(target_arch = "wasm32"))]
    let nonce = if local.is_some() { lan::new_secret().map_err(|e| NetError::Parse { reason: e.to_string() })? } else { String::new() };
    #[cfg(target_arch = "wasm32")]
    let nonce = if local.is_some() { return Err(NetError::Parse { reason: "LAN YouTube jams require a native client".into() }); } else { String::new() };
    if let Some(secret) = local {
        let url = format!("{}{}", server.trim_end_matches('/'), lan::signed(secret, "GET", &lan::target("noriRemote.join", &[("name".into(), name), ("nonce".into(), nonce.clone())]), b"", db::now_ms()));
        // A door refuses with 401 and a JSON body, which `transport::get` would take for an answer.
        match transport.get(url, 0).await?.status {
            401 => return Ok(JamJoin::Ended),
            200..=299 => {}
            status => return Err(NetError::Http { status }),
        }
        let key = format!("lan-{}", lan::member_secret(secret, &nonce));
        return Ok(JamJoin::Joined { pass: JamPass { url: server, api_key: nori_remote::guest_key(&key) } });
    }
    let url = api::Server::with(&server, api::Auth::ApiKey(&nori_remote::guest_key(&invite))).url("noriRemote.join", &[("name".into(), name)]);
    let body = match transport::get(&*transport, url, 0).await {
        Err(NetError::Http { status: 401 }) => return Ok(JamJoin::Ended),
        got => got?,
    };
    // The relay takes an invite only while its jam lasts.
    if refusal_code(&body) == Some(40) {
        return Ok(JamJoin::Ended);
    }
    let joined: Joined = serde_json::from_slice(&body).map_err(|e| NetError::Parse { reason: e.to_string() })?;
    Ok(JamJoin::Joined { pass: JamPass { url: server, api_key: nori_remote::guest_key(&joined.key) } })
}

/// What a client calls each kind of device, for [`device_names`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct KindWords {
    pub phone: String,
    pub desktop: String,
    pub terminal: String,
    pub guest: String,
}

impl KindWords {
    fn of(&self, kind: DeviceKind) -> &str {
        match kind {
            DeviceKind::Phone => &self.phone,
            DeviceKind::Desktop => &self.desktop,
            DeviceKind::Terminal => &self.terminal,
            DeviceKind::Guest => &self.guest,
        }
    }
}

/// Characters of a device id that tell apart devices of one name and kind.
const SHORT_ID: usize = 4;

/// The names to list `devices` by, told apart from each other and from this device (`me`): a name
/// another one has too gets the device's kind in brackets ("Mac (terminal)"), or a short id when one of
/// those is of its kind too ("Mac (3fa2)").
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn device_names(devices: Vec<RemoteDevice>, me: RemoteMe, words: KindWords) -> Vec<String> {
    let all: Vec<(&str, DeviceKind)> = std::iter::once((me.name.as_str(), me.kind)).chain(devices.iter().map(|d| (d.name.as_str(), d.kind))).collect();
    devices
        .iter()
        .enumerate()
        .map(|(k, d)| {
            let others: Vec<DeviceKind> = all.iter().enumerate().filter(|&(j, &(name, _))| j != k + 1 && name == d.name).map(|(_, &(_, kind))| kind).collect();
            if others.is_empty() {
                d.name.clone()
            } else if !others.contains(&d.kind) {
                format!("{} ({})", d.name, words.of(d.kind))
            } else {
                format!("{} ({})", d.name, d.id.chars().take(SHORT_ID).collect::<String>())
            }
        })
        .collect()
}

/// An answer from the relay; None when the server has no relay (a Subsonic error for an unknown endpoint).
/// The Subsonic error code a relay answers with instead of an [`Answer`].
fn refusal_code(body: &[u8]) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("subsonic-response")?.get("error")?.get("code")?.as_i64()
}

fn answer(body: &[u8]) -> Option<Answer> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    if v.get("subsonic-response").is_some() {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The sends waiting to go, in order.
#[derive(Default)]
struct Sends {
    queue: Mutex<Queued>,
    /// Something was queued, the queue closed, or the sender ended.
    changed: Notify,
}

#[derive(Default)]
struct Queued {
    waiting: VecDeque<Out>,
    /// Nothing more comes: the sender ends once the waiting ones went.
    closed: bool,
    ended: bool,
}

impl Sends {
    /// Until the sender ended.
    async fn ended(&self) {
        self.changed.until(|| self.queue.lock().ended).await;
    }
}

/// Queues sends; dropped, it closes the queue.
struct Outbox(Arc<Sends>);

impl Outbox {
    fn send(&self, o: Out) {
        self.0.queue.lock().waiting.push_back(o);
        self.0.changed.notify();
    }
}

impl Drop for Outbox {
    fn drop(&mut self) {
        self.0.queue.lock().closed = true;
        self.0.changed.notify();
    }
}

/// Sends what `sends` queues one at a time, in order, until it closes; `who` is this device's
/// [`Remote::who`]. It holds no [`Remote`], so it ends once the remote stops or goes.
async fn send_all(sends: Arc<Sends>, client: Arc<Client>, who: Vec<(String, String)>) {
    loop {
        let changed = sends.changed.notified();
        let (next, closed) = {
            let mut q = sends.queue.lock();
            (q.waiting.pop_front(), q.closed)
        };
        match next {
            Some(o) => deliver(&client, &who, o).await,
            None if closed => break,
            None => changed.await,
        }
    }
    sends.queue.lock().ended = true;
    sends.changed.notify();
}

/// The tasks a remote started and has not seen end.
#[derive(Default)]
struct Running {
    count: Mutex<usize>,
    /// One of them ended.
    ended: Notify,
}

impl Running {
    /// Until none runs.
    async fn ended(&self) {
        self.ended.until(|| *self.count.lock() == 0).await;
    }
}

/// The paired-device credential; YouTube has no server account credential.
fn keep_pairing(settings: &Settings, secret: &str) -> bool {
    settings.app_db().is_some_and(|db| db.lock().execute("INSERT OR REPLACE INTO app_kv(key,value) VALUES('remoteLanSecret',?1)", [secret]).is_ok())
}

fn lan_secret(client: &Client) -> Option<String> {
    if let Some((_, secret)) = client.core.lan_jam.read().as_ref() { return Some(secret.clone()); }
    if let Some((_, secret)) = client.core.account.read().as_ref() { return Some(secret.clone()); }
    let settings = &client.core.session.settings;
    let db = settings.app_db()?;
    let db = db.lock();
    if let Ok(secret) = db.query_row("SELECT value FROM app_kv WHERE key='remoteLanSecret'", [], |r| r.get::<_, String>(0)) { return Some(secret); }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secret = lan::new_secret().ok()?;
        db.execute("INSERT INTO app_kv(key,value) VALUES('remoteLanSecret',?1)", [&secret]).ok()?;
        Some(secret)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

fn remote_url(client: &Client, endpoint: &str, params: &[(String, String)], method: &str, body: &[u8]) -> String {
    if let Some((base, secret)) = client.core.lan_jam.read().as_ref() {
        return format!("{}{}", base.trim_end_matches('/'), lan::signed(secret, method, &lan::target(endpoint, params), body, db::now_ms()));
    }
    client.core.server.read().url(endpoint, params)
}

/// Sends `o`; `who` is this device's [`Remote::who`].
async fn deliver(client: &Client, who: &[(String, String)], o: Out) {
    let (url, json) = match o {
        Out::Get(url) => (url, None),
        Out::Send(link, outgoing) => {
            let json = serde_json::to_string(&outgoing).unwrap_or_default();
            let url = match link {
                Link::Relay => remote_url(client, "noriRemote.send", who, "POST", json.as_bytes()),
                Link::Lan(base) => {
                    let Some(secret) = lan_secret(client) else { return };
                    let dev = &who[0].1;
                    format!("{base}{}", lan::signed(&secret, "POST", &format!("/rest/noriRemote.send?dev={dev}"), json.as_bytes(), db::now_ms()))
                }
            };
            (url, Some(json))
        }
    };
    let sent = client.transport.send(Exchange { url, json, timeout_ms: 15_000, ..Default::default() }).await;
    if let Err(e) = sent {
        crate::alog::info(&format!("remote: not sent: {e}"));
    }
}

impl Remote {
    /// Song `id`'s star in the queue of the device [`Remote::active`] shows, without copying that queue;
    /// None while nothing is mirrored or the song is not in it.
    pub fn mirrored_starred(&self, id: &str) -> Option<bool> {
        let i = self.inner.lock();
        let m = i.mirror.as_ref()?;
        i.who(&m.id)?;
        m.starred(id)
    }

    /// Hands this device's queue and position to device `to`, which plays on; this one pauses.
    pub fn hand_over(self: Arc<Self>, to: String) {
        let me = self.id.clone();
        self.transfer(&Via::Relay(None), &me, to, None, false);
    }

    /// A remote whose work runs on `tasks`.
    pub fn with_tasks(client: Arc<Client>, me: RemoteMe, player: Arc<dyn RemotePlayer>, shown: Arc<dyn RemoteShown>, discovery: Option<Arc<dyn Discovery>>, tasks: Arc<dyn Tasks>) -> Arc<Remote> {
        let settings = &client.core.session.settings;
        let (id, id_lock) = claim_device_id(settings, me.kind);
        // Persist once per process so delayed states stay ordered across restarts.
        let instance = settings.app_db().and_then(|db| {
            db.lock().query_row(
                "INSERT INTO app_kv(key,value) VALUES('remoteInstance','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1 RETURNING value",
                [], |row| row.get::<_, String>(0),
            ).map_err(|e| crate::alog::info(&format!("remote: could not keep instance: {e}"))).ok()?.parse().ok()
        });
        let sends = Arc::new(Sends::default());
        let remote = Arc::new(Remote { client, id, id_lock: Mutex::new(id_lock), instance, me, player, shown, discovery, inner: Mutex::default(), tasks, retry: Notify::default(), timing: Notify::default(), cancel: Notify::default(), out: Mutex::new(Some(Outbox(sends.clone()))), running: Arc::default(), follower: Mutex::default() });
        remote.tasks.spawn(Box::pin(send_all(sends, remote.client.clone(), remote.who())));
        *remote.client.core.remote.lock() = Arc::downgrade(&remote);
        if remote.client.core.kind() == crate::settings::ServerKind::YouTubeMusic && remote.client.core.lan_jam.read().is_none() {
            remote.inner.lock().relay = RelaySupport::Unsupported;
        } else { remote.probe(); }
        remote
    }

    /// Plays along with a jam's host through `follower` while this guest listens along.
    pub fn follow_with(self: &Arc<Self>, follower: Option<Arc<dyn Follower>>) {
        *self.follower.lock() = follower;
        self.inner.lock().listen.iter_mut().for_each(|l| l.given = None);
        self.follow_lead();
    }

    /// Plays along no more: the follower hears there is nothing to follow, and is let go.
    fn unfollow(&self) {
        self.inner.lock().listen = None;
        let follower = self.follower.lock().take();
        if let Some(f) = follower {
            f.lead(None);
        }
    }

    /// Lets each foresight of the mirrored device go once it has shown for long enough, in a task that
    /// runs while there are any.
    fn end_foresight(self: &Arc<Self>) {
        {
            let mut i = self.inner.lock();
            if i.ending_foresight {
                return;
            }
            i.ending_foresight = true;
        }
        let me = self.clone();
        self.spawn(async move {
            loop {
                let timing = me.timing.notified();
                let wait = {
                    let mut i = me.inner.lock();
                    let Some(until) = i.mirror.as_ref().and_then(Mirrored::foreseen_until) else {
                        i.ending_foresight = false;
                        return;
                    };
                    Duration::from_micros((until - me.now()).max(0) as u64)
                };
                if let Either::Right(_) = select(me.tasks.sleep(wait), timing).await {
                    continue;
                }
                let changed = me.inner.lock().mirror.as_mut().is_some_and(|m| m.forget(None, me.now()));
                if changed {
                    me.shown.changed();
                }
            }
        });
    }

    /// Hands the host's playback to the follower when it changed.
    fn follow_lead(self: &Arc<Self>) {
        // Held from working the lead out until the follower has it, so leads reach it in the order they
        // were made, and the last one it is given is the one noted.
        let follower = self.follower.lock();
        let lead = {
            let mut i = self.inner.lock();
            let lead = i.lead(self.now());
            match i.listen.as_mut() {
                Some(l) if l.given == lead => return,
                Some(l) => l.given = lead.clone(),
                None => {}
            }
            lead
        };
        if let Some(f) = &*follower {
            f.lead(lead);
        }
    }

    /// While a jam lets its guests listen along, each transition planned here is published at once.
    fn hear_plans(self: &Arc<Self>) {
        let along = self.inner.lock().hosted.as_ref().is_some_and(|h| h.along);
        let me = Arc::downgrade(self);
        self.client.core.session.planner.on_plan(along.then(|| {
            Box::new(move || {
                if let Some(r) = me.upgrade() {
                    r.publish();
                }
            }) as nori_automix::planner::Planned
        }));
    }

    /// The lyrics service chosen here for the song at list index `index`, which a jam's members ask too.
    fn lyrics_chosen(&self, index: Option<u32>) -> Option<JamLyrics> {
        let session = &self.client.core.session;
        let id = session.playlist(|p| p.ids().get(index? as usize).cloned())?;
        let service = nori_lyrics::race::chosen_service(&*self.client.core, &session.song(&id)?)?;
        Some(JamLyrics { song: id, service })
    }

    /// Lyrics were chosen here: a jam this device hosts names them.
    pub(crate) fn lyrics_chose(self: &Arc<Self>) {
        let hosting = self.inner.lock().hosted.is_some();
        if hosting {
            self.publish();
        }
    }

    /// The lyrics service the host of the jam joined chose for song `id`.
    pub(crate) fn jam_lyrics(&self, id: &str) -> Option<String> {
        let i = self.inner.lock();
        let lyrics = i.joined()?.1?.state.as_ref()?.jam.as_ref()?.lyrics.as_ref()?;
        (lyrics.song == id).then(|| lyrics.service.clone())
    }

    /// What guests play along by: this device's speed and pitch, and the plan out of the song heard.
    fn along(&self, index: Option<u32>, playing: Playing) -> Along {
        let session = &self.client.core.session;
        let (speed, pitch) = session.settings.current().map_or((1.0, 1.0), |p| (p.speed, p.pitch));
        let mix = playing.mix_from.or(index).and_then(|at| {
            let id = session.playlist(|p| p.ids().get(at as usize).cloned())?;
            let mut mix = mix_of(at, &session.planner.made(&id)??)?;
            if playing.mix_from == Some(at) {
                mix.position_ms = Some(playing.mix_position_ms);
                mix.rate = Some(playing.mix_rate);
            }
            Some(mix)
        });
        Along { speed, pitch, mix }
    }

    fn out(&self, o: Out) {
        if let Some(out) = &*self.out.lock() {
            out.send(o);
        }
    }

    /// Starts a task [`Remote::stop`] waits for; none once stopped.
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        {
            // Counted under the lock stop reads the count with: a task starts before stop counts, or not at all.
            let mut count = self.running.count.lock();
            if self.inner.lock().stopped {
                return;
            }
            *count += 1;
        }
        let running = self.running.clone();
        self.tasks.spawn(Box::pin(async move {
            task.await;
            *running.count.lock() -= 1;
            running.ended.notify();
        }));
    }

    /// Until `gap` passed, or sooner once `done` holds, rechecked at each [`Remote::timing`] notice.
    async fn wait_timing(&self, gap: Duration, done: impl Fn(&Inner) -> bool) {
        let mut sleep = self.tasks.sleep(gap);
        loop {
            let timing = self.timing.notified();
            if done(&self.inner.lock()) {
                return;
            }
            if let Either::Left(_) = select(&mut sleep, timing).await {
                return;
            }
        }
    }

    /// A GET through the client; None when the remote stopped or the request is no longer `current`
    /// meanwhile (it is cancelled).
    async fn get(&self, url: String, timeout_ms: u32, current: impl Fn(&Inner) -> bool) -> Option<Result<Vec<u8>, NetError>> {
        match select(std::pin::pin!(self.given_up(current)), std::pin::pin!(transport::get(&*self.client.transport, url, timeout_ms))).await {
            Either::Left(_) => None,
            Either::Right((got, _)) => Some(got),
        }
    }

    /// Until the remote stopped, or a request is no longer `current`, checked at each [`Remote::cancel`].
    async fn given_up(&self, current: impl Fn(&Inner) -> bool) {
        loop {
            let cancel = self.cancel.notified();
            {
                let i = self.inner.lock();
                if i.stopped || !current(&i) {
                    return;
                }
            }
            cancel.await;
        }
    }

    /// This device's clock, µs, as the remote's executor keeps it.
    fn now(&self) -> i64 {
        self.tasks.now_us()
    }

    fn next_id(&self) -> u64 {
        let mut i = self.inner.lock();
        i.next_id += 1;
        i.next_id
    }

    fn relay_url(&self, endpoint: &str, params: &[(&str, String)]) -> String {
        let p: Vec<(String, String)> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        remote_url(&self.client, endpoint, &p, "GET", b"")
    }

    /// This device's id, name and kind, as the relay is told them (the device id first).
    fn who(&self) -> Vec<(String, String)> {
        let kind = kind_name(self.me.kind);
        vec![("dev".into(), self.id.clone()), ("name".into(), self.me.name.clone()), ("kind".into(), kind)]
    }

    /// A poll for the events after `since`, held until there are some if `hold`.
    fn poll_url(&self, since: Option<u64>, hold: bool, serve: bool) -> String {
        let mut p = self.who();
        p.push(("serve".into(), (serve as u8).to_string()));
        if let Some(s) = since {
            p.push(("since".into(), s.to_string()));
            if hold {
                p.push(("hold".into(), "1".into()));
            }
        }
        remote_url(&self.client, "noriRemote.poll", &p, "GET", b"")
    }

    fn account_tag(&self) -> Option<String> {
        let user = self.client.core.account.read().as_ref().map(|(user, _)| user.clone()).unwrap_or_else(|| "youtube".into());
        lan_secret(&self.client).map(|secret| lan::account_tag(&user, &secret))
    }

    pub(crate) fn sync_mark(&self, kind: crate::client::Starrable, id: String) {
        if !self.client.core.session.settings.current().is_some_and(|p| p.remote_control) {
            return;
        }
        let on = {
            let marks = self.client.core.stars.lock();
            let map = match kind {
                crate::client::Starrable::Song => &marks.songs,
                crate::client::Starrable::Album => &marks.albums,
                crate::client::Starrable::Artist => &marks.artists,
            };
            map.get(&id).copied()
        };
        let Some(on) = on else { return };
        let (command, peers) = {
            let mut i = self.inner.lock();
            if i.stopped { return }
            i.next_id += 1;
            let peers: Vec<_> = i.peers.iter().filter(|p| p.library_marks)
                .map(|p| (p.member.id.clone(), p.base())).collect();
            (i.next_id, peers)
        };
        for (to, base) in peers {
            let op = Op::LibraryMarks { marks: vec![wire::LibraryMark { kind, id: id.clone(), on }] };
            self.out(Out::send(Link::Lan(base), Outgoing { to: Some(to), body: Some(Body::Command { id: command, op: Box::new(op) }), ..Default::default() }));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn announcement(&self, port: u16) -> Announcement {
        let mut txt = self.who();
        txt[0].0 = "id".into();
        txt.push(("marks".into(), "1".into()));
        txt.push(("acct".into(), self.account_tag().unwrap_or_default()));
        Announcement { name: format!("nori-{}", self.id), port, txt: txt.into_iter().map(|(key, value)| Param { key, value }).collect() }
    }

    /// Closes this device's door, and opens a new one while it serves; what is announced of it.
    #[cfg(not(target_arch = "wasm32"))]
    fn reopen_door(self: &Arc<Self>, i: &mut Inner, serving: bool) -> Option<Announcement> {
        let hosting = i.hosted.as_ref().is_some_and(|h| h.jam.room == "lan-jam");
        if !hosting || i.door.is_none() {
            i.door = None;
            if serving { i.door = self.open_door(); }
        }
        i.door.as_ref().filter(|_| serving).map(|d| self.announcement(d.port()))
    }

    /// A browser opens no door.
    #[cfg(target_arch = "wasm32")]
    fn reopen_door(self: &Arc<Self>, _: &mut Inner, _: bool) -> Option<Announcement> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_lan_jam(self: &Arc<Self>) -> Result<Option<String>, NetError> {
        let secret = lan::new_secret().map_err(|e| NetError::Parse { reason: e.to_string() })?;
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| { s.connect("192.0.2.1:9")?; Ok(s) }).map_err(|e| NetError::Parse { reason: e.to_string() })?;
        let ip = socket.local_addr().map_err(|e| NetError::Parse { reason: e.to_string() })?.ip();
        let link = {
            let mut i = self.inner.lock();
            if i.hosted.is_some() || self.me.kind == DeviceKind::Guest { return Ok(None); }
            if i.door.is_none() { i.door = self.open_door(); }
            let door = i.door.as_ref().ok_or_else(|| NetError::Parse { reason: "LAN listener unavailable".into() })?;
            let invite = format!("lan-{secret}");
            let link = nori_remote::app_invite_link(&format!("http://{ip}:{}", door.port()), &invite);
            let weak = Arc::downgrade(self);
            let members = weak.clone();
            door.open_jam(secret, Box::new(move |from, body| {
                if let (Some(remote), Body::Command { id, op }) = (weak.upgrade(), body) { remote.obey(Via::LanJam, from.into(), id, *op, remote.now()); }
            }), Box::new(move |listed| {
                if let Some(remote) = members.upgrade() {
                    if let Some(hosted) = remote.inner.lock().hosted.as_mut() { hosted.jam.present(&listed); }
                    remote.publish();
                }
            }));
            remember_hosted(&self.client.core.session.settings, &invite);
            i.hosted = Some(Hosted { jam: Jam::new("lan-jam".into(), invite, self.id.clone(), self.me.name.clone()), link: link.clone(), along: true });
            i.published = None;
            link
        };
        self.retime();
        self.publish();
        self.hear_plans();
        Ok(Some(link))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_door(self: &Arc<Self>) -> Option<Door> {
        let secret = lan_secret(&self.client)?;
        let me = Member { id: self.id.clone(), name: self.me.name.clone(), kind: self.me.kind, state: None };
        let weak = Arc::downgrade(self);
        let opened = Door::open(
            me,
            secret,
            Box::new(move |from, body| {
                if let (Some(r), Body::Command { id, op }) = (weak.upgrade(), body) {
                    r.obey(Via::Door, from.to_string(), id, *op, r.now());
                }
            }),
            self.tasks.clone(),
        );
        opened.map_err(|e| crate::alog::info(&format!("remote: no door: {e}"))).ok()
    }

    /// Asks whether the server relays: the relay poller's first poll, which is not held. Polls side by
    /// side could reach the relay out of order, a late one saying this device does not serve.
    fn probe(self: &Arc<Self>) {
        if self.client.core.kind() == crate::settings::ServerKind::YouTubeMusic && self.client.core.lan_jam.read().is_none() { return; }
        let generation = {
            let mut i = self.inner.lock();
            i.probes_asked += 1;
            if std::mem::replace(&mut i.relay_polling, true) {
                return;
            }
            i.generation
        };
        self.spawn(self.clone().poll_relay(generation));
    }

    /// Tells the relay this device no longer serves.
    fn leave(&self) {
        if self.relay() != RelaySupport::Unsupported {
            self.out(Out::Get(self.poll_url(None, false, false)));
        }
    }

    /// Starts the pollers that should run and are not.
    fn keep_polling(self: &Arc<Self>) {
        let mut start_relay = None;
        let mut start_peers = Vec::new();
        {
            let mut i = self.inner.lock();
            if i.wants_relay() {
                if !i.relay_polling {
                    i.relay_polling = true;
                    start_relay = Some(i.generation);
                }
            } else if i.relay_polling {
                i.end_relay_poll();
                self.cancel.notify();
            }
            let generation = i.lan_generation;
            let follow: Vec<bool> = i.peers.iter().map(|p| !p.polling && i.follows_peer(&p.member.id)).collect();
            for (p, _) in i.peers.iter_mut().zip(follow).filter(|(_, f)| *f) {
                p.polling = true;
                start_peers.push((p.service.clone(), generation));
            }
        }
        if let Some(generation) = start_relay {
            // An older poller resting after a failure ends now.
            self.retry.notify();
            self.spawn(self.clone().poll_relay(generation));
        }
        for (service, generation) in start_peers {
            self.spawn(self.clone().poll_peer(service, generation));
        }
        self.shown.changed();
    }

    async fn poll_relay(self: Arc<Self>, generation: u64) {
        let said_serving = self.clone().relay_polls(generation).await;
        // Its last poll, saying this device serves, may reach the relay after the leave: it goes again.
        if said_serving && !self.inner.lock().serving {
            self.leave();
        }
    }

    /// Polls the relay while `generation` is current; whether its last poll said this device serves.
    async fn relay_polls(self: Arc<Self>, generation: u64) -> bool {
        let mut first_poll = true;
        let mut said_serving = false;
        let mut retry_ms = RETRY_MS;
        loop {
            let (url, probes) = {
                let mut i = self.inner.lock();
                if i.generation != generation {
                    return said_serving;
                }
                // A mirrored device paused out of sight: nothing else wants the relay.
                if !i.wants_relay() {
                    i.relay_polling = false;
                    return said_serving;
                }
                let url = self.poll_url(i.since, !first_poll && !i.relay_down, i.serving);
                first_poll = false;
                said_serving = i.serving;
                (url, i.probes_asked)
            };
            let poll = std::pin::pin!(transport::get(&*self.client.transport, url, POLL_TIMEOUT_MS));
            let got = match select(std::pin::pin!(self.given_up(|i| i.generation == generation)), poll).await {
                Either::Left(((), poll)) => {
                    // A poll saying this device serves may reach the relay after its leave: the leave goes
                    // again once the relay answered it (or the remote stopped).
                    if said_serving && !self.inner.lock().serving {
                        select(std::pin::pin!(self.given_up(|_| true)), poll).await;
                    }
                    return said_serving;
                }
                Either::Right((got, _)) => got,
            };
            let received = self.now();
            let asked_since = {
                let mut i = self.inner.lock();
                // Asked for before the rooms changed: its seq may pass events of a room it did not cover.
                if i.generation != generation {
                    return said_serving;
                }
                i.probes_answered = probes;
                i.probing()
            };
            let found = support(&got);
            self.note_relay(found, &got);
            if self.jam_gone(&got) {
                self.jam_ended(generation);
                return said_serving;
            }
            match (found, got) {
                (Some(RelaySupport::Supported), Ok(body)) => {
                    self.inner.lock().relay = RelaySupport::Supported;
                    retry_ms = RETRY_MS;
                    self.took(answer(&body).unwrap_or_default(), received);
                }
                // Asked again while this answer was on its way: the server may have gained the relay since.
                (Some(_), _) if asked_since => first_poll = true,
                (Some(_), _) => {
                    self.no_relay(generation);
                    return said_serving;
                }
                (None, _) => {
                    if !std::mem::replace(&mut self.inner.lock().relay_down, true) {
                        self.follow_active();
                        self.shown.changed();
                    }
                    let retry = self.retry.notified();
                    if self.inner.lock().generation == generation {
                        retry_ms = match select(self.tasks.sleep(Duration::from_millis(retry_ms)), retry).await {
                            Either::Left(_) => (retry_ms * 2).min(RETRY_MAX_MS),
                            Either::Right(_) => RETRY_MS,
                        };
                    }
                }
            }
        }
    }

    /// Whether a relay poll's answer says this guest's jam is gone: the relay no longer takes its key, or
    /// no longer lists its jam's room (it lists it while the jam lasts).
    fn jam_gone(&self, got: &Result<Vec<u8>, NetError>) -> bool {
        if self.me.kind == DeviceKind::Guest && self.client.core.lan_jam.read().is_some() && matches!(got, Err(NetError::Http { status: 401 })) { return true; }
        let Ok(body) = got else { return false };
        self.me.kind == DeviceKind::Guest && (refusal_code(body) == Some(40) || answer(body).is_some_and(|a| !a.rooms.iter().any(|r| r.jam)))
    }

    /// The jam this guest is in ended: nothing more is followed or asked, and the platform is told.
    fn jam_ended(&self, generation: u64) {
        let host = {
            let mut i = self.inner.lock();
            if i.generation != generation || std::mem::replace(&mut i.jam_over, true) {
                return;
            }
            i.relay_polling = false;
            i.rooms.clear();
            i.jam_host.clone()
        };
        self.unfollow();
        self.shown.jam_ended(host);
        self.shown.changed();
    }

    /// Logs what the relay's answer says of it when that differs from the last one: the relay answering,
    /// the server answering something else, the network failing.
    fn note_relay(&self, found: Option<RelaySupport>, got: &Result<Vec<u8>, NetError>) {
        let said = match (found, got) {
            (Some(RelaySupport::Supported), _) => "answers".to_string(),
            (Some(_), Ok(body)) => format!("is not there, the server said {}", String::from_utf8_lossy(&body[..body.len().min(120)])),
            (Some(_), Err(e)) => format!("is not there: {e}"),
            (None, Ok(_)) => return,
            (None, Err(e)) => format!("cannot be reached: {e}"),
        };
        if self.inner.lock().relay_said.replace(said.clone()).as_deref() != Some(&said) {
            crate::alog::info(&format!("remote: the relay {said}"));
        }
    }

    fn no_relay(&self, generation: u64) {
        let mut i = self.inner.lock();
        if i.generation == generation {
            i.relay = RelaySupport::Unsupported;
            i.relay_polling = false;
        }
        drop(i);
        self.shown.changed();
    }

    async fn poll_peer(self: Arc<Self>, service: String, generation: u64) {
        loop {
            let (base, since) = {
                let mut i = self.inner.lock();
                let Some(at) = i.peers.iter().position(|p| p.service == service) else { return };
                if i.lan_generation != generation {
                    return;
                }
                if !i.follows_peer(&i.peers[at].member.id) {
                    i.peers[at].polling = false;
                    return;
                }
                (i.peers[at].base(), i.peers[at].since)
            };
            let Some(secret) = lan_secret(&self.client) else { return };
            let query = match since {
                Some(s) => format!("/rest/noriRemote.poll?dev={}&since={s}&hold=1", self.id),
                None => format!("/rest/noriRemote.poll?dev={}", self.id),
            };
            let url = format!("{base}{}", lan::signed(&secret, "GET", &query, b"", db::now_ms()));
            let Some(got) = self.get(url, POLL_TIMEOUT_MS, |_| true).await else { return };
            let got = got.ok().and_then(|b| serde_json::from_slice::<Answer>(&b).ok());
            let received = self.now();
            let Some(a) = got else {
                // Not there at this address: the next one it was seen at, until none answers.
                let gone = {
                    let mut i = self.inner.lock();
                    let Some(p) = i.peers.iter_mut().find(|p| p.service == service) else { return };
                    p.failed += 1;
                    p.bases.rotate_left(1);
                    p.failed >= p.bases.len()
                };
                if gone {
                    self.lan_lost(service);
                    return;
                }
                continue;
            };
            let mut commands = Vec::new();
            let (from, base) = {
                let mut i = self.inner.lock();
                let Some(p) = i.peers.iter_mut().find(|p| p.service == service) else { return };
                p.since = Some(a.seq);
                let mut played = false;
                if let Some(m) = a.rooms.into_iter().flat_map(|r| r.members).next() {
                    if m.state != p.member.state {
                        p.received = received;
                    }
                    played = p.member.state.iter().chain(&m.state).any(|s| s.playing);
                    p.member.state = m.state;
                }
                p.failed = 0;
                let (from, base) = (p.member.id.clone(), p.base());
                if played {
                    i.heard_playing.insert(from.clone(), received);
                }
                commands.extend(a.events.into_iter().filter_map(|e| heard(&mut i, &from, e.body, received)));
                (from, base)
            };
            for (id, op) in commands {
                self.obey(Via::Peer(base.clone()), from.clone(), id, op, received);
            }
            self.follow_active();
            self.shown.changed();
        }
    }

    /// A relay poll's answer, received at `received` (this device's clock).
    fn took(self: &Arc<Self>, mut a: Answer, received: i64) {
        // A request from before this device served, reaching the relay late, takes it off the list.
        let unlisted = !a.rooms.iter().filter(|r| !r.jam).flat_map(|r| &r.members).any(|m| m.id == self.id && m.state.is_some());
        // The relay may still list this device from before it restarted: it is never one of the others.
        for r in &mut a.rooms {
            r.members.retain(|m| m.id != self.id);
        }
        let mut commands = Vec::new();
        let mut republish = false;
        let mut lost = Vec::new();
        {
            let mut i = self.inner.lock();
            let clock_before = i.timed();
            // An account's device is listed only the jams it opened: one it does not host is one it ended
            // or lost (its close or open went unanswered, the app started again). It is ended, not shown.
            if self.me.kind != DeviceKind::Guest {
                let hosted = i.jam_room().map(str::to_string);
                let (keep, gone): (Vec<Room>, Vec<Room>) = std::mem::take(&mut a.rooms).into_iter().partition(|r| !r.jam || Some(&r.room) == hosted.as_ref());
                a.rooms = keep;
                a.events.retain(|e| !gone.iter().any(|r| r.room == e.room));
                // One being opened may be listed before its answer arrives.
                if i.opening.is_none() {
                    lost = gone.into_iter().map(|r| r.room).collect();
                }
            }
            // Commands are heard from the first answer on: only then is this device's state said to the
            // relay, so no controller or guest sends it one before it can hear it.
            if i.since.is_none() && (i.serving || i.hosted.is_some()) || i.serving && unlisted {
                i.published = None;
                republish = true;
            }
            i.since = Some(a.seq);
            i.relay_down = false;
            i.you = a.you;
            i.relay_along = a.along;
            // The time keeper may wait for it.
            if std::mem::replace(&mut i.relay_time, a.time) != a.time {
                self.timing.notify();
            }
            for m in a.rooms.iter().flat_map(|r| &r.members) {
                let before = i.rooms.iter().flat_map(|r| &r.members).find(|o| o.id == m.id).map(|o| &o.state);
                let played = before.into_iter().chain([&m.state]).flatten().any(|s| s.playing);
                if before != Some(&m.state) {
                    i.received.insert(m.id.clone(), received);
                }
                // It played until this answer if it plays, or did as last heard.
                if played {
                    i.heard_playing.insert(m.id.clone(), received);
                }
            }
            if let Some(h) = &mut i.hosted {
                if let Some(r) = a.rooms.iter().find(|r| r.room == h.jam.room) {
                    republish |= h.jam.present(&r.members);
                }
            }
            for e in a.events {
                if let Some((id, op)) = heard(&mut i, &e.from, e.body, received) {
                    let jam = a.rooms.iter().any(|r| r.jam && r.room == e.room);
                    commands.push((Via::Relay(jam.then_some(e.room)), e.from, id, op));
                }
            }
            i.rooms = a.rooms;
            if i.timed() != clock_before {
                self.timing.notify();
            }
            if let Some((_, Some(host))) = i.joined() {
                i.jam_host = Some(host.name.clone());
            }
        }
        for room in lost {
            crate::alog::info("remote: the relay still lists a jam this device ended: closed again");
            self.out(Out::Get(self.relay_url("noriRemote.close", &[("room", room)])));
        }
        for (via, from, id, op) in commands {
            self.obey(via, from, id, op, received);
        }
        if republish {
            self.publish();
        }
        self.follow_active();
        self.follow_lead();
        self.shown.changed();
    }

    /// Makes `to` the active device (None: this one), and follows it while it is another.
    fn set_active(self: &Arc<Self>, to: Option<String>) {
        {
            let mut i = self.inner.lock();
            if i.mirror.as_ref().map(|m| &m.id) == to.as_ref() || (to.is_some() && i.jams()) {
                return;
            }
            i.mirror = to.map(|id| Mirrored::new(id, self.now()));
        }
        self.retime();
        self.follow_active();
        self.keep_polling();
    }

    /// What needs another clock changed: the time keeper starts again for it, or ends.
    fn retime(self: &Arc<Self>) {
        let generation = {
            let mut i = self.inner.lock();
            i.timing += 1;
            (i.mirror.is_some() || i.listen.is_some() || i.hosts_along()).then_some(i.timing)
        };
        self.timing.notify();
        if let Some(generation) = generation {
            self.spawn(self.clone().keep_time(generation));
        }
    }

    /// While what needs another clock stays the same (`generation`), learns how that clock stands to this
    /// one's: a burst of time exchanges, then one every [`clock::EVERY_US`]. The mirrored device's only
    /// while it plays on screen (paused its playhead stands still, and off screen nothing shows it to the
    /// millisecond: nothing wakes for it), through its door when it is near, else through the relay,
    /// whose answer comes with a poll. A jam's host and its guests listening along each learn the relay's
    /// clock when it tells it, the guests the host's otherwise.
    async fn keep_time(self: Arc<Self>, generation: u64) {
        let mut sent = 0;
        let mut last = None;
        loop {
            if sent > 0 {
                let gap = if sent < clock::BURST { Duration::from_millis(BURST_GAP_MS) } else { Duration::from_micros(clock::EVERY_US as u64) };
                self.wait_timing(gap, |i| i.timing != generation || i.timed() != last).await;
            }
            let (target, link) = loop {
                let timing = self.timing.notified();
                {
                    let i = self.inner.lock();
                    if i.timing != generation {
                        return;
                    }
                    if i.wants_time() {
                        // A jam's host is known once the relay has answered: a guest listening along waits.
                        match i.timed() {
                            None if i.listen.is_none() => return,
                            None => {}
                            Some(target) => {
                                let door = match &target {
                                    Timed::Device { id, room: None } => i.peers.iter().find(|p| p.member.id == *id).map(|p| Link::Lan(p.base())),
                                    _ => None,
                                };
                                break (target, door.or((i.relay != RelaySupport::Unsupported).then_some(Link::Relay)));
                            }
                        }
                    }
                }
                timing.await;
            };
            // Another clock: its burst again.
            if last.as_ref() != Some(&target) {
                sent = 0;
                last = Some(target.clone());
            }
            sent += 1;
            match (target, link) {
                (Timed::Server, _) => {
                    let base = self.client.profile.read().url.trim_end_matches('/').to_string();
                    let Some(got) = self.get(format!("{base}/nori/time?t1={}", self.now()), 5_000, |_| true).await else { return };
                    let t4 = self.now();
                    if let Some(Body::Clock { t1, t2, t3 }) = got.ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                        self.inner.lock().server_clock.add(clock::Exchange { t1, t2, t3, t4 });
                        self.follow_lead();
                        self.publish();
                        self.shown.changed();
                    }
                }
                (Timed::Device { id, .. }, Some(Link::Lan(base))) => {
                    let Some(secret) = lan_secret(&self.client) else { return };
                    let t1 = self.now();
                    let url = format!("{base}{}", lan::signed(&secret, "GET", &format!("/rest/noriRemote.time?dev={}&t1={t1}", self.id), b"", db::now_ms()));
                    let Some(got) = self.get(url, 5_000, |_| true).await else { return };
                    let t4 = self.now();
                    if let Some(Body::Clock { t1, t2, t3 }) = got.ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                        timed(&mut self.inner.lock(), &id, clock::Exchange { t1, t2, t3, t4 });
                        self.follow_lead();
                        self.shown.changed();
                    }
                }
                (Timed::Device { id, room }, Some(Link::Relay)) => {
                    let command = Body::Command { id: self.next_id(), op: Box::new(Op::Clock { t1: self.now() }) };
                    self.out(Out::send(Link::Relay, Outgoing { room, to: Some(id), body: Some(command), ..Default::default() }));
                }
                (Timed::Device { .. }, None) => {}
            }
        }
    }

    /// Takes the mirrored device's newest state, follows it on to the device it handed its playback to,
    /// lets it go once it is gone, and asks for the next page of its queue.
    fn follow_active(self: &Arc<Self>) {
        let gone = {
            let mut i = self.inner.lock();
            let gone = i.taking.as_ref().is_some_and(|t| !i.listed(&t.from));
            if gone { i.taking = None; }
            gone
        };
        if gone { self.keep_polling(); }
        let discovered = {
            let mut i = self.inner.lock();
            let to = if i.discovering && i.mirror.is_none() && i.taking.is_none() && !i.playing.playing && !i.jams() {
                i.peers.iter().map(|p| &p.member).chain(i.rooms.iter().filter(|r| !r.jam && !i.relay_down).flat_map(|r| &r.members))
                    .find(|m| i.state_of(&m.id).is_some_and(|(s, _)| s.playing && s.index.is_some()))
                    .map(|m| m.id.clone())
            } else {
                None
            };
            to.map(|id| {
                let before = i.browsing();
                i.mirror = Some(Mirrored::new(id, self.now()));
                i.discovering = false;
                if !i.watching {
                    i.lan_generation += 1;
                    i.peers.iter_mut().for_each(|p| p.polling = false);
                }
                (before != i.browsing()).then_some(i.browsing())
            })
        };
        if let Some(browse) = discovered {
            if let (Some(d), Some(on)) = (&self.discovery, browse) {
                d.browse(on);
            }
            self.retime();
            self.keep_polling();
        }
        let mut page = None;
        let next = {
            let mut i = self.inner.lock();
            let Some(id) = i.mirror.as_ref().map(|m| m.id.clone()) else { return };
            if !i.listed(&id) {
                i.mirror = None;
                drop(i);
                self.retime();
                self.keep_polling();
                return;
            }
            let heard = i.state_of(&id).map(|(s, at)| (s.clone(), at.unwrap_or_else(|| self.now())));
            let me = self.id.clone();
            let m = i.mirror.as_mut().expect("mirrored");
            let moved = match heard {
                Some((st, at)) if m.heard(&st, at, &me) => {
                    // It may have started or stopped playing: the time keeper looks again.
                    self.timing.notify();
                    st.handed_to.filter(|to| *to != me && *to != id)
                }
                _ => None,
            };
            if moved.is_none() {
                if let Some(from) = m.wanted() {
                    m.asking = Some(from);
                    page = Some((id, from));
                }
            }
            moved
        };
        if let Some(to) = next {
            return self.set_active(Some(to));
        }
        if let Some((id, from)) = page {
            self.clone().send(id, Op::Page { from, count: PAGE });
        }
    }

    /// Carries out a command, received at `received` (this device's clock), and answers it.
    fn obey(self: &Arc<Self>, via: Via, from: String, id: u64, op: Op, received: i64) {
        let body = match op {
            Op::LibraryMarks { marks } => {
                let allowed = matches!(via, Via::Door) && !self.inner.lock().stopped
                    && self.client.core.session.settings.current().is_some_and(|p| p.remote_control)
                    && self.account_tag().is_some();
                let refusal = if allowed {
                    for mark in &marks {
                        for prefix in crate::cache_policy::stale(&crate::client::Write::Star { kind: mark.kind, id: mark.id.clone(), on: mark.on }) {
                            let _ = self.client.core.cache_stale(prefix);
                        }
                    }
                    {
                        let mut stars = self.client.core.stars.lock();
                        for mark in marks {
                            stars.insert(mark.kind, mark.id, mark.on);
                        }
                    }
                    self.shown.marks_changed();
                    self.publish();
                    None
                } else {
                    Some(Refusal::NotAllowed)
                };
                Body::Ack { id, refusal }
            }
            Op::Clock { t1 } => Body::Clock { t1, t2: received, t3: self.now() },
            Op::Page { from: turn, count } if !matches!(via, Via::Relay(Some(_)) | Via::LanJam) => {
                let q = self.read_queue(|_, len| {
                    let start = (turn as usize).min(len);
                    start..(turn.saturating_add(count.min(PAGE_MAX)) as usize).min(len)
                });
                Body::Page { id, rev: q.rev, from: turn, entries: q.entries }
            }
            op => Body::Ack { id, refusal: self.carry_out(&via, &from, id, op).err() },
        };
        self.answer_to(via, from, body);
    }

    fn answer_to(&self, via: Via, to: String, body: Body) {
        match via {
            Via::Relay(room) => self.out(Out::send(Link::Relay, Outgoing { room, to: Some(to), body: Some(body), state: None })),
            #[cfg(not(target_arch = "wasm32"))]
            Via::Door | Via::LanJam => {
                if let Some(d) = &self.inner.lock().door {
                    d.reply(&to, body);
                }
            }
            #[cfg(target_arch = "wasm32")]
            Via::Door | Via::LanJam => {}
            Via::Peer(base) => self.out(Out::send(Link::Lan(base), Outgoing { to: Some(to), body: Some(body), ..Default::default() })),
        }
    }

    fn carry_out(self: &Arc<Self>, via: &Via, from: &str, id: u64, op: Op) -> Result<(), Refusal> {
        let in_jam = matches!(via, Via::Relay(Some(_)) | Via::LanJam);
        if is_jam(&op) {
            let kick = match &op {
                Op::Kick { member } => Some(member.clone()),
                _ => None,
            };
            let (song, room) = {
                let mut i = self.inner.lock();
                let h = i.hosted.as_mut().ok_or(Refusal::Unknown)?;
                let by = if in_jam { By::Member(from) } else { By::Host };
                (h.jam.apply(by, op)?, h.jam.room.clone())
            };
            if let Some(song) = song {
                self.player.apply(Op::Add { songs: vec![song], next: false });
            }
            if let Some(member) = kick {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(door) = &self.inner.lock().door { if room == "lan-jam" { door.kick(&member); } }
                if room != "lan-jam" { self.out(Out::Get(self.relay_url("noriRemote.kick", &[("room", room), ("member", member)]))); }
            }
            self.publish();
            return Ok(());
        }
        let sender = if in_jam {
            let role = self.inner.lock().hosted.as_ref().and_then(|h| h.jam.role(By::Member(from)));
            Sender::Member(role.ok_or(Refusal::NotAllowed)?)
        } else {
            Sender::Owner
        };
        let (rev, len) = self.client.core.session.playlist(|p| (p.rev(), p.len() as u32));
        admit(&op, sender, rev, len)?;
        if in_jam {
            let mut i = self.inner.lock();
            let h = i.hosted.as_mut().ok_or(Refusal::Unknown)?;
            match &op {
                Op::Add { songs, .. } => h.jam.queue_songs(By::Member(from), songs)?,
                Op::Replace { songs, .. } => {
                    for song in songs {
                        h.jam.may_request(By::Member(from), song)?;
                    }
                }
                _ => {}
            }
        }
        if let Op::Replace { transfer, .. } = &op {
            let mut i = self.inner.lock();
            if let Some(request) = transfer {
                if !i.taking.as_ref().is_some_and(|t| t.from == from && t.id == *request) {
                    return Err(Refusal::Stale);
                }
            }
            let stale = i.taking.as_ref().is_some_and(|t| t.from == from && t.rev != rev);
            let cancelled = i.taking.take().is_some();
            drop(i);
            if cancelled {
                self.keep_polling();
            }
            if stale {
                return Err(Refusal::Stale);
            }
        }
        // Said with the next state published, which shows what the player made of it.
        if !in_jam {
            let mut i = self.inner.lock();
            i.obeyed.retain(|o| o.from != from);
            i.obeyed.push(Obeyed { from: from.to_string(), id });
        }
        match op {
            Op::Transfer { to, listen_handoff } => {
                let transfer = (to == from).then_some(id);
                self.transfer(via, from, to, transfer, listen_handoff);
            }
            Op::Clear => {
                for index in self.client.core.session.playlist(|p| p.after_current()) {
                    self.player.apply(Op::Remove { index: index as u32, rev });
                }
            }
            op => {
                // A queue sent here plays here: this is the active device.
                if let Op::Replace { listen, .. } = &op {
                    self.client.core.session.receive_listen(listen.clone());
                    self.set_active(None);
                }
                self.player.apply(op)
            }
        }
        Ok(())
    }

    /// Takes the controls back here while the previous device answers with its queue.
    fn pull(self: &Arc<Self>, from: String) {
        let id = self.next_id();
        let rev = self.client.core.session.playlist(|p| p.rev());
        let link = {
            let mut i = self.inner.lock();
            let link = i.peers.iter().find(|p| p.member.id == from).map_or(Link::Relay, |p| Link::Lan(p.base()));
            i.taking = Some(PendingTransfer { from: from.clone(), id, rev });
            i.mirror = None;
            link
        };
        self.retime();
        self.out(Out::send(link, Outgoing { to: Some(from), body: Some(Body::Command { id, op: Box::new(Op::Transfer { to: self.id.clone(), listen_handoff: true }) }), ..Default::default() }));
        self.keep_polling();
    }

    /// Hands the queue, its play order, shuffle, repeat and the position to device `to`, then pauses here
    /// and follows `to`, the active device now.
    fn transfer(self: &Arc<Self>, via: &Via, from: &str, to: String, transfer: Option<u64>, listen_handoff: bool) {
        if to != self.id && self.inner.lock().jams() {
            return;
        }
        let session = &self.client.core.session;
        // A weighted shuffle's list is already in play order: shown shuffled, it goes as that order.
        let (ids, index, order, shuffle, repeat) = session.playlist(|p| (p.ids().to_vec(), p.current(), p.lit().then(|| p.play_order().map(|i| i as u32).collect()), p.lit(), p.repeat()));
        let Some(index) = index else { return };
        let songs = ids.into_iter().map(|id| session.song(&id).unwrap_or_else(|| Song::only_id(id))).collect();
        let (playing, position_ms) = {
            let mut i = self.inner.lock();
            i.handed_to = Some(to.clone());
            (i.playing.playing, i.position_now(self.now()))
        };
        let accepts_listen = (to == from && listen_handoff) || {
            let i = self.inner.lock();
            i.peers.iter().map(|p| &p.member).chain(i.rooms.iter().filter(|r| !r.jam).flat_map(|r| &r.members))
                .any(|m| m.id == to && m.state.as_ref().is_some_and(|s| s.listen_handoff))
        };
        let listen = accepts_listen.then(|| session.hand_listen(self.now() / 1_000)).flatten();
        let op = Op::Replace { songs, index: index as u32, position_ms, play: playing, order, shuffle, repeat, transfer, listen };
        let id = self.next_id();
        let command = Body::Command { id, op: Box::new(op) };
        if to == from {
            self.answer_to(via.clone(), to.clone(), command);
        } else {
            let link = self.inner.lock().peers.iter().find(|p| p.member.id == to).map_or(Link::Relay, |p| Link::Lan(p.base()));
            self.out(Out::send(link, Outgoing { to: Some(to.clone()), body: Some(command), ..Default::default() }));
        }
        self.player.apply(Op::Pause);
        self.set_active(Some(to));
        self.publish();
    }

    /// The songs at the turns of the play order `span` picks (from where the song heard is, and how many
    /// there are), with the queue's revision, length, shuffle and repeat.
    fn read_queue(&self, span: impl FnOnce(Option<usize>, usize) -> Range<usize>) -> QueueRead {
        self.read_queue_at(&self.inner.lock(), span)
    }

    fn read_queue_at(&self, i: &Inner, span: impl FnOnce(Option<usize>, usize) -> Range<usize>) -> QueueRead {
        let session = &self.client.core.session;
        let heard = i.playing.index.map(|i| i as usize);
        let (picked, index, rev, len, shuffle, repeat) = session.playlist(|p| {
            let current = heard.filter(|&i| i < p.len()).or(p.current());
            let order: Vec<usize> = p.play_order().collect();
            let at = current.and_then(|c| order.iter().position(|&o| o == c));
            let r = span(at, order.len());
            let picked: Vec<(u32, u32, String)> = order[r.clone()].iter().zip(r.start..).map(|(&i, turn)| (i as u32, turn as u32, p.ids()[i].clone())).collect();
            (picked, current.map(|c| c as u32), p.rev(), p.len() as u32, p.lit(), p.repeat())
        });
        let jam = i.hosted.as_ref().map(|h| &h.jam);
        // Hearts as this device shows them: pressed here or by another device since the song was read.
        let stars = self.client.core.stars.lock();
        let entries = picked
            .into_iter()
            .map(|(index, turn, id)| {
                let by = jam.and_then(|j| j.added_by(&id));
                let mut song = session.song(&id).unwrap_or_else(|| Song::only_id(id));
                song.starred = stars.starred(crate::client::Starrable::Song, &song.id, song.starred);
                Entry::of(index, turn, &song, by)
            })
            .collect();
        QueueRead { entries, index, rev, len, shuffle, repeat }
    }

    /// This device's state now, its position where the song is at this moment.
    fn state_now(&self, i: &Inner) -> DeviceState {
        let q = self.read_queue_at(i, |at, len| {
            let at = at.unwrap_or(0);
            at.saturating_sub(ENTRIES_BEFORE)..(at + ENTRIES_AFTER + 1).min(len)
        });
        let p = i.playing;
        let jam = i.hosted.as_ref().map(|h| JamState { along: h.along.then(|| self.along(q.index, p)), lyrics: self.lyrics_chosen(q.index), ..h.jam.state() });
        let mut st = DeviceState {
            seq: i.seq,
            listen_handoff: true,
            instance: self.instance,
            playing: p.playing,
            buffering: p.buffering,
            mixing: p.mixing,
            position_ms: p.position_ms,
            index: q.index,
            rev: q.rev,
            len: q.len,
            entries: q.entries,
            volume: p.volume,
            shuffle: q.shuffle,
            repeat: q.repeat,
            jam,
            handed_to: i.handed_to.clone(),
            obeyed: i.obeyed.clone(),
            at_us: None,
            server_us: None,
            rate: Some(p.rate),
        };
        let now = self.now();
        st.position_ms = i.position_at_of(&st, now);

        if p.playing && !p.buffering {
            if let Some(mix) = st
                .jam
                .as_mut()
                .and_then(|j| j.along.as_mut())
                .and_then(|a| a.mix.as_mut())
            {
                if let Some(ms) = mix.position_ms.as_mut() {
                    *ms += (i.since_played_ms(now) as f64 * mix.rate.unwrap_or(1.0) as f64) as i64;
                }
            }
        }
        st.at_us = Some(now);
        st.server_us = i.server_clock.settled_offset_at(now).filter(|_| i.hosts_along());
        st
    }

    /// Publishes this device's state where it is followed, unless only time moved on.
    fn publish(self: &Arc<Self>) {
        let (state, serving, jam_room, relay) = {
            let mut i = self.inner.lock();
            let mut state = self.state_now(&i);
            if let Some((last, _)) = &i.published {
                if same_but_time(last, &state) {
                    return;
                }
            }
            i.seq += 1;
            state.seq = i.seq;
            i.published = Some((state.clone(), self.now()));
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(d) = &i.door {
                d.publish(state.clone());
            }
            let heard = i.since.is_some();
            (state, i.serving && heard, i.jam_room().filter(|_| heard).map(str::to_string), i.relay != RelaySupport::Unsupported)
        };
        if serving && relay {
            self.out(Out::send(Link::Relay, Outgoing { state: Some(state.clone()), ..Default::default() }));
        }
        if let Some(room) = jam_room {
            self.out(Out::send(Link::Relay, Outgoing { room: Some(room), state: Some(state), ..Default::default() }));
        }
        self.shown.changed();
    }
}

/// What [`Remote::read_queue`] read.
struct QueueRead {
    entries: Vec<Entry>,
    index: Option<u32>,
    rev: u64,
    len: u32,
    shuffle: bool,
    repeat: u8,
}

/// The transition planned out of list index `from`, as a jam's guests get it.
fn mix_of(from: u32, plan: &Plan) -> Option<Mix> {
    Some(Mix { from, into: plan.incoming_id.clone(), plan: serde_json::to_string(&plan.mixer).ok()? ,
        position_ms: None,
        rate: None})
}

/// A guest's transition out of the song `mix` names in the host's `entries`: (its id, the plan).
fn plan_of(mix: &Mix, entries: &[Entry]) -> Option<(String, Plan)> {
    let from = entries.iter().find(|e| e.index == mix.from)?;
    let plan: TransitionPlan = serde_json::from_str(&mix.plan).ok()?;
    Some((from.id.clone(), engine_plan(&plan, &mix.into)?))
}

/// Event `body` from device `from` arrived at `received` (this device's clock): an answer, a page or a
/// time exchange is noted in `i`; a command is handed back, to obey once `i` is let go.
fn heard(i: &mut Inner, from: &str, body: Body, received: i64) -> Option<(u64, Op)> {
    match body {
        Body::Command { id, op } => return Some((id, *op)),
        Body::Ack { id, refusal } => note(i, from, id, refusal, received),
        Body::Page { rev, from: turn, entries, .. } => paged(i, from, rev, turn, entries),
        Body::Clock { t1, t2, t3 } => timed(i, from, clock::Exchange { t1, t2, t3, t4: received }),
    }
    None
}

/// A page of the queue of device `from` arrived.
fn paged(i: &mut Inner, from: &str, rev: u64, turn: u32, entries: Vec<Entry>) {
    if let Some(m) = i.mirror.as_mut().filter(|m| m.id == from) {
        m.page(rev, turn, entries);
    }
}

/// A time exchange with device `from` came back.
fn timed(i: &mut Inner, from: &str, exchange: clock::Exchange) {
    if let Some(m) = i.mirror.as_mut().filter(|m| m.id == from) {
        m.clock.add(exchange);
    }
    let host = i.joined().and_then(|(_, h)| h).is_some_and(|h| h.id == from);
    if let Some(l) = i.listen.as_mut().filter(|_| host) {
        l.clock.add(exchange);
    }
}

/// Device `from` answered command `id`; a refusal ends its foresight.
fn note(i: &mut Inner, from: &str, id: u64, refusal: Option<Refusal>, now: i64) {
    match refusal {
        Some(r) => {
            i.refused.insert(from.to_string(), r);
            if let Some(m) = i.mirror.as_mut().filter(|m| m.id == from) {
                m.forget(Some(id), now);
            }
        }
        None => {
            i.refused.remove(from);
        }
    }
}

/// Whether `now` is `last` with only its position run on as time passed.
fn same_but_time(last: &DeviceState, now: &DeviceState) -> bool {
    let elapsed_ms = now.at_us.zip(last.at_us).map_or(0, |(now, last)| (now - last) / 1000);
    let expected = nori_remote::position_now(last, elapsed_ms);
    let server_same = match (last.server_us, now.server_us) {
        (Some(a), Some(b)) => (a - b).abs() <= SERVER_SLACK_US,
        (a, b) => a == b,
    };
    let mut adjusted= DeviceState { position_ms: now.position_ms, seq: now.seq, at_us: now.at_us, server_us: now.server_us, ..last.clone() } ;
    if let (Some(a), Some(b)) = (
        adjusted
            .jam
            .as_mut()
            .and_then(|j| j.along.as_mut())
            .and_then(|a| a.mix.as_mut()),
        now.jam
            .as_ref()
            .and_then(|j| j.along.as_ref())
            .and_then(|a| a.mix.as_ref()),
    ) {
        if let (Some(a_ms), Some(b_ms)) = (a.position_ms.as_mut(), b.position_ms) {
            let ran = if last.playing && !last.buffering {
                (elapsed_ms as f64 * a.rate.unwrap_or(1.0) as f64) as i64
            } else {
                0
            };
            if (*a_ms + ran - b_ms).abs() > POSITION_SLACK_MS {
                return false;
            }
            *a_ms = b_ms;
        }
    }
    (expected - now.position_ms).abs() <= POSITION_SLACK_MS && server_same && adjusted == *now
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;
    impl RemotePlayer for Quiet { fn apply(&self, _: Op) {} }
    impl RemoteShown for Quiet {
        fn changed(&self) {}
        fn marks_changed(&self) {}
        fn jam_ended(&self, _: Option<String>) {}
    }

    /// Leaving a jam opens the profile in use before it, not whichever was added last, and drops only
    /// guest profiles.
    #[test]
    fn leaving_a_jam_returns_to_the_profile_before_it() {
        use crate::settings::{SavedServer, ServerList};
        let dir = nori_testdir::TempDir::new("jam-return");
        let settings = crate::settings_store::Settings::new();
        let mut prefs = settings.open(&dir.path().join(crate::db::DB_FILE).to_string_lossy()).unwrap();
        let own = |id: &str| SavedServer { id: id.into(), url: format!("http://{id}"), ..Default::default() };
        let guest = jam_profile(JamPass { url: "http://friend".into(), api_key: nori_remote::guest_key("k") }, "guest".into(), "Jam".into());
        prefs.servers = vec![own("navidrome"), own("youtube")];
        prefs.active_server_id = "navidrome".into();
        settings.put(prefs);
        jam_joining(&settings);
        crate::background::flush();
        let list = ServerList { servers: vec![own("navidrome"), own("youtube"), guest], active_server_id: "guest".into() };
        let back = jam_return(&settings, list);
        assert_eq!(back.active_server_id, "navidrome");
        assert_eq!(back.servers.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["navidrome", "youtube"]);
    }

    #[test]
    fn the_players_controls_become_the_devices_commands() {
        let mut m = Mirror { id: "desk".into(), name: "Desk".into(), kind: DeviceKind::Desktop, rows: Vec::new(), at: None, len: 3, rev: 7, playing: true, buffering: false, mixing: false, position_ms: 0, rate: 1.0, at_us: 0, shuffle: false, repeat: 0, volume: Some(40), refused: None };
        let pressed = [
            (Press::Toggle, Some(Op::Pause)),
            (Press::Play, Some(Op::Play)),
            (Press::Seek { ms: -40 }, Some(Op::Seek { ms: 0 })),
            (Press::Jump { index: 1 }, Some(Op::Jump { index: 1, rev: 7 })),
            (Press::Repeat { mode: 2 }, Some(Op::Repeat { mode: 2 })),
            (Press::Remove { index: 2 }, Some(Op::Remove { index: 2, rev: 7 })),
            (Press::Move { from: 2, to: 0 }, Some(Op::Move { from: 2, to: 0, rev: 7 })),
            (Press::Clear, Some(Op::Clear)),
            (Press::Volume { level: 0.254 }, Some(Op::Volume { percent: 25 })),
            (Press::Volume { level: 0.4 }, None),
        ];
        for (press, op) in pressed {
            assert_eq!(m.op(press), op, "{press:?}");
        }
        m.playing = false;
        assert_eq!(m.op(Press::Toggle), Some(Op::Play));
        m.volume = None;
        assert_eq!(m.op(Press::Volume { level: 0.9 }), None, "a volume the device does not offer");
    }

    #[test]
    fn stop_returns_while_a_task_starts_another() {
        let (client, _) = crate::client::tests::client(crate::client::NetProfile { url: "http://test".into(), ..Default::default() });
        let remote = Remote::new(client, RemoteMe { name: "Test".into(), kind: DeviceKind::Desktop }, Arc::new(Quiet), Arc::new(Quiet), None);
        let late = remote.clone();
        let (started, running) = std::sync::mpsc::channel();
        remote.spawn(async move {
            // Starts another task once stop is under way.
            loop {
                let timing = late.timing.notified();
                let _ = started.send(());
                if late.inner.lock().stopped {
                    break;
                }
                timing.await;
            }
            late.spawn(async {});
        });
        running.recv_timeout(Duration::from_secs(10)).unwrap();
        let (sent, stopped) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            remote.clone().stop();
            let _ = sent.send(*remote.running.count.lock());
        });
        assert_eq!(stopped.recv_timeout(Duration::from_secs(10)), Ok(0), "stop waited for a task waiting on it, or a task started after it");
    }

    #[test]
    fn published_state_uses_one_player_report() {
        let (client, _) = crate::client::tests::client(crate::client::NetProfile { url: "http://test".into(), ..Default::default() });
        client.core.session.set(vec!["a".into(), "b".into()], Some(0), false, None);
        let remote = Remote::new(client, RemoteMe { name: "Test".into(), kind: DeviceKind::Desktop }, Arc::new(Quiet), Arc::new(Quiet), None);
        remote.inner.lock().playing = Playing { index: Some(0), volume: Some(0), ..Default::default() };
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let start = Arc::new(std::sync::Barrier::new(2));
        let reporter = {
            let (remote, done, start) = (remote.clone(), done.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                let mut index = 0;
                while !done.load(std::sync::atomic::Ordering::Acquire) {
                    index ^= 1;
                    remote.inner.lock().playing = Playing { index: Some(index), volume: Some(index as u8), ..Default::default() };
                }
            })
        };
        start.wait();
        let inconsistent = (0..50_000).find_map(|_| {
            remote.publish();
            let i = remote.inner.lock();
            let state = &i.published.as_ref().expect("published").0;
            (state.index != state.volume.map(u32::from)).then_some((state.index, state.volume, state.seq))
        });
        done.store(true, std::sync::atomic::Ordering::Release);
        reporter.join().unwrap();
        remote.stop();
        assert!(inconsistent.is_none(), "a published frame mixed player reports: {inconsistent:?}");
    }

    #[test]
    fn acknowledged_skip_waits_for_the_heard_song() {
        let entries = (0..3).map(|index| Entry { index, turn: index, duration: 300, ..Default::default() }).collect();
        let old = DeviceState { seq: 1, index: Some(0), len: 3, entries, ..Default::default() };
        for refused in [false, true] {
            let mut m = Mirrored::new("desk".into(), clock::now_us());
            m.heard(&old, 0, "me");
            m.foresee(1, &Op::Next, clock::now_us());
            assert_eq!(m.shown.as_ref().unwrap().index, Some(1));
            let accepted = DeviceState { seq: 2, obeyed: vec![Obeyed { from: "me".into(), id: 1 }], ..old.clone() };
            m.heard(&accepted, 1, "me");
            assert_eq!(m.shown.as_ref().unwrap().index, Some(1), "accepting the command keeps the next song");
            assert!(!m.forget(None, clock::now_us() + FORESEEN_US * 2), "an accepted skip waits for the player");
            assert_eq!(m.shown.as_ref().unwrap().index, Some(1));
            if refused {
                assert!(m.forget(Some(1), clock::now_us()));
            } else {
                m.heard(&DeviceState { seq: 3, index: Some(1), ..accepted }, 2, "me");
            }
            assert!(m.foreseen.is_empty(), "the heard song or a refusal settles the skip");
            assert_eq!(m.shown.as_ref().unwrap().index, Some(if refused { 0 } else { 1 }));
        }
        let mut unanswered = Mirrored::new("legacy".into(), clock::now_us());
        unanswered.heard(&old, 0, "me");
        unanswered.foresee(1, &Op::Next, clock::now_us());
        assert!(unanswered.forget(None, clock::now_us() + FORESEEN_US * 2));
        assert_eq!(unanswered.shown.as_ref().unwrap().index, Some(0), "an unanswered skip expires");
    }

    #[test]
    fn older_remote_state_cannot_undo_a_confirmed_command() {
        let old = DeviceState { seq: 2, playing: true, index: Some(0), len: 1, entries: vec![Entry { duration: 300, ..Default::default() }], ..Default::default() };
        let mut m = Mirrored::new("desk".into(), clock::now_us());
        m.heard(&old, 0, "me");
        m.foresee(1, &Op::Pause, clock::now_us());
        let paused = DeviceState { seq: 3, playing: false, obeyed: vec![Obeyed { from: "me".into(), id: 1 }], ..old.clone() };
        m.heard(&paused, 1, "me");
        assert!(m.foreseen.is_empty());
        assert!(!m.heard(&old, 2, "me"), "a delayed relay answer cannot replace the newer direct answer");
        assert!(!m.shown.as_ref().unwrap().playing);
    }

    #[test]
    fn the_last_confirmed_command_replaces_earlier_ones() {
        for ops in [vec![Op::Next, Op::Next], vec![Op::Pause, Op::Play]] {
            let entries = (0..3).map(|index| Entry { index, turn: index, duration: 300, ..Default::default() }).collect();
            let old = DeviceState { seq: 1, index: Some(0), len: 3, entries, playing: true, ..Default::default() };
            let mut m = Mirrored::new("desk".into(), clock::now_us());
            m.heard(&old, 0, "me");
            for (k, op) in ops.iter().enumerate() {
                m.foresee(k as u64 + 1, op, clock::now_us());
            }
            let expected = m.shown.clone().unwrap();
            let confirmed = DeviceState { seq: 2, obeyed: vec![Obeyed { from: "me".into(), id: 2 }], ..expected.clone() };
            m.heard(&confirmed, 1, "me");
            assert!(m.foreseen.is_empty());
            let shown = m.shown.as_ref().unwrap();
            assert_eq!((shown.index, shown.playing), (expected.index, expected.playing));
        }
    }

    #[test]
    fn restarted_remote_replaces_its_old_process() {
        let old = DeviceState { instance: Some(1), seq: 80, playing: true, len: 2, rev: 1, ..Default::default() };
        let mut m = Mirrored::new("desk".into(), clock::now_us());
        m.heard(&old, 0, "me");
        m.foresee(1, &Op::Pause, clock::now_us());
        let restarted = DeviceState { instance: Some(2), seq: 1, len: 2, rev: 1, playing: true, ..Default::default() };
        assert!(m.heard(&restarted, 1, "me"));
        assert!(m.foreseen.is_empty());
        assert!(m.shown.as_ref().unwrap().playing);
        assert!(!m.heard(&DeviceState { seq: 81, ..old }, 2, "me"));
        assert!(!m.heard(&DeviceState { seq: 90, ..Default::default() }, 3, "me"));
        assert_eq!(m.heard.as_ref().unwrap().instance, Some(2));
        let mut fresh = Mirrored::new("desk".into(), clock::now_us());
        fresh.heard(&restarted, 0, "me");
        assert!(!fresh.heard(&DeviceState { instance: Some(1), seq: 100, ..Default::default() }, 1, "me"));
    }

    #[test]
    fn only_time_moving_is_not_news() {
        let last = DeviceState { playing: true, position_ms: 10_000, rev: 2, at_us: Some(1_000_000), ..Default::default() };
        let later = |position_ms: i64| DeviceState { position_ms, at_us: Some(6_000_000), ..last.clone() };
        assert!(same_but_time(&last, &later(15_003)));
        assert!(!same_but_time(&last, &later(15_020)), "a place 20 ms off is said again");
        assert!(!same_but_time(&last, &later(40_000)), "a seek");
        assert!(!same_but_time(&last, &DeviceState { rev: 3, ..later(15_000) }), "the queue changed");
        assert!(!same_but_time(&last, &DeviceState { playing: false, ..later(15_000) }), "paused");
        // At another speed: the place runs on at it, and a new speed is news at once.
        let fast = DeviceState { rate: Some(1.25), ..last.clone() };
        assert!(same_but_time(&fast, &DeviceState { position_ms: 16_250, ..DeviceState { at_us: Some(6_000_000), ..fast.clone() } }));
        assert!(!same_but_time(&fast, &DeviceState { rate: Some(1.0), position_ms: 16_250, at_us: Some(6_000_000), ..fast.clone() }), "the speed changed");
    }

    #[test]
    fn a_songs_star_is_read_as_the_mirrored_rows_have_it() {
        let entry = |index: u32, turn: u32, id: &str, starred: bool| Entry { index, turn, id: id.into(), starred, duration: 300, ..Default::default() };
        let window = vec![entry(1, 1, "b", true), entry(2, 2, "c", false)];
        let st = DeviceState { seq: 1, rev: 7, index: Some(1), len: 4, entries: window, ..Default::default() };
        let mut m = Mirrored::new("desk".into(), clock::now_us());
        m.heard(&st, 0, "me");
        let from_rows = |m: &Mirrored, id: &str| m.view("Desk".into(), DeviceKind::Desktop, None).unwrap().rows.iter().find(|r| r.song.id == id).map(|r| r.song.starred);
        let ids = ["a", "b", "c", "d", "e"];
        for id in ids {
            assert_eq!(m.starred(id), from_rows(&m, id), "{id}, the window only");
        }
        m.page(7, 0, vec![entry(0, 0, "a", true), entry(1, 1, "b", false), entry(2, 2, "c", true), entry(3, 3, "d", true)]);
        m.foresee(1, &Op::Star { id: "c".into(), on: true }, clock::now_us());
        for id in ids {
            assert_eq!(m.starred(id), from_rows(&m, id), "{id}, every page in, the window over them");
        }
        assert_eq!((m.starred("a"), m.starred("b"), m.starred("c")), (Some(true), Some(true), Some(true)));
    }

    #[test]
    fn a_mirrored_playhead_runs_on_from_when_it_was_heard_there() {
        let entries = vec![Entry { index: 0, duration: 300, ..Default::default() }];
        let st = DeviceState { seq: 4, playing: true, position_ms: 10_000, index: Some(0), len: 1, entries, at_us: Some(50_000_000), ..Default::default() };
        let mut m = Mirrored::new("desk".into(), clock::now_us());
        let view = |m: &Mirrored| m.view("Desk".into(), DeviceKind::Desktop, None).unwrap();
        // Its clock not known yet: as of when the state arrived.
        assert!(m.heard(&st, 2_000_000, "me"));
        assert_eq!((view(&m).position_ms, view(&m).at_us), (10_000, 2_000_000));
        // The same state read again (every poll answer carries it) does not start the clock again.
        assert!(!m.heard(&st, 3_000_000, "me"));
        assert_eq!(view(&m).at_us, 2_000_000);
        // Its clock 49.2 s ahead of this one's: heard there at 0.8 s here, though it arrived at 2 s.
        m.clock.add(clock::Exchange { t1: 1_000_000, t2: 50_201_000, t3: 50_201_500, t4: 1_002_500 });
        let v = view(&m);
        assert_eq!(v.at_us, 800_000);
        assert_eq!(v.position_at(1_800_000), 11_000);
        assert_eq!(v.position_at(400_000_000), 300_000, "held at the song's end");
        // Playing at 1.25 times: a second there is a second and a quarter of the song.
        assert!(m.heard(&DeviceState { seq: 5, rate: Some(1.25), ..st.clone() }, 3_500_000, "me"));
        assert_eq!(view(&m).position_at(1_800_000), 11_250);
        let stalled = DeviceState { seq: 6, buffering: true, ..st.clone() };
        assert!(m.heard(&stalled, 4_000_000, "me"));
        assert_eq!(view(&m).position_at(9_000_000), 10_000, "waiting for audio holds the playhead");
        assert_eq!(view(&m).rate, 0.0, "client clocks hold while buffering");
        assert!(m.heard(&DeviceState { seq: 7, position_ms: 12_000, at_us: Some(55_000_000), ..st }, 6_000_000, "me"));
        assert_eq!(view(&m).position_at(6_800_000), 13_000, "the playhead resumes from the next heard position");
        // A command foreseen here runs on from when it was sent.
        m.foresee(1, &Op::Pause, clock::now_us());
        assert!(!view(&m).playing && view(&m).position_at(900_000_000) == view(&m).position_ms);
    }

    #[test]
    fn devices_of_one_name_are_told_apart() {
        use DeviceKind::*;
        let words = KindWords { phone: "phone".into(), desktop: "computer".into(), terminal: "terminal".into(), guest: "guest".into() };
        let me = RemoteMe { name: "Mac".into(), kind: Desktop };
        /// (name, kind, id) of each device.
        type Listed = &'static [(&'static str, DeviceKind, &'static str)];
        let cases: &[(Listed, &[&str])] = &[
            (&[("Pixel", Phone, "aa11")], &["Pixel"]),
            (&[("Mac", Terminal, "bb22")], &["Mac (terminal)"]),
            (&[("Mac", Desktop, "3fa29c01")], &["Mac (3fa2)"]),
            (&[("Pixel", Phone, "aa11"), ("Pixel", Terminal, "bb22"), ("Desk", Desktop, "cc33")], &["Pixel (phone)", "Pixel (terminal)", "Desk"]),
            (&[("Mac", Terminal, "1111ab"), ("Mac", Terminal, "2222cd")], &["Mac (1111)", "Mac (2222)"]),
        ];
        for (devices, want) in cases {
            let devices = devices.iter().map(|&(name, kind, id)| RemoteDevice { id: id.into(), name: name.into(), kind, state: None, age_ms: 0, nearby: false, refused: None }).collect();
            assert_eq!(device_names(devices, me.clone(), words.clone()), *want);
        }
    }

    #[test]
    fn a_mix_survives_the_incoming_song_takeover() {
        let (client, _) = crate::client::tests::client(crate::client::NetProfile {
            url: "http://test".into(),
            ..Default::default()
        });
        let session = &client.core.session;
        session.settings.open("").unwrap();
        let mut prefs = session.settings.current().unwrap();
        prefs.crossfade_sec = 4;
        session.settings.put(prefs);
        session.set(
            vec!["a".into(), "b".into(), "c".into()],
            Some(0),
            false,
            None,
        );
        session.planner.transition_window(
            ["a", "b", "c"]
                .into_iter()
                .map(|id| nori_player::transitions::WindowSong {
                    id: id.into(),
                    duration_ms: 30_000,
                    ..Default::default()
                })
                .collect(),
            false,
        );
        let a = session.planner.plan_for("a").unwrap();
        session.planner.plan_for("b").unwrap();
        let remote = Remote::new(
            client,
            RemoteMe {
                name: "Test".into(),
                kind: DeviceKind::Desktop,
            },
            Arc::new(Quiet),
            Arc::new(Quiet),
            None,
        );
        for playing in [true, false] {
            remote.inner.lock().playing = Playing {
                index: Some(1),
                mix_from: Some(0),
                mixing: true,
                playing,
                ..Default::default()
            };
            let playing = remote.inner.lock().playing;
            let along = remote.along(Some(1), playing);
            assert_eq!(
                along.mix.map(|m| Mix {
                    position_ms: None,
                    rate: None,
                    ..m
                }),
                mix_of(0, &a),
                "the displayed song changed, the audible transition did not"
            );
        }
        remote.stop();
    }

    #[test]
    fn a_mirrored_mix_runs_on_without_extra_publications() {
        let plan = engine_plan(
            &nori_player::automix::plan::plan(None, None, 30_000, 30_000, &Default::default()),
            "b",
        )
        .unwrap();
        let mut mix = mix_of(0, &plan).unwrap();
        mix.position_ms = Some(28_000);
        mix.rate = Some(1.0);
        let last = DeviceState {
            index: Some(1),
            len: 2,
            entries: vec![
                Entry {
                    id: "a".into(),
                    duration: 30,
                    ..Default::default()
                },
                Entry {
                    index: 1,
                    id: "b".into(),
                    duration: 30,
                    ..Default::default()
                },
            ],
            playing: true,
            mixing: true,
            position_ms: 2_000,
            rate: Some(1.04),
            at_us: Some(1_000_000),
            jam: Some(JamState {
                along: Some(Along {
                    speed: 1.0,
                    pitch: 1.0,
                    mix: Some(mix),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut now = DeviceState {
            position_ms: 3_040,
            at_us: Some(2_000_000),
            ..last.clone()
        };
        now.jam
            .as_mut()
            .unwrap()
            .along
            .as_mut()
            .unwrap()
            .mix
            .as_mut()
            .unwrap()
            .position_ms = Some(29_000);
        assert!(same_but_time(&last, &now));
        let mut mirrored = Mirrored::new("host".into(), clock::now_us());
        mirrored.heard(&now, 2_000_000, "guest");
        assert!(
            mirrored
                .view("Host".into(), DeviceKind::Phone, None)
                .unwrap()
                .mixing
        );
        assert!(
            !same_but_time(
                &now,
                &DeviceState {
                    mixing: false,
                    ..now.clone()
                }
            ),
            "the mix ending must be published"
        );
        assert!(
            !same_but_time(
                &now,
                &DeviceState {
                    playing: false,
                    ..now.clone()
                }
            ),
            "a pause must be published"
        );
    }

    #[test]
    fn a_planned_mix_reaches_guests_whole() {
        let settings = nori_player::types::AutoMixSettings { max_transition_s: 8.0, ..Default::default() };
        let mut t = nori_player::automix::plan::plan(None, None, 200_000, 180_000, &settings);
        t.tempo_ratio = 1.03;
        t.tempo_ramp_ms = 2_500;
        let plan = engine_plan(&t, "s2").expect("a mix");
        let entries = [Entry { index: 4, id: "s1".into(), ..Default::default() }, Entry { index: 5, id: "s2".into(), ..Default::default() }];
        let mix = mix_of(4, &plan).unwrap();
        let wire: Mix = serde_json::from_str(&serde_json::to_string(&mix).unwrap()).unwrap();
        assert_eq!(plan_of(&wire, &entries), Some(("s1".to_string(), plan)));
        assert_eq!(plan_of(&Mix { from: 9, ..wire }, &entries), None, "not in the queue window");
    }

    #[test]
    fn a_server_without_a_relay_answers_with_an_error() {
        assert!(answer(br#"{"subsonic-response":{"status":"failed","error":{"code":0}}}"#).is_none());
        assert_eq!(answer(br#"{"seq":3,"you":"a"}"#).map(|a| a.seq), Some(3));
    }
}
