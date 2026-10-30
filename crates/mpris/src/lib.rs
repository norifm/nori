//! The system's media controls: MPRIS (`org.mpris.MediaPlayer2`) on the Linux session bus (zbus), served on
//! one thread with async-io's poller beside it, its position read on request; on macOS, Now Playing and the media keys (MediaPlayer's info and
//! command centers), told the position and rate at each change and running on from there by themselves.
//! Nothing ticks.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Client callbacks, called on the bus thread.
pub trait Controls: Send + Sync + 'static {
    fn play(&self);
    fn pause(&self);
    fn toggle(&self);
    fn next(&self);
    fn previous(&self);
    /// Seeks the current song to `ms`.
    fn seek(&self, ms: i64);
    /// Hearts the song playing, or takes its heart away; controls that cannot star leave it.
    fn like(&self) {}
    fn now(&self) -> Now;
}

/// Playback state and current song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Now {
    pub playing: bool,
    /// A song is loaded: paused rather than stopped.
    pub loaded: bool,
    /// Queue index, used as the track id.
    pub index: Option<usize>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length_ms: i64,
    pub position_ms: i64,
    pub starred: bool,
    /// The cover's file (JPEG, PNG, WebP), once read; Now Playing draws it.
    pub art: Option<Arc<[u8]>>,
    /// Where the cover's file lies on disk, once read; MPRIS hands it out as `mpris:artUrl`.
    pub art_file: Option<PathBuf>,
}

/// What the system's controls show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Playing,
    Paused,
    Stopped,
}

impl Now {
    /// By the state alone: a song whose details are still on their way (another device's queue) plays.
    fn status(&self) -> Status {
        match (self.playing, self.loaded) {
            (true, _) => Status::Playing,
            (false, true) => Status::Paused,
            _ => Status::Stopped,
        }
    }
}

#[cfg(target_os = "linux")]
pub use bus::{Commands, Run};

#[cfg(any(target_os = "linux", all(test, unix)))]
mod bus {
    use std::collections::HashMap;
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::{mpsc, Arc};

    use futures_lite::future::block_on;
    use zbus::connection::Builder;
    use zbus::interface;
    use zbus::names::BusName;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
    use zbus::Connection;

    use super::{Controls, Now, Status};

    const OBJECT: &str = "/org/mpris/MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

