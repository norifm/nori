//! `nori-cli --sync`: pairing and profile sync with no screen, for scripts (tools/sync-check.sh). It
//! reads one command per line on stdin and answers each with one line on stdout; what sync tells on its
//! own (events, changed profiles) comes as lines starting with `told`.
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
foreground on|off, playing on|off, woke, quit";

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
    let sockets = nori_http::WebSockets::new();
    let told = Arc::new(Told { root: root.clone(), data: data.into() });
    let syncing = nori_core::sync::Syncing::native(settings.clone(), sockets.clone(), told, reach);
    sockets.tell(&syncing);
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
            _ => format!("unknown: {HELP}"),
        };
        say(&answer);
    }
    syncing.settings_changed();
    settings.writer.flush();
    Ok(())
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
