//! `nori-cli --sync`: pairing, profile sync and remote control with no screen, for scripts
//! (tools/sync-check.sh). It reads one command per line on stdin and answers each with one line on
//! stdout; what sync tells on its own (events, changed profiles) comes as lines starting with `told`.
//! `open` opens the profile in use as the terminal client does, with remote control on and silent, so
//! the device plays, serves and controls the group's other devices.
//!
//! ```text
//! nori-cli --sync --data DIR [--name NAME] [--phone]
//! ```

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nori_core::settings::{servers_activated, ServerList, StoredPrefs};
use nori_core::settings_store::Settings;
use nori_core::sync::{Came, Here, NeededPlugin, PairingView, ProfilesSynced, Reach, SyncClient, SyncEvent};
use nori_host::desktop::Root;

const HELP: &str = "state, code, take <link>, confirm [keep], cancel, devices, reset, keep-copy on|off, profiles, \
add <name> <url> <user> <password>, rename <name> <new name>, remove <name>, sync on|off, server <url>|-, \
foreground on|off, playing on|off, woke, look on|off, nearby, take-nearby <name>, open, play <query>, now, \
remote watch on|off, remote devices, remote send <name>|* play|pause|next, remote pick <name>|*|here, \
local <name>, quit";

/// Prints a line at once, whichever thread says it.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

struct Told {
    root: Arc<Root>,
    data: PathBuf,
}

impl SyncClient for Told {
    fn here(&self) -> Here {
        self.root.plugins().kinds.sync_here()
    }

    fn profiles(&self, synced: ProfilesSynced) {
        nori_host::sync::profiles_synced(&self.data, &synced);
        say(&format!("told profiles {:?} removed {:?}", synced.active, synced.removed));
    }

    fn install(&self, plugins: Vec<NeededPlugin>) {
        let ids: Vec<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
        say(&format!("told install {}", ids.join(" ")));
    }

    fn event(&self, event: SyncEvent) {
        say(&format!("told event {event:?}"));
    }

    fn changed(&self) {}
}

fn usage() -> ! {
    eprintln!("usage: nori-cli --sync [--data DIR] [--name NAME] [--phone]\ncommands on stdin: {HELP}");
    std::process::exit(2);
}

pub fn main(args: Vec<String>) {
    let (mut data, mut name, mut reach) = (nori_host::data_dir(), nori_host::device_name(), Reach::Computer);
    let mut it = args.into_iter().filter(|a| a != "--sync");
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| usage());
        match k.as_str() {
            "--data" => data = v().into(),
            "--name" => name = v(),
            "--phone" => reach = Reach::Phone,
            _ => usage(),
        }
    }
    if let Err(e) = run(&data, name, reach) {
        eprintln!("nori: {e}");
        std::process::exit(1);
    }
}