    fn status(n: &Now) -> &'static str {
        match n.status() {
            Status::Playing => "Playing",
            Status::Paused => "Paused",
            Status::Stopped => "Stopped",
        }
    }

    fn track_path(n: &Now) -> OwnedObjectPath {
        let path = match n.index {
            Some(i) => format!("/org/nori/track/{i}"),
            None => "/org/mpris/MediaPlayer2/TrackList/NoTrack".into(),
        };
        OwnedObjectPath::try_from(path).expect("a valid path")
    }

    fn metadata(n: &Now) -> HashMap<&'static str, Value<'static>> {
        let mut m = HashMap::from([("mpris:trackid", Value::from(track_path(n).into_inner()))]);
        if n.index.is_some() {
            m.insert("mpris:length", Value::from(n.length_ms * 1000));
            m.insert("xesam:title", Value::from(n.title.clone()));
            m.insert("xesam:artist", Value::from(vec![n.artist.clone()]));
            m.insert("xesam:album", Value::from(n.album.clone()));
            if let Some(file) = &n.art_file {
                m.insert("mpris:artUrl", Value::from(file_url(file)));
            }
        }
        m
    }

    /// `path` as a `file://` URL, its bytes outside the unreserved set percent-encoded.
    fn file_url(path: &Path) -> String {
        let mut url = String::from("file://");
        for &b in path.as_os_str().as_bytes() {
            if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
                url.push(b as char);
            } else {
                let _ = write!(url, "%{b:02X}");
            }
        }
        url
    }

    struct Root {
        identity: String,
    }

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {}
        fn quit(&self) {}
        #[zbus(property)]
        fn identity(&self) -> String {
            self.identity.clone()
        }
        #[zbus(property)]
        fn can_quit(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn can_raise(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            Vec::new()
        }
        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            Vec::new()
        }
    }

    struct Player(Arc<dyn Controls>);

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn play(&self) {
            self.0.play();
        }
        fn pause(&self) {
            self.0.pause();
        }
        fn play_pause(&self) {
            self.0.toggle();
        }
        fn stop(&self) {
            self.0.pause();
        }
        fn next(&self) {
            self.0.next();
        }
        fn previous(&self) {
            self.0.previous();
        }
        fn seek(&self, offset: i64) {
            let at = self.0.now().position_ms + offset / 1000;
            self.0.seek(at.max(0));
        }
        fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
            // Ignored for a stale track id.
            if track_id == *track_path(&self.0.now()) {
                self.0.seek(position / 1000);
            }
        }
        #[zbus(property)]
        fn playback_status(&self) -> String {
            status(&self.0.now()).into()
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
            metadata(&self.0.now())
        }
        #[zbus(property(emits_changed_signal = "false"))]
        fn position(&self) -> i64 {
            self.0.now().position_ms * 1000
        }
        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn volume(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    /// The session bus. Its Unix socket is connected here: zbus would connect it on the `blocking` crate's
    /// pool, whose thread then stays and wakes twice a second.
    #[cfg(target_os = "linux")]
    fn session() -> zbus::Result<Builder<'static>> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixStream};
        use zbus::address::transport::{Transport, UnixSocket};

        let address = zbus::Address::session()?;
        let Transport::Unix(unix) = address.transport() else { return Builder::address(address) };
        let stream = match unix.path() {
            UnixSocket::File(path) => UnixStream::connect(path)?,
            UnixSocket::Abstract(name) => UnixStream::connect_addr(&SocketAddr::from_abstract_name(name.as_encoded_bytes())?)?,
            _ => return Builder::address(address),
        };
        Ok(Builder::async_io_unix_stream(stream))
    }

    /// `builder`'s connection, its executor run on a thread of its own named `name`, where its objects
    /// are called.
    fn running(builder: Builder<'static>, name: &str) -> Result<Connection, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                // futures-lite's block_on leaves the socket to async-io's thread, which sleeps in the
                // poller between messages; async-io's own block_on would have it nap in a backoff loop.
                block_on(async move {
                    let conn = match builder.build().await {
                        Ok(conn) => conn,
                        Err(e) => return drop(tx.send(Err(e.to_string()))),
                    };
                    let executor = conn.executor().clone();
                    let _ = tx.send(Ok(conn));
                    while !executor.is_empty() {
                        executor.tick().await;
                    }
                })
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    /// What another app's `Run(plugin, command)` reaches.
    pub type Run = Box<dyn Fn(String, String) + Send + Sync>;

    struct Runner(Run);

    #[interface(name = "fm.nori.Commands")]
    impl Runner {
        fn run(&self, plugin: String, command: String) {
            (self.0)(plugin, command);
        }
    }

    const COMMANDS: &str = "/fm/nori/Commands";

    /// Plugins' commands other apps run: `fm.nori.Commands` on the session bus, `Run(plugin, command)`
    /// at `/fm/nori/Commands`. The name is let go when this is dropped.
    pub struct Commands {
        conn: Connection,
    }

    impl Commands {
        #[cfg(target_os = "linux")]
        pub fn start(run: Run) -> Result<Commands, String> {
            Commands::on(session().map_err(|e| e.to_string())?, run, true)
        }

        fn on(builder: Builder<'static>, run: Run, named: bool) -> Result<Commands, String> {
            let builder = builder.internal_executor(false).serve_at(COMMANDS, Runner(run)).map_err(|e| e.to_string())?;
            let conn = running(builder, "nori-commands")?;
            if named {
                block_on(conn.request_name("fm.nori.Commands")).map_err(|e| e.to_string())?;
            }
            Ok(Commands { conn })
        }
    }

    impl Drop for Commands {
        fn drop(&mut self) {
            let _ = block_on(self.conn.object_server().remove::<Runner, _>(COMMANDS));
            let _ = block_on(self.conn.release_name("fm.nori.Commands"));
        }
    }

    /// The objects on a connection whose executor runs on the "nori-mpris" thread, where the controls are called.
    pub struct Served {
        conn: Connection,
        controls: Arc<dyn Controls>,
    }

    impl Served {
        #[cfg(target_os = "linux")]
        pub fn start(name: &str, controls: Arc<dyn Controls>) -> Result<Served, String> {
            let served = Served::on(session().map_err(|e| e.to_string())?, name, controls)?;
            // Without DoNotQueue: waits in line behind an owner that keeps the name.
            let flags = zbus::fdo::RequestNameFlags::ReplaceExisting.into();
            block_on(served.conn.request_name_with_flags(format!("org.mpris.MediaPlayer2.{name}"), flags)).map_err(|e| e.to_string())?;
            Ok(served)
        }

        fn on(builder: Builder<'static>, identity: &str, controls: Arc<dyn Controls>) -> Result<Served, String> {
            let builder = builder
                .internal_executor(false)
                .serve_at(OBJECT, Root { identity: identity.to_string() })
                .and_then(|b| b.serve_at(OBJECT, Player(controls.clone())))
                .map_err(|e| e.to_string())?;
            Ok(Served { conn: running(builder, "nori-mpris")?, controls })
        }

        pub fn changed(&self) {
            let n = self.controls.now();
            let changed = HashMap::from([("PlaybackStatus", Value::from(status(&n))), ("Metadata", Value::from(metadata(&n)))]);
            let body = (PLAYER, changed, Vec::<&str>::new());
            let _ = block_on(self.conn.emit_signal(None::<BusName>, OBJECT, "org.freedesktop.DBus.Properties", "PropertiesChanged", &body));
        }

        /// Emits Seeked with the position now, µs.
        pub fn seeked(&self) {
            let us = self.controls.now().position_ms * 1000;
            let _ = block_on(self.conn.emit_signal(None::<BusName>, OBJECT, PLAYER, "Seeked", &us));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::net::UnixStream;
        use std::sync::Mutex;

        use futures_lite::StreamExt;
        use zbus::message::Type;
        use zbus::zvariant::OwnedValue;
        use zbus::{Guid, Message, MessageStream};

        #[derive(Default)]
        struct Asked {
            now: Mutex<Now>,
            asked: Mutex<Vec<String>>,
        }

        impl Asked {
            fn ask(&self, what: String) {
                self.asked.lock().unwrap().push(what);
            }
        }

        impl Controls for Asked {
            fn play(&self) {
                self.ask("play".into());
            }
            fn pause(&self) {
                self.ask("pause".into());
            }
            fn toggle(&self) {
                self.ask("toggle".into());
            }
            fn next(&self) {
                self.ask("next".into());
            }
            fn previous(&self) {
                self.ask("previous".into());
            }
            fn seek(&self, ms: i64) {
                self.ask(format!("seek {ms}"));
            }
            fn now(&self) -> Now {
                self.now.lock().unwrap().clone()
            }
        }

        fn song() -> Now {
            Now { playing: true, loaded: true, index: Some(2), title: "T".into(), artist: "A".into(), album: "B".into(), length_ms: 200_000, position_ms: 30_000, ..Now::default() }
        }

        /// `controls` served over a socket pair, and the client on its other end.
        fn served(controls: Arc<Asked>) -> (Served, Connection) {
            let (server, client) = UnixStream::pair().unwrap();
            let guid = Guid::generate();
            let served = std::thread::spawn(move || Served::on(Builder::async_io_unix_stream(server).server(guid).unwrap().p2p(), "nori", controls));
            let client = block_on(Builder::async_io_unix_stream(client).p2p().build()).unwrap();
            (served.join().unwrap().unwrap(), client)
        }

        fn call<B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType>(client: &Connection, interface: &str, method: &str, body: &B) -> zbus::Result<Message> {
            block_on(client.call_method(None::<BusName>, OBJECT, Some(interface), method, body))
        }

        fn get(client: &Connection, interface: &str, property: &str) -> OwnedValue {
            call(client, "org.freedesktop.DBus.Properties", "Get", &(interface, property)).unwrap().body().deserialize().unwrap()
        }

        fn metadata_of(value: OwnedValue) -> HashMap<String, OwnedValue> {
            value.try_into().unwrap()
        }

        fn path(p: &str) -> ObjectPath<'_> {
            ObjectPath::try_from(p).unwrap()
        }

        #[test]
        fn another_apps_command_reaches_its_plugin() {
            let (server, client) = UnixStream::pair().unwrap();
            let guid = Guid::generate();
            let ran = Arc::new(Mutex::new(Vec::new()));
            let into = ran.clone();
            let run: Run = Box::new(move |plugin, command| into.lock().unwrap().push(format!("{plugin} {command}")));
            let served = std::thread::spawn(move || Commands::on(Builder::async_io_unix_stream(server).server(guid).unwrap().p2p(), run, false));
            let client = block_on(Builder::async_io_unix_stream(client).p2p().build()).unwrap();
            let _commands = served.join().unwrap().unwrap();
            block_on(client.call_method(None::<BusName>, COMMANDS, Some("fm.nori.Commands"), "Run", &("fm.nori.test", "ping"))).unwrap();
            assert_eq!(*ran.lock().unwrap(), ["fm.nori.test ping"]);
        }

        #[test]
        fn calls_reach_the_controls() {
            let asked = Arc::new(Asked { now: Mutex::new(song()), ..Asked::default() });
            let (_served, client) = served(asked.clone());
            for method in ["Play", "Pause", "PlayPause", "Stop", "Next", "Previous"] {
                call(&client, PLAYER, method, &()).unwrap();
            }
            call(&client, PLAYER, "Seek", &5_000_000i64).unwrap();
            call(&client, PLAYER, "Seek", &-60_000_000i64).unwrap();
            call(&client, PLAYER, "SetPosition", &(path("/org/nori/track/2"), 90_000_000i64)).unwrap();
            call(&client, PLAYER, "SetPosition", &(path("/org/nori/track/1"), 10_000_000i64)).unwrap();
            call(&client, "org.mpris.MediaPlayer2", "Raise", &()).unwrap();
            call(&client, "org.mpris.MediaPlayer2", "Quit", &()).unwrap();
            let asked = asked.asked.lock().unwrap().clone();
            assert_eq!(asked, ["play", "pause", "toggle", "pause", "next", "previous", "seek 35000", "seek 0", "seek 90000"]);
        }

        #[test]
        fn properties_read_the_song_now() {
            let asked = Arc::new(Asked { now: Mutex::new(song()), ..Asked::default() });
            let (_served, client) = served(asked.clone());
            assert_eq!(String::try_from(get(&client, "org.mpris.MediaPlayer2", "Identity")).unwrap(), "nori");
            assert_eq!(i64::try_from(get(&client, PLAYER, "Position")).unwrap(), 30_000_000, "µs");
            let m = metadata_of(get(&client, PLAYER, "Metadata"));
            assert_eq!(ObjectPath::try_from(m["mpris:trackid"].clone()).unwrap().as_str(), "/org/nori/track/2");
            assert_eq!(i64::try_from(m["mpris:length"].clone()).unwrap(), 200_000_000);
            assert_eq!(String::try_from(m["xesam:title"].clone()).unwrap(), "T");
            assert_eq!(Vec::<String>::try_from(m["xesam:artist"].clone()).unwrap(), ["A"]);
            assert_eq!(String::try_from(m["xesam:album"].clone()).unwrap(), "B");
            for (playing, loaded, index, status) in [(true, true, Some(2), "Playing"), (false, true, Some(2), "Paused"), (false, false, None, "Stopped")] {
                *asked.now.lock().unwrap() = Now { playing, loaded, index, ..song() };
                assert_eq!(String::try_from(get(&client, PLAYER, "PlaybackStatus")).unwrap(), status);
            }
            let m = metadata_of(get(&client, PLAYER, "Metadata"));
            assert_eq!(m.keys().collect::<Vec<_>>(), ["mpris:trackid"], "no song, no tags");
            assert_eq!(ObjectPath::try_from(m["mpris:trackid"].clone()).unwrap().as_str(), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
        }

        #[test]
        fn changed_signals_status_and_metadata() {
            let asked = Arc::new(Asked { now: Mutex::new(Now { playing: false, ..song() }), ..Asked::default() });
            let (served, client) = served(asked);
            let mut stream = MessageStream::from(&client);
            served.changed();
            let signal = block_on(stream.find(|m| m.as_ref().is_ok_and(|m| m.message_type() == Type::Signal))).unwrap().unwrap();
            let header = signal.header();
            assert_eq!((header.path().unwrap().as_str(), header.member().unwrap().as_str()), (OBJECT, "PropertiesChanged"));
            let (interface, changed, invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) = signal.body().deserialize().unwrap();
            assert_eq!((interface.as_str(), invalidated.len()), (PLAYER, 0));
            assert_eq!(String::try_from(changed["PlaybackStatus"].clone()).unwrap(), "Paused");
            let m = metadata_of(changed["Metadata"].clone());
            assert_eq!(String::try_from(m["xesam:title"].clone()).unwrap(), "T");
            assert_eq!(changed.len(), 2);
        }

        fn next_signal(stream: &mut MessageStream) -> Message {
            block_on(stream.find(|m| m.as_ref().is_ok_and(|m| m.message_type() == Type::Signal))).unwrap().unwrap()
        }

        /// `url`'s path with its percent escapes decoded.
        fn path_of(url: &str) -> std::path::PathBuf {
            let bytes = url.strip_prefix("file://").expect("a file URL").as_bytes();
            let (mut out, mut i) = (Vec::new(), 0);
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16).unwrap());
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            std::path::PathBuf::from(String::from_utf8(out).unwrap())
        }

        #[test]
        fn art_url_is_the_cover_file_and_follows_the_song() {
            let dir = nori_testdir::TempDir::new("mpris-art");
            let (first, second) = (dir.path().join("cover one%.jpg"), dir.path().join("two.png"));
            std::fs::write(&first, b"jpeg").unwrap();
            std::fs::write(&second, b"png").unwrap();
            let asked = Arc::new(Asked { now: Mutex::new(Now { art_file: Some(first.clone()), ..song() }), ..Asked::default() });
            let (served, client) = served(asked.clone());
            let url = String::try_from(metadata_of(get(&client, PLAYER, "Metadata"))["mpris:artUrl"].clone()).unwrap();
            assert!(!url.contains(' '), "{url}");
            assert_eq!(std::fs::read(path_of(&url)).unwrap(), b"jpeg");

            let mut stream = MessageStream::from(&client);
            *asked.now.lock().unwrap() = Now { index: Some(3), art_file: Some(second), ..song() };
            served.changed();
            let (_, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) = next_signal(&mut stream).body().deserialize().unwrap();
            let url = String::try_from(metadata_of(changed["Metadata"].clone())["mpris:artUrl"].clone()).unwrap();
            assert_eq!(std::fs::read(path_of(&url)).unwrap(), b"png");

            *asked.now.lock().unwrap() = Now { index: Some(4), ..song() };
            assert!(!metadata_of(get(&client, PLAYER, "Metadata")).contains_key("mpris:artUrl"), "a song whose cover is not read yet");
        }

        #[test]
        fn seeked_signals_each_jump_in_microseconds() {
            let asked = Arc::new(Asked { now: Mutex::new(song()), ..Asked::default() });
            let (served, client) = served(asked.clone());
            let mut stream = MessageStream::from(&client);
            served.seeked();
            asked.now.lock().unwrap().position_ms = 0;
            served.changed();
            served.seeked();
            let mut seen = Vec::new();
            for _ in 0..3 {
                let signal = next_signal(&mut stream);
                let header = signal.header();
                let member = header.member().unwrap().to_string();
                let us = (member == "Seeked").then(|| signal.body().deserialize::<i64>().unwrap());
                assert_eq!(header.interface().unwrap().as_str(), if us.is_some() { PLAYER } else { "org.freedesktop.DBus.Properties" });
                seen.push((member, us));
            }
            let seeked = |us| ("Seeked".to_string(), Some(us));
            assert_eq!(seen, [seeked(30_000_000), ("PropertiesChanged".into(), None), seeked(0)]);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::sync::Arc;

    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::NSImage;
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
        MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
        MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
    };

    use super::{Controls, Current, Now, Status};

    /// A cover's file and the artwork made of it.
    type Made = (Arc<[u8]>, Retained<MPMediaItemArtwork>);

    thread_local! {
        /// The cover Now Playing shows, made once per song. On the main thread: AppKit's objects stay there.
        static ART: RefCell<Option<Made>> = const { RefCell::new(None) };
    }

    type Act = fn(&Current, NonNull<MPRemoteCommandEvent>);

    /// Hands the media keys and Now Playing's buttons to `current`. On the main thread, once.
    pub fn listen(current: Arc<Current>) {
        // SAFETY: the shared command center lives for the process, and copies each handler block.
        unsafe {
            let center = MPRemoteCommandCenter::sharedCommandCenter();
            let on = |command: &MPRemoteCommand, act: Act| {
                let current = current.clone();
                let block = RcBlock::new(move |e: NonNull<MPRemoteCommandEvent>| {
                    act(&current, e);
                    MPRemoteCommandHandlerStatus::Success
                });
                command.setEnabled(true);
                command.addTargetWithHandler(&block);
            };
            on(&center.playCommand(), |c, _| c.play());
            on(&center.pauseCommand(), |c, _| c.pause());
            on(&center.togglePlayPauseCommand(), |c, _| c.toggle());
            on(&center.nextTrackCommand(), |c, _| c.next());
            on(&center.previousTrackCommand(), |c, _| c.previous());
            on(&center.changePlaybackPositionCommand(), |c, e| {
                // This command's events are position changes.
                let at = e.cast::<MPChangePlaybackPositionCommandEvent>().as_ref().positionTime();
                c.seek((at * 1000.0) as i64);
            });
            on(&center.likeCommand(), |c, _| c.like());
        }
    }

    /// Tells Now Playing what `current` plays now, on the main thread.
    pub fn show(current: Arc<Current>) {
        DispatchQueue::main().exec_async(move || set(&current.now()));
    }

    fn set(n: &Now) {
        // SAFETY: on the main thread; the keys are MediaPlayer's constants, each with a value of its type.
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            MPRemoteCommandCenter::sharedCommandCenter().likeCommand().setActive(n.starred);
            if n.status() == Status::Stopped {
                ART.with(|a| a.borrow_mut().take());
                center.setNowPlayingInfo(None);
                center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
                return;
            }
            let info = NSMutableDictionary::<NSString, AnyObject>::new();
            let text = |key: &NSString, v: &str| info.insert(key, AsRef::<AnyObject>::as_ref(&*NSString::from_str(v)));
            let number = |key: &NSString, v: f64| info.insert(key, AsRef::<AnyObject>::as_ref(&*NSNumber::new_f64(v)));
            text(MPMediaItemPropertyTitle, &n.title);
            text(MPMediaItemPropertyArtist, &n.artist);
            text(MPMediaItemPropertyAlbumTitle, &n.album);
            number(MPMediaItemPropertyPlaybackDuration, n.length_ms as f64 / 1000.0);
            number(MPNowPlayingInfoPropertyElapsedPlaybackTime, n.position_ms.max(0) as f64 / 1000.0);
            number(MPNowPlayingInfoPropertyPlaybackRate, if n.playing { 1.0 } else { 0.0 });
            if let Some(art) = n.art.as_ref().and_then(artwork) {
                info.insert(MPMediaItemPropertyArtwork, AsRef::<AnyObject>::as_ref(&*art));
            }
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(if n.status() == Status::Playing { MPNowPlayingPlaybackState::Playing } else { MPNowPlayingPlaybackState::Paused });
        }
    }

    /// The cover `file` as Now Playing's artwork, made again only for another cover.
    fn artwork(file: &Arc<[u8]>) -> Option<Retained<MPMediaItemArtwork>> {
        ART.with(|a| {
            let mut a = a.borrow_mut();
            if let Some((_, art)) = a.as_ref().filter(|(made, _)| Arc::ptr_eq(made, file)) {
                return Some(art.clone());
            }
            // SAFETY: on the main thread; the block hands back the image it holds, alive as long as the block.
            let art = unsafe {
                let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(file))?;
                let size = image.size();
                let block = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
                MPMediaItemArtwork::initWithBoundsSize_requestHandler(MPMediaItemArtwork::alloc(), size, &block)
            };
            *a = Some((file.clone(), art.clone()));
            Some(art)
        })
    }
}

