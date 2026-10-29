//! The session, its hooks, timers, cache, assets, wakes and commands, as tests/session.rs drives them.
//! What the plugin finds it says as a notice: the action's id (or "heard" for an event) and a line.

use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::notices::{show, Tone};
use nori_plugin_sdk::timers::When;
use nori_plugin_sdk::types::{Item, ItemKind};
use nori_plugin_sdk::{assets, cache, session, sign_in, subsonic, timers, wake, Asset, Hook, Permission, Wake};

pub const SIGNIN: &[&str] = &["accounts.example"];
pub const PERMISSIONS: &[Permission] = &[Permission::PlaybackRead, Permission::PlaybackControl, Permission::LibraryRead, Permission::Subsonic];
pub const HOOKS: &[Hook] = &[Hook::PlaybackStarted, Hook::PlaybackPaused, Hook::SongChanged, Hook::QueueChanged, Hook::FavoriteChanged, Hook::LyricLine, Hook::DeviceConnected, Hook::OutputPicker];
pub const WAKE: &[Wake] = &[Wake::Boot];
pub const COMMANDS: &[&str] = &["ping"];
pub const CACHE_MB: u32 = 1;
pub const ASSETS: &[Asset] = &[Asset { name: "blob", url: "https://allowed.example/blob", size: 11, sha256: "84293ed06cb3210e7d549afec3140d0c48494416ad25b7f25196afffaa5eb796" }];

fn say(what: &str, line: String) {
    show(Tone::Info, what, &[line]);
}

/// `ErrorKind::Refused` as `Refused`.
fn plain(v: impl std::fmt::Debug) -> String {
    let said = format!("{v:?}");
    said.rsplit("::").next().unwrap_or_default().to_string()
}

fn kind(e: &nori_plugin_sdk::types::Error) -> String {
    plain(e.kind)
}

/// Runs action `id` when it is one of these; false otherwise.
pub fn action(id: &str) -> bool {
    match id {
        "now" => {
            let n = session::now();
            let title = n.song.map(|s| s.title).unwrap_or_default();
            say(id, format!("{title} {:?} {} {} {}", n.index, n.playing, n.position_ms, session::queue_length()));
        }
        "queue" => say(id, session::queue(1, 5).into_iter().map(|s| s.title).collect::<Vec<_>>().join(",")),
        "add" => say(id, format!("{:?}", session::add(&["a".into(), "b".into()], true).map_err(|e| kind(&e)))),
        "add-provider" => say(id, format!("{:?}", session::add(&["ext-1".into()], false).map_err(|e| kind(&e)))),
        "next" => session::next(),
        "search" => {
            let found = session::search("blue", 5).map(|items| items.into_iter().filter_map(|i| if let Item::Song(s) = i { Some(s.title) } else { None }).collect::<Vec<_>>());
            say(id, format!("{:?}", found.map_err(|e| kind(&e))));
        }
        "star" => say(id, format!("{:?}", session::star(ItemKind::Song, "a", true).map_err(|e| kind(&e)))),
        "subsonic" => say(id, format!("{:?}", subsonic::call("getUser", &[("username".into(), "u".into())]).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| kind(&e)))),
        "subsonic-path" => say(id, format!("{:?}", subsonic::call("../x", &[]).map(|_| ()).map_err(|e| kind(&e)))),
        "every-second" => timers::start(1, 1000, true, When::Always),
        "visible" => timers::start(2, 1000, true, When::WhileVisible),
        "once" => timers::start(3, 500, false, When::Always),
        "stop" => timers::stop(1),
        "cache" => {
            let put = cache::put("k", b"kept").map_err(|e| kind(&e));
            let big = cache::put("big", &vec![0; 2 << 20]).map_err(|e| kind(&e));
            let got = cache::get("k").map(|b| String::from_utf8_lossy(&b).into_owned());
            say(id, format!("{put:?} {big:?} {got:?} {}", cache::path("k").is_some()));
        }
        "asset" => assets::fetch("blob"),
        "asset-read" => say(id, format!("{} {:?}", plain(assets::state("blob")), assets::read("blob").map(|b| String::from_utf8_lossy(&b).into_owned()))),
        "wake" => wake::want(&["boot".into(), "bluetooth-connected".into()]),
        "sleep" => wake::want(&[]),
        "ping" => say(id, "pong".into()),
        "sign-in" => say(id, format!("{:?} {:?}", sign_in::open("https://accounts.example/start").map_err(|e| kind(&e)), sign_in::open("https://elsewhere.example/").map_err(|e| kind(&e)))),
        _ => return false,
    }
    true
}

/// Says each event it hears as a notice "heard".
pub fn on(event: Event) {
    let line = match event {
        Event::PlaybackStarted => "started".into(),
        Event::PlaybackPaused => "paused".into(),
        Event::SongChanged(s) => format!("song {}", s.map(|s| s.title).unwrap_or_default()),
        Event::QueueChanged => "queue".into(),
        Event::FavoriteChanged((id, on)) => format!("favorite {id} {on}"),
        Event::LyricLine((i, words)) => format!("lyric {i} {words}"),
        Event::Timer(id) => format!("timer {id}"),
        Event::AssetReady(name) => format!("asset {name}"),
        Event::Woke(name) => format!("woke {name}"),
        Event::DeviceConnected(name) => format!("device {name}"),
        other => format!("{other:?}"),
    };
    say("heard", line);
}