fn run(data: &Path, name: String, reach: Reach) -> Result<(), String> {
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    nori_host::keep_log(data);
    let root = Arc::new(Root::new(data));
    let settings = root.app.settings.clone();
    settings.open(&nori_host::db_path(data)).map_err(|e| format!("the settings: {e}"))?;
    let told = Arc::new(Told { root: root.clone(), data: data.into() });
    let syncing = nori_host::sync::computer(settings.clone(), told, reach);
    let mut session: Option<crate::backend::Session> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let on = |w: Option<&&str>| matches!(w, Some(&"on"));
        let answer = match words.as_slice() {
            [] => continue,
            ["quit"] => break,
            ["state"] => format!("state {:?}", syncing.state()),
            ["pairing"] => pairing(&syncing.pairing()),
            ["code"] => {
                syncing.pair_show(name.clone());
                pairing(&syncing.pairing())
            }
            ["take", link] => format!("take {:?}", syncing.pair_take(link.to_string(), Came::Link)),
            ["confirm", rest @ ..] => {
                syncing.pair_confirm(name.clone(), rest.first() == Some(&"keep"));
                pairing(&syncing.pairing())
            }
            ["cancel"] => {
                syncing.pair_cancel();
                "cancelled".into()
            }
            ["devices"] => {
                let devices: Vec<String> = syncing.devices().iter().map(|d| format!("{}:{}", d.name, if d.online { "online" } else { "offline" })).collect();
                format!("devices {}", devices.join(","))
            }
            ["reset"] => {
                syncing.reset();
                "reset".into()
            }
            ["keep-copy", w] => {
                syncing.keep_copy(on(Some(w)));
                format!("keep-copy {}", syncing.keeps_copy())
            }
            ["profiles"] => profiles(&settings),
            ["add", name, url, user, password] => {
                let profile = nori_core::settings::SavedServer { id: nori_core::settings::new_server_id(), name: name.to_string(), url: url.to_string(), user: user.to_string(), password: password.to_string(), ..Default::default() };
                edit(&settings, |list| servers_activated(list, profile));
                profiles(&settings)
            }
            ["rename", from, to] => {
                edit(&settings, |mut list| {
                    list.servers.iter_mut().filter(|s| s.name == *from).for_each(|s| s.name = to.to_string());
                    list
                });
                profiles(&settings)
            }
            ["remove", which] => {
                let id = settings.with_prefs(|p| p.servers.iter().find(|s| s.name == *which).map(|s| s.id.clone())).flatten();
                if let Some(id) = id {
                    edit(&settings, |list| nori_core::settings::servers_removed(list, id));
                }
                profiles(&settings)
            }
            ["sync", w] => {
                settings.change(|p| StoredPrefs { sync: on(Some(w)), ..p.clone() });
                syncing.settings_changed();
                format!("state {:?}", syncing.state())
            }
            ["server", url] => {
                settings.change(|p| StoredPrefs { sync_server: Some(url.to_string()).filter(|u| u != "-"), ..p.clone() });
                syncing.settings_changed();
                format!("server {}", settings.with_prefs(|p| p.sync_server.clone()).flatten().unwrap_or_default())
            }
            ["foreground", w] => {
                syncing.foreground(on(Some(w)));
                format!("state {:?}", syncing.state())
            }
            ["playing", w] => {
                syncing.playing(on(Some(w)));
                format!("state {:?}", syncing.state())
            }
            ["woke"] => {
                syncing.woke();
                "woke".into()
            }
            ["local", which] => {
                let id = settings.with_prefs(|p| p.servers.iter().find(|s| s.name == *which).map(|s| s.id.clone())).flatten();
                if let Some(id) = id {
                    syncing.sync_off(id.clone(), false);
                    edit(&settings, |mut list| {
                        list.servers.iter_mut().filter(|s| s.id == id).for_each(|s| s.sync_off = true);
                        list
                    });
                }
                profiles(&settings)
            }
            ["look", w] => {
                syncing.pair_look(on(Some(w)));
                "look".into()
            }
            ["nearby"] => format!("nearby {}", syncing.nearby().iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(",")),
            ["take-nearby", name] => match syncing.nearby().into_iter().find(|d| d.name == *name) {
                Some(d) => format!("take {:?}", syncing.pair_take(d.link, Came::Nearby)),
                None => "take none".into(),
            },
            ["open"] => {
                session = None;
                match open_session(&root, &syncing) {
                    Ok(s) => {
                        session = Some(s);
                        format!("open {}", settings.with_prefs(|p| p.active_server().map(|s| s.name.clone())).flatten().unwrap_or_default())
                    }
                    Err(e) => format!("open failed: {e}"),
                }
            }
            // As a search typed and answered by the server.
            ["play", query @ ..] => match session.as_ref().and_then(|s| s.search_server_now(s.search_typed(&query.join(" ")).query, |e| e.to_string())) {
                Some(nori_core::search::SearchView { answer: nori_core::search::SearchShown::Found { result }, .. }) if !result.songs.is_empty() => {
                    let s = session.as_ref().expect("open");
                    s.play(result.songs.clone(), 0, false, None);
                    format!("play {}", result.songs.len())
                }
                _ => "play nothing".into(),
            },
            ["now"] => session.as_ref().map_or("now closed".into(), |s| now(&s.now())),
            ["remote", rest @ ..] => match session.as_ref().and_then(|s| s.remote()) {
                Some(r) => remote(&r, rest),
                None => "remote off".into(),
            },
            _ => format!("unknown: {HELP}"),
        };
        say(&answer);
    }
    if let Some(s) = session.take() {
        s.close();
    }
    syncing.settings_changed();
    settings.writer.flush();
    Ok(())
}