/// The controls of the session open now; nothing while none is.
#[derive(Default)]
struct Current(RwLock<Option<Arc<dyn Controls>>>);

impl Current {
    fn with(&self, f: impl FnOnce(&dyn Controls)) {
        if let Some(c) = self.0.read().unwrap_or_else(|p| p.into_inner()).as_deref() {
            f(c);
        }
    }
}

impl Controls for Current {
    fn play(&self) {
        self.with(|c| c.play());
    }
    fn pause(&self) {
        self.with(|c| c.pause());
    }
    fn toggle(&self) {
        self.with(|c| c.toggle());
    }
    fn next(&self) {
        self.with(|c| c.next());
    }
    fn previous(&self) {
        self.with(|c| c.previous());
    }
    fn seek(&self, ms: i64) {
        self.with(|c| c.seek(ms));
    }
    fn like(&self) {
        self.with(|c| c.like());
    }
    fn now(&self) -> Now {
        let mut now = Now::default();
        self.with(|c| now = c.now());
        now
    }
}

/// The process's media controls: one bus name on Linux (`org.mpris.MediaPlayer2.<name>`), Now Playing
/// on macOS, whichever session is open.
pub struct Mpris {
    current: Arc<Current>,
    #[cfg(target_os = "linux")]
    served: bus::Served,
}

impl Mpris {
    /// Errors without a session bus, or off Linux.
    pub fn start(name: &str) -> Result<Mpris, String> {
        let current = Arc::new(Current::default());
        #[cfg(target_os = "linux")]
        return Ok(Mpris { served: bus::Served::start(name, current.clone())?, current });
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (name, current);
            Err("MPRIS is Linux's".into())
        }
    }

    /// [`Mpris::start`] for a windowed app, whose main thread runs the system's event loop: on macOS, Now
    /// Playing and the media keys (their commands arrive on that loop). Called on the main thread.
    pub fn for_app(name: &str) -> Result<Mpris, String> {
        #[cfg(target_os = "macos")]
        {
            let _ = name;
            let current = Arc::new(Current::default());
            mac::listen(current.clone());
            Ok(Mpris { current })
        }
        #[cfg(not(target_os = "macos"))]
        Mpris::start(name)
    }

    /// Media keys drive `controls` from now on; None while no session is open.
    pub fn serve(&self, controls: Option<Arc<dyn Controls>>) {
        *self.current.0.write().unwrap_or_else(|p| p.into_inner()) = controls;
        self.changed();
    }

    /// The song or the playback changed: emits PropertiesChanged for status and metadata (Linux), or
    /// tells Now Playing (macOS).
    pub fn changed(&self) {
        #[cfg(target_os = "linux")]
        self.served.changed();
        #[cfg(target_os = "macos")]
        mac::show(self.current.clone());
    }

    /// The position jumped: emits Seeked (Linux), or tells Now Playing the place (macOS).
    pub fn seeked(&self) {
        #[cfg(target_os = "linux")]
        self.served.seeked();
        #[cfg(target_os = "macos")]
        mac::show(self.current.clone());
    }
}