/// The profile in use opened as the terminal client opens it, remote control on, the music silent.
fn open_session(root: &Root, syncing: &Arc<nori_core::sync::Syncing>) -> Result<crate::backend::Session, String> {
    let settings = &root.app.settings;
    settings.change(|p| StoredPrefs { remote_control: true, ..p.clone() });
    let profile = settings.with_prefs(|p| p.active_server().cloned()).flatten().ok_or("no profile")?;
    // What the session tells a screen goes nowhere: these commands ask instead.
    let (tx, _) = std::sync::mpsc::channel();
    let o = crate::backend::Open { root, http: nori_http::Http::new(), profile, device: None, images: false, offline: false, mpris: None, sync: Some(syncing.clone()), tx };
    let s = crate::backend::Session::open(o)?;
    s.set_volume(0.0);
    s.in_sight(true);
    Ok(s)
}

/// What the player shows: where it plays, playing or paused, and the song.
fn now(now: &nori_host::player::Now) -> String {
    let title = now.song.as_ref().map_or("-", |s| s.title.as_str());
    format!("now {} {} {title}", now.device.as_deref().unwrap_or("here"), if now.playing { "playing" } else { "paused" })
}

/// A remote control command's answer.
fn remote(r: &Arc<nori_core::remote::Remote>, words: &[&str]) -> String {
    use nori_core::remote::wire::Op;
    // `*`: the first device listed, as two terminals on one computer share its name.
    let id_of = |name: &str| r.devices().into_iter().find(|d| name == "*" || d.name == name).map(|d| d.id);
    match words {
        ["watch", w] => {
            r.clone().watch(*w == "on");
            "remote watch".into()
        }
        ["devices"] => {
            let listed: Vec<String> = r.devices().iter().map(|d| {
                let st = d.state.as_ref().map_or("none".to_string(), |s| format!("{} {}", if s.playing { "playing" } else { "paused" }, d.now.as_ref().map_or("-", |e| e.title.as_str())));
                format!("{}{}:{st}", d.name, if d.nearby { " (nearby)" } else { "" })
            }).collect();
            format!("remote devices {}", listed.join("|"))
        }
        ["send", name, op] => {
            let op = match *op {
                "play" => Op::Play,
                "pause" => Op::Pause,
                "next" => Op::Next,
                _ => return format!("unknown: {HELP}"),
            };
            match id_of(name) {
                Some(id) => {
                    r.clone().send(id, op);
                    "remote sent".into()
                }
                None => "remote no such device".into(),
            }
        }
        ["pick", "here"] => {
            r.clone().pick(None);
            "remote picked here".into()
        }
        ["pick", name] => match id_of(name) {
            Some(id) => {
                r.clone().pick(Some(id));
                "remote picked".into()
            }
            None => "remote no such device".into(),
        },
        _ => format!("unknown: {HELP}"),
    }
}

/// The pairing as a line: a code shows its link and words.
fn pairing(view: &PairingView) -> String {
    match view {
        PairingView::Code { link, words } => format!("pairing Code {link} {}", words.join(" ")),
        other => format!("pairing {other:?}"),
    }
}

fn profiles(settings: &Settings) -> String {
    let names: Vec<String> = settings.with_prefs(|p| p.servers.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    format!("profiles {}", names.join(","))
}

/// The saved profiles changed by `change`, kept as a form keeps them.
fn edit(settings: &Settings, change: impl FnOnce(ServerList) -> ServerList) {
    settings.change(|p| {
        let list = change(ServerList { servers: p.servers.clone(), active_server_id: p.active_server_id.clone() });
        StoredPrefs { servers: list.servers, active_server_id: list.active_server_id, ..p.clone() }
    });
}