#[cfg(test)]
mod current_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Presses(Mutex<u32>);

    impl Controls for Presses {
        fn play(&self) {
            *self.0.lock().unwrap() += 1;
        }
        fn pause(&self) {}
        fn toggle(&self) {}
        fn next(&self) {}
        fn previous(&self) {}
        fn seek(&self, _: i64) {}
        fn now(&self) -> Now {
            Now { playing: true, ..Now::default() }
        }
    }

    #[test]
    fn keys_reach_the_session_open_now() {
        let current = Current::default();
        current.play();
        assert!(!current.now().playing, "no session");
        let (old, new) = (Arc::new(Presses::default()), Arc::new(Presses::default()));
        *current.0.write().unwrap() = Some(old.clone());
        current.play();
        *current.0.write().unwrap() = Some(new.clone());
        current.play();
        current.play();
        assert_eq!((*old.0.lock().unwrap(), *new.0.lock().unwrap()), (1, 2));
        assert!(current.now().playing);
    }

    #[test]
    fn a_song_whose_details_are_on_their_way_still_plays() {
        assert_eq!(Now { playing: true, index: None, ..Now::default() }.status(), Status::Playing);
        assert_eq!(Now { loaded: true, ..Now::default() }.status(), Status::Paused);
        assert_eq!(Now::default().status(), Status::Stopped);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use futures_lite::future::block_on;
    use zbus::names::BusName;

    #[derive(Default)]
    struct Asked(Mutex<Vec<String>>);

    impl Controls for Asked {
        fn play(&self) {}
        fn pause(&self) {}
        fn toggle(&self) {
            self.0.lock().unwrap().push("toggle".into());
        }
        fn next(&self) {}
        fn previous(&self) {}
        fn seek(&self, _: i64) {}
        fn now(&self) -> Now {
            Now { playing: true, loaded: true, index: Some(2), ..Now::default() }
        }
    }

    #[test]
    #[ignore = "needs a D-Bus session bus"]
    fn owns_its_name_on_the_session_bus() {
        let client = block_on(zbus::Connection::session()).unwrap();
        let name = format!("nori_test_{}", std::process::id());
        let asked = Arc::new(Asked::default());
        let m = Mpris::start(&name).unwrap();
        m.serve(Some(asked.clone()));
        let to = BusName::try_from(format!("org.mpris.MediaPlayer2.{name}")).unwrap();
        block_on(client.call_method(Some(to), "/org/mpris/MediaPlayer2", Some("org.mpris.MediaPlayer2.Player"), "PlayPause", &())).unwrap();
        assert_eq!(*asked.0.lock().unwrap(), ["toggle"]);
    }
}
