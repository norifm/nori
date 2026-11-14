//! The event loop: terminal input, engine events and worker results arrive on one channel; the screen
//! is drawn only after something arrived or a timer (`App::next_wake`) fired. Idle, it blocks.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;

use nori_host::desktop::Root;
use std::time::Instant;

use nori_core::remote::wire::Op;
use nori_core::providers::flow::{AddDo, AddEvent, SignInEvent};
use nori_core::providers::AfterAdding;
use nori_core::remote::JamStart;
use nori_core::settings::SavedServer;
use nori_host::player::{Playback, QueueShown};
use nori_host::remote::Elsewhere;
use nori_http::Http;
use nori_plugins::ItemKind;
use ratatui::crossterm::event::{self, Event as TermEvent};
use ratatui_image::picker::Picker;

use crate::app::{App, Cmd, Now, Panel, View};
use crate::art::{protocol_name, Art, Colors, COVER_PX};
use crate::backend::{block_on, own, Msg, Open, Session};
use crate::Options;

/// Forwards terminal input from a dedicated thread.
fn read_input(tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("nori-input".into()).spawn(move || {
        let mut replies = Replies::default();
        loop {
        let msg = match event::read() {
            Ok(TermEvent::Key(k)) if replies.swallows(&k) => continue,
            Ok(TermEvent::Key(k)) => Msg::Key(k),
            Ok(TermEvent::Mouse(m)) => Msg::Mouse(m),
            Ok(TermEvent::Paste(p)) => Msg::Paste(p),
            Ok(TermEvent::Resize(..)) => Msg::Resize,
            Ok(TermEvent::FocusGained) => Msg::Focus(true),
            Ok(TermEvent::FocusLost) => Msg::Focus(false),
            Err(_) => return,
        };
        if tx.send(msg).is_err() {
            return;
        }
        }
    });
}

/// Drops terminal query replies that arrive after the query timed out (common over ssh). crossterm reads
/// them (APC `ESC _ … ESC \`, DCS `ESC P … ESC \`, OSC `ESC ] … BEL`) as alt+_ followed by keys that
/// would trigger bindings. A reply is dropped from its opener to its terminator, 300 ms, or 512 keys.
#[derive(Default)]
pub(crate) struct Replies {
    since: Option<(Instant, usize)>,
}

impl Replies {
    pub(crate) fn swallows(&mut self, k: &ratatui::crossterm::event::KeyEvent) -> bool {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if let Some((at, n)) = &mut self.since {
            *n += 1;
            let over = at.elapsed() > std::time::Duration::from_millis(300) || *n > 512;
            let end = (alt && k.code == KeyCode::Char('\\')) || k.code == KeyCode::Char('\x07') || (k.code == KeyCode::Char('g') && k.modifiers.contains(KeyModifiers::CONTROL));
            if end || over {
                self.since = None;
            }
            return !over || end;
        }
        if alt && matches!(k.code, KeyCode::Char('_' | 'P' | ']')) {
            self.since = Some((Instant::now(), 0));
            return true;
        }
        false
    }
}

/// Graphics query timeout; longer over ssh.
fn query_ms() -> u64 {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        2500
    } else {
        1000
    }
}

/// Queries the terminal's graphics protocol and cell size (ratatui-image). Returns whether tmux itself
/// keeps the pictures.
///
/// After a timeout the query reader still waits on stdin and would take the next key, so a
/// device-attributes query (always answered) is sent to release it; late replies are drained.
///
/// Under tmux, tmux is queried directly when it renders sixel ([`crate::term::tmux_draws_sixel`]):
/// it then keeps and redraws pictures itself. Otherwise pictures pass through and are resent when the
/// pane returns ([`crate::term::tmux_focus_events`]).
fn query_picker() -> (Picker, bool) {
    use std::io::Write;
    let t0 = Instant::now();
    let query = || {
        let mut o = ratatui_image::picker::cap_parser::QueryStdioOptions::default();
        o.timeout = std::time::Duration::from_millis(query_ms());
        Picker::from_query_stdio_with_options(o).unwrap_or_else(|_| Picker::halfblocks())
    };
    let tmux_sixel = (crate::term::in_tmux() && crate::term::tmux_draws_sixel())
        .then(|| crate::term::not_tmux(query))
        .filter(|p| p.protocol_type() == ratatui_image::picker::ProtocolType::Sixel);
    let kept_by_tmux = tmux_sixel.is_some();
    let picker = tmux_sixel.unwrap_or_else(query);
    if t0.elapsed() >= std::time::Duration::from_millis(query_ms()) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[c");
        let _ = out.flush();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    while event::poll(std::time::Duration::from_millis(30)).unwrap_or(false) {
        let _ = event::read();
    }
    let by = if kept_by_tmux { " (by tmux)" } else { "" };
    eprintln!("nori: pictures drawn with {:?}{by}, cells {:?} px, in {:?}", picker.protocol_type(), picker.font_size(), t0.elapsed());
    (picker, kept_by_tmux)
}

struct Runner {
    o: Options,
    root: Arc<Root>,
    http: Arc<Http>,
    tx: Sender<Msg>,
    session: Option<Session>,
    /// The profile in use while its plugin doesn't run here, opened once it does.
    waiting: Option<SavedServer>,
    /// The lyrics panel was shown when last told to the session ([`Session::lyrics_view`]).
    lyrics_viewed: bool,
    /// Media controls for the process; each session drives them while open.
    mpris: Option<Arc<nori_mpris::Mpris>>,
    art: Option<Art>,
    picker: Option<Picker>,
    /// Pending cover requests; dropping a ticket cancels the fetch.
    tickets: Vec<nori_covers::loader::Ticket>,
    /// Id of the audible song last shown.
    heard: Option<String>,
    /// Rewrite the whole screen, pictures included, at the next draw.
    repaint: bool,
    /// A cover arrived while unfocused: repaint when focus returns.
    unseen_cover: bool,
    /// Whose playback the player shows, as last read: this computer's, the device playing while it is
    /// another one (its controls go there), or a guest's jam.
    playback: Playback,
    /// The queue shown, as last read.
    queue: Option<QueueShown>,
    /// The provider's sign-in page, while it is open.
    #[cfg(feature = "sign-in-window")]
    sign_in_window: Option<nori_sign_in_window::SignInWindow>,
    /// The code sign-in running while the account sheet shows its code, with its number; and how many
    /// have run.
    code: Option<(u32, Arc<nori_core::providers::code::CodeSignIn>)>,
    codes: u32,
}

pub fn run(o: Options) -> Result<(), String> {
    crate::term::stderr_to(&o.data.join("nori.log"));
    crate::term::hook_panics();
    let db = crate::backend::db_path(&o.data);
    let root = Arc::new(Root::new(&o.data));
    let ran = run_with(o, &root, &db);
    root.app.settings.writer.flush();
    ran
}

fn run_with(o: Options, root: &Arc<Root>, db: &str) -> Result<(), String> {
    let mut prefs = root.app.settings.open(db).map_err(|e| format!("the settings: {e}"))?;
    // A server given on the command line is added (or found) and made active.
    if let Some((url, user, password)) = o.login.clone() {
        let found = prefs.servers.iter().find(|s| s.url == url && s.user == user).map(|s| s.id.clone());
        let id = found.unwrap_or_else(|| {
            let s = SavedServer { id: nori_core::settings::new_server_id(), url: url.clone(), user: user.clone(), password: password.clone(), ..Default::default() };
            prefs.servers.push(s.clone());
            s.id
        });
        prefs.servers.iter_mut().filter(|s| s.id == id && !password.is_empty()).for_each(|s| s.password = password.clone());
        root.app.settings.put(prefs);
        nori_host::profile_picked(&root.app.settings, &id);
        prefs = root.app.settings.current().unwrap_or_default();
    }
    if let Some(file) = &o.install_plugin {
        root.install_plugin(file)?;
    }
    if let Some(kind) = o.provider.clone() {
        let list = nori_core::settings::ServerList { servers: prefs.servers.clone(), active_server_id: prefs.active_server_id.clone() };
        let form = SavedServer { id: nori_core::settings::new_server_id(), kind, ..Default::default() };
        prefs = nori_core::providers::profile_activated(prefs, root.plugins().kinds.login_profile(list, form));
        root.app.settings.put(prefs.clone());
    }
    let mouse = o.mouse.unwrap_or_else(|| own::flag(&root.app.settings, own::MOUSE, true));
    let images = o.images.unwrap_or_else(|| own::flag(&root.app.settings, own::IMAGES, true));
    let mut terminal = crate::term::enter(mouse).map_err(|e| e.to_string())?;
    // Before the input thread starts: the query reply arrives on stdin.
    let (picker, kept_by_tmux) = match images {
        true => {
            let (p, kept) = query_picker();
            (Some(p), kept)
        }
        false => (None, false),
    };
    // Pictures passed through tmux are lost while the pane is hidden: have tmux report its return.
    if picker.as_ref().is_some_and(|p| p.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks) && !kept_by_tmux {
        crate::term::tmux_focus_events();
    }
    let (tx, rx) = channel();
    read_input(tx.clone());
    let mut app = App::new(prefs.clone());
    app.colors = Colors::detect(|k| std::env::var(k).ok());
    app.paint = o.paint.unwrap_or_else(|| own::flag(&root.app.settings, own::PAINT, true));
    app.reduced_motion = std::env::var("NORI_REDUCE_MOTION").is_ok_and(|v| !v.is_empty() && v != "0");
    app.retheme();
    app.mouse = mouse;
    app.images = images;
    app.offline = o.offline;
    app.volume = own::number(&root.app.settings, own::VOLUME, 1.0);
    app.protocol = picker.as_ref().map_or("off", |p| protocol_name(p.protocol_type()));
    app.settings.own.data = o.data.display().to_string();
    app.settings.own.kinds = root.plugins().kinds.clone();
    app.settings.own.device = own::text(&root.app.settings, own::DEVICE).unwrap_or_default();
    let art = picker.clone().map(Art::new);
    let mpris = o.mpris.then(|| nori_mpris::Mpris::start("").ok().map(Arc::new)).flatten();
    app.shared = root.app.clone();
    app.settings.own.shared = root.app.clone();
    let mut r = Runner { root: root.clone(), http: Http::new(), tx, session: None, waiting: None, lyrics_viewed: false, mpris, art, picker, tickets: Vec::new(), heard: None, repaint: false, unseen_cover: false, playback: Playback::Here, queue: None, #[cfg(feature = "sign-in-window")] sign_in_window: None, code: None, codes: 0, o };
    // Plugins reach the web before any profile opens; a session's own transport takes over.
    app.settings.own.kinds.set_transport(r.http.clone());
    let notices = r.tx.clone();
    r.root.plugins().watched.hear(move |n| {
        let _ = notices.send(Msg::Note { text: n.words, error: n.problem });
    });
    let changed = r.tx.clone();
    r.root.plugins().watched.on_changed(move || {
        let _ = changed.send(Msg::PluginsChanged);
    });
    let casts = r.tx.clone();
    r.root.plugins().watched.hear_casts(move |c| {
        let _ = casts.send(Msg::Cast(c));
        // What can be cast to leaves out what is cast to.
        let _ = casts.send(Msg::PluginsChanged);
    });
    let asks = r.tx.clone();
    r.root.plugins().watched.sign_in_with(move |plugin, url| {
        let _ = asks.send(Msg::PluginSignIn { plugin, url });
    });
    if let Some((safe, names)) = r.root.safe_start() {
        app.say(crate::text::plugins_safe(&safe, &names), true);
    }
    match prefs.active_server().cloned() {
        Some(p) => r.open(&mut app, p, None),
        None => {
            app.choose_provider();
            r.update_plugins();
        }
    }
    if app.view != View::Login {
        app.go(View::Home);
    }
    let result = r.run(&mut app, &mut terminal, rx);
    if let Some(s) = r.session.take() {
        eprintln!("nori: output underruns this run: {}", s.engine.status().underruns);
        s.close();
    }
    crate::term::leave();
    result
}

impl Runner {
    /// Opens `profile`; `added` says what follows when it was just added.
    fn open(&mut self, app: &mut App, profile: SavedServer, added: Option<AfterAdding>) {
        if let Some(s) = self.session.take() {
            s.close_for(Some(&profile));
        }
        (self.playback, self.queue) = (Playback::Here, None);
        self.waiting = self.root.plugins().kinds.waits(profile.clone()).map(|_| profile.clone());
        if self.waiting.is_some() {
            // The first screen, where its plugin's line installs it or turns it on; one built for an older
            // nori is updated now, and the profile opens once it is (`waited`).
            self.update_plugins();
            return app.session_ended();
        }
        let name = nori_core::settings::label(&profile.name, &profile.url);
        let o = Open {
            root: &self.root,
            http: self.http.clone(),
            profile,
            device: self.o.device.clone(),
            images: app.images,
            offline: self.o.offline,
            mpris: self.mpris.clone(),
            tx: self.tx.clone(),
        };
        match Session::open(o) {
            Ok(s) => {
                s.check();
                s.update_check(false);
                if !app.in_sight {
                    s.in_sight(false);
                }
                app.server = name;
                app.unreachable = None;
                self.session = Some(s);
                self.lyrics_viewed = false;
                self.heard = None;
                // Fresh screens for the new server, keeping client-wide state.
                let prefs = app.prefs.clone();
                let keep = (app.mouse, app.images, app.volume, app.protocol, app.offline, app.server.clone(), app.settings.own.clone(), app.colors, app.paint, app.reduced_motion);
                *app = App::new(prefs);
                (app.mouse, app.images, app.volume, app.protocol, app.offline, app.server, app.settings.own, app.colors, app.paint, app.reduced_motion) = keep;
                app.retheme();
                app.rules = self.session.as_ref().map(|s| s.rules()).unwrap_or_default();
                app.output = self.o.device.clone().or_else(|| own::text(&self.root.app.settings, own::DEVICE)).unwrap_or_default();
                match self.session.as_ref().and_then(|s| s.client.sign_in_due(added)) {
                    Some(offer @ SignInEvent::Offer { .. }) => self.offer_sign_in(app, offer),
                    Some(SignInEvent::Code) => {
                        self.offer_sign_in(app, SignInEvent::Offer { ended: false });
                        app.sign_in_with_code();
                    }
                    Some(SignInEvent::Close) => {}
                    Some(_) => {
                        self.offer_sign_in(app, SignInEvent::Offer { ended: false });
                        self.carry(app, Cmd::SignInWindow);
                    }
                    None => {}
                }
                self.remote_read(app);
                self.follow(app);
                self.update_plugins();
                self.plugin(app, crate::app::PluginCmd::Read);
            }
            Err(said) => {
                app.session_ended();
                app.login.flow.on(AddEvent::Unopened { said });
            }
        }
    }

    fn run(&mut self, app: &mut App, terminal: &mut crate::term::Term, rx: Receiver<Msg>) -> Result<(), String> {
        loop {
            self.carry_out(app);
            if !app.checks_code() {
                self.code = None;
            }
            if app.quit {
                return Ok(());
            }
            if app.dirty {
                self.follow(app);
                // Run what following asked for (e.g. the new song's cover) now.
                self.carry_out(app);
                if std::mem::take(&mut self.repaint) {
                    crate::term::debug!("the whole screen written again, pictures included");
                    if let Some(a) = &mut self.art {
                        a.resend();
                    }
                    crate::term::repaint(terminal).map_err(|e| e.to_string())?;
                }
                let art = self.art.as_mut();
                terminal.draw(|f| crate::ui::draw(f, app, art)).map_err(|e| e.to_string())?;
                // Request card covers drawing found missing.
                self.carry_out(app);
                crate::term::debug!("drew {:?}: {:?} {:?} at {} ms, song {:?}", app.view, app.now.state, self.heard, app.now.position_ms, app.song.as_ref().map(|s| &s.title));
                app.dirty = false;
            }
            let now = Instant::now();
            let msg = match app.next_wake(now) {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(now)),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(m) => {
                    self.take(app, m);
                    // Drain the queue before drawing, running each message's commands before the next
                    // (a held key steps from where the last one left).
                    while let Ok(m) = rx.try_recv() {
                        self.carry_out(app);
                        self.take(app, m);
                    }
                    self.account_changed(app);
                }
                Err(RecvTimeoutError::Timeout) => app.tick(Instant::now()),
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }

    /// The screens follow the account when it changed under the profile; a session the source ended
    /// says so.
    fn account_changed(&self, app: &mut App) {
        let Some(s) = &self.session else { return };
        let Some(state) = s.account_changed() else { return };
        app.rules = s.rules();
        app.excluded = s.core.mix_excluded_songs().unwrap_or_default().into_iter().collect();
        app.dirty = true;
        if let Some(offer) = s.client.sign_in_due(None).filter(|_| state == nori_core::providers::AccountState::Ended) {
            self.offer_sign_in(app, offer);
        }
    }

    /// The account sheet over the profile in use, as its provider has the account, opened by `offer`.
    fn offer_sign_in(&self, app: &mut App, offer: SignInEvent) {
        let Some(profile) = self.root.app.settings.current().and_then(|p| p.active_server().cloned()) else { return };
        let Some(ways) = app.settings.own.kinds.providers().into_iter().find(|i| i.kind == profile.kind).map(|i| i.sign_in).filter(|w| !w.is_empty()) else { return };
        let mut flow = nori_core::providers::flow::SignInFlow::default();
        flow.on(offer);
        let ended = flow.shown == (nori_core::providers::flow::SignInShown::Offered { ended: true });
        let user = if ended { String::new() } else { profile.user };
        app.overlay = Some(crate::app::Overlay::Account { kind: profile.kind, ways, user, pasting: false, text: String::new(), flow, window_failed: false });
        app.dirty = true;
    }

    /// Updates the plugins once a day. Called once the session (or the sign-in) is up, which ends the
    /// start for safe mode.
    fn update_plugins(&self) {
        self.root.plugins().host.started();
        let tx = self.tx.clone();
        self.root.update_plugins(move |updated| {
            let _ = tx.send(Msg::Updated(updated));
        });
    }

    /// Does `c` with the plugin host, then shows the plugins as they now are. Installs and the repos'
    /// answers wait on files and the network, off this thread.
    fn plugin(&mut self, app: &mut App, c: crate::app::PluginCmd) {
        use crate::app::PluginCmd;
        let host = self.root.plugins().host.clone();
        let tx = self.tx.clone();
        match c {
            PluginCmd::Read => {}
            PluginCmd::Switched => host.switched(),
            PluginCmd::Install(path) => nori_host::spawn("nori-plugin-preview", move || {
                let _ = tx.send(Msg::Preview(host.preview_file(path, "en".into())));
            }),
            PluginCmd::InstallPreview(p) => nori_host::spawn("nori-plugin-install", move || {
                let _ = tx.send(Msg::Installed(host.install_preview(*p)));
            }),
            PluginCmd::Menu(entry, items) => {
                return nori_host::spawn("nori-plugin-menu", move || block_on(host.run_menu(entry, items)));
            }
            PluginCmd::Ask(song) => {
                return nori_host::spawn("nori-plugin-slots", move || {
                    let info = block_on(host.song_info((*song).clone(), "en".into()));
                    let status = block_on(host.player_status());
                    let _ = tx.send(Msg::Slots { song: song.id, info, status });
                });
            }
            PluginCmd::Turn(id, on) => host.turn(id, on),
            PluginCmd::TurnOnChosen(id) => {
                host.turn(id.clone(), true);
                let kind = nori_core::settings::ServerKind::Plugin { id };
                // The profile waiting for it opens below, instead of a new one's form.
                if self.waiting.as_ref().is_none_or(|p| p.kind != kind) {
                    app.provider_chosen(kind);
                }
            }
            PluginCmd::Place(id, at) => host.place(id, at),
            PluginCmd::Uninstall(id) => {
                app.settings.plugin_gone(&id);
                host.uninstall(id);
            }
            PluginCmd::Set(id, row, v) => host.set(id, row, v),
            PluginCmd::Act(id, row) => host.act(id, row),
            PluginCmd::CastTo(target) => host.cast_to(target),
            PluginCmd::EndCast => {
                host.cast_button(nori_plugins::CastButton::Stop);
            }
            PluginCmd::Look => {
                app.settings.own.shelf = crate::settings_view::Shelf::Asking;
                nori_host::spawn("nori-plugin-offers", move || {
                    let _ = tx.send(Msg::Offers(block_on(host.offers())));
                });
            }
            PluginCmd::Readme(id, offer) => {
                return nori_host::spawn("nori-plugin-readme", move || {
                    let blocks = block_on(host.readme(id.clone(), offer.map(|o| *o)));
                    let _ = tx.send(Msg::Readme { id, blocks });
                });
            }
            PluginCmd::Search(query) => {
                app.settings.own.query = query;
                return narrowed(app);
            }
            PluginCmd::ShowRepo(repo) => {
                app.settings.own.repo_shown = repo;
                return narrowed(app);
            }
            PluginCmd::Get(offer, version) => {
                let crate::settings_view::Shelf::Read(offers) = &app.settings.own.shelf else { return };
                let Some(v) = offers.offers.get(offer as usize).and_then(|o| o.versions.get(version as usize)).cloned() else { return };
                nori_host::spawn("nori-plugin-get", move || {
                    let _ = tx.send(Msg::Preview(block_on(host.preview_version(v, "en".into()))));
                });
            }
            PluginCmd::Hold(id, held) => host.hold(id, held),
            PluginCmd::AddRepo(url) => {
                host.add_repo(url);
                return self.plugin(app, PluginCmd::Look);
            }
            PluginCmd::RemoveRepo(url) => {
                host.remove_repo(url);
                return self.plugin(app, PluginCmd::Look);
            }
            PluginCmd::Noise(device, noise) => return host.set_noise(device, noise),
            PluginCmd::AllowMobileAssets(id) => host.allow_mobile_assets(id),
        }
        let plugged = self.root.plugins();
        let host = &plugged.host;
        let mut cards = host.cards("en".into());
        let own = &mut app.settings.own;
        // What was installed since the repos were asked shows as installed.
        if let crate::settings_view::Shelf::Read(offers) = &mut own.shelf {
            host.offers_stand(&mut offers.offers);
        }
        app.slots.asked = host.running().iter().any(|i| i.manifest.extends("slots"));
        own.off = plugged.watched.as_said(&mut cards);
        own.assets = cards.iter().map(|c| (c.id.clone(), host.assets(c.id.clone()))).filter(|(_, a)| !a.is_empty()).collect();
        own.plugins = cards;
        own.repos = host.repos();
        app.slots.menu = [ItemKind::Song, ItemKind::Album, ItemKind::Artist, ItemKind::Playlist].into_iter().flat_map(|k| host.menu_entries(k, "en".into()).into_iter().map(move |e| (k, e))).collect();
        // A device's page: its known controls, then its plugin's rows.
        let controlled = host.controlled("en".into());
        let mut shown = host.device_cards("en".into());
        for d in &controlled {
            if !shown.iter().any(|c| c.id == d.card.id) {
                shown.push(d.card.clone());
            }
        }
        own.off.extend(plugged.watched.as_said(&mut shown));
        app.devices.plugins = shown;
        app.devices.targets = host.cast_targets();
        app.devices.finds = host.finds_outputs();
        own.controlled = controlled.clone();
        app.devices.controlled = controlled;
        app.settings.invalidate();
        app.cmds.extend(app.settings.readme_wanted());
        app.dirty = true;
        self.waited(app);
    }

    /// The waiting profile opened once its plugin runs; until then the first screen says why it waits.
    fn waited(&mut self, app: &mut App) {
        let Some(profile) = self.waiting.clone() else { return };
        let nori_core::settings::ServerKind::Plugin { id } = &profile.kind else { return };
        match self.root.plugins().kinds.waits(profile.clone()) {
            None => self.open(app, profile, None),
            Some(wait) => {
                let own = &app.settings.own;
                let name = nori_plugins::plugin_display_name(id.clone(), own.plugins.clone(), own.offers().to_vec());
                app.login.flow.on(AddEvent::Unopened { said: crate::text::profile_waits(&name, wait) });
            }
        }
    }

    /// The profile in use opened again, as signing in or out left it.
    fn reopen(&mut self, app: &mut App) {
        let Some(profile) = self.root.app.settings.current().and_then(|p| p.active_server().cloned()) else { return };
        self.open(app, profile, None);
        app.go(View::Home);
    }

    fn take(&mut self, app: &mut App, m: Msg) {
        crate::term::debug!("took {}", m.brief());
        let Some(m) = current(self.session.as_ref().map(|s| s.id), m) else { return };
        match &m {
            Msg::CodeStep { code, .. } if self.code.as_ref().map(|c| c.0) != Some(*code) => return,
            Msg::Cover { art, image, .. } => {
                if let Some(a) = &mut self.art {
                    a.put(art.clone(), image);
                }
                self.unseen_cover |= !app.in_sight;
            }
            Msg::Focus(on) => {
                if let Some(s) = &self.session {
                    s.in_sight(*on);
                }
                if *on {
                    self.update_plugins();
                }
                // Under tmux, pictures sent while the pane was hidden were lost.
                if *on && (std::mem::take(&mut self.unseen_cover) || crate::term::in_tmux()) {
                    self.repaint = true;
                }
            }
            Msg::Remote | Msg::Jam(_) => self.remote_read(app),
            // The plugin installed is shown, and asked to be turned on, from its card.
            Msg::Installed(_) => self.plugin(app, crate::app::PluginCmd::Read),
            Msg::BackedUp(r) => return app.say(r.as_ref().map_or_else(crate::text::backup_failed, |_| crate::text::BACKUP_SAVED.into()), r.is_err()),
            Msg::Restored(Ok(r)) => {
                if let Some(s) = &self.session {
                    s.backup_restored(&r.settings);
                }
                prefs_changed(app);
                let said = crate::text::restored(r);
                self.plugin(app, crate::app::PluginCmd::Read);
                return app.say(said, false);
            }
            Msg::Restored(Err(f)) => return app.say(crate::text::backup_failed(f), true),
            Msg::Readme { id, blocks } => {
                app.settings.readme_came(id, blocks.clone());
                return app.dirty = true;
            }
            Msg::Offers(o) => {
                app.settings.own.shelf = crate::settings_view::Shelf::Read(o.clone());
                return self.plugin(app, crate::app::PluginCmd::Read);
            }
            Msg::PluginSignIn { plugin, url } => {
                #[cfg(feature = "sign-in-window")]
                if let Some(s) = &self.session {
                    let kind = nori_core::settings::ServerKind::Plugin { id: plugin.clone() };
                    let (h, tx) = ((**s).clone(), self.tx.clone());
                    let title = format!("Sign in to {}", crate::text::provider_of(&app.settings.own.kinds, &kind).name);
                    self.sign_in_window = nori_sign_in_window::SignInWindow::open_at(app.settings.own.kinds.clone(), kind, title, url.clone(), move |_, cookies| {
                        let _ = tx.send(Msg::SignedIn(h.sign_in(cookies)));
                    });
                }
                let _ = (plugin, url);
                return;
            }
            Msg::IdentityChosen(Ok(())) => {
                app.handle(m);
                return self.reopen(app);
            }
            Msg::SignedIn(nori_core::providers::SignInResult::SignedIn { .. }) => {
                #[cfg(feature = "sign-in-window")]
                {
                    self.sign_in_window = None;
                }
                app.handle(m);
                return self.reopen(app);
            }
            // The guest profile opens in place of the user's own, which opens again on leaving.
            Msg::Joined(Ok(pass)) => {
                let guest = nori_host::jam_joined(&self.root.app.settings, pass.clone(), crate::text::JAM_GUEST);
                prefs_changed(app);
                self.open(app, guest, None);
                app.go(View::Home);
                return;
            }
            Msg::Left(said) => {
                match nori_host::jam_left(&self.root.app.settings) {
                    Some(p) => {
                        prefs_changed(app);
                        self.open(app, p, None);
                        app.go(View::Home);
                    }
                    None => {
                        if let Some(s) = self.session.take() {
                            s.close_for(None);
                        }
                        prefs_changed(app);
                        app.session_ended();
                    }
                }
                return app.say(said.clone(), false);
            }
            Msg::LoggedIn(Ok(p)) => {
                let added = match app.login.flow.on(AddEvent::Accepted) {
                    AddDo::Open { added } => added,
                    _ => None,
                };
                prefs_changed(app);
                self.open(app, p.clone(), added);
                app.go(View::Home);
                return;
            }
            _ => {}
        }
        app.handle(m);
    }

    /// Reads the other devices, the device playing and the jam hosted or joined into `app`. Playback
    /// moving to another device or back starts the player over from what it then shows; a jam guest's
    /// player shows the jam.
    fn remote_read(&mut self, app: &mut App) {
        let Some(s) = &self.session else { return };
        let remote = s.remote();
        let guest = s.guest;
        let jams = guest || self.root.app.settings.prefs(|p| p.remote_control);
        let d = &mut app.devices;
        // A guest's music is the jam's: it moves to no other device.
        d.on = remote.is_some() && !guest;
        d.pairing = remote.as_ref().filter(|_| d.on).and_then(|r| r.pairing_link());
        d.paired = remote.as_ref().filter(|_| d.pairing.is_some()).map(|r| r.paired()).unwrap_or_default();
        d.jam_start = remote.as_ref().filter(|_| d.on).map(|r| match r.jam_start() {
            JamStart::Offered | JamStart::Starting | JamStart::Hosting if !jams => JamStart::Unavailable,
            start => start,
        });
        d.list = remote.as_ref().filter(|_| d.on).map(|r| r.devices()).unwrap_or_default();
        d.reached = remote.as_ref().filter(|_| d.on).map(|r| r.outputs()).unwrap_or_default();
        if let Some(r) = &remote {
            let names = nori_core::remote::device_names(d.list.clone(), r.me(), crate::text::kind_words());
            d.list.iter_mut().zip(names).for_each(|(x, name)| x.name = name);
        }
        let jam = remote.as_ref().filter(|_| jams).and_then(|r| r.jam_view().map(|v| (v, r.jam_added())));
        (d.jam, d.added) = jam.map_or_else(Default::default, |(v, added)| (Some(v), added));
        let playback = s.playback();
        let e = playback.mirrored();
        d.controls = e.filter(|_| guest).and_then(|e| e.jam_controls());
        let moved = e.is_some() != self.playback.mirrored().is_some();
        let device = e.filter(|_| !guest);
        d.active = device.map(|e| (e.mirror.id.clone(), e.mirror.name.clone()));
        d.hearts = device.map(|e| e.mirror.rows.iter().map(|r| (r.song.id.clone(), r.song.starred)).collect()).unwrap_or_default();
        if let Some(v) = e.and_then(|e| e.volume()) {
            app.volume = v;
        }
        if moved {
            if e.is_none() {
                app.volume = s.volume();
            }
            self.queue = None;
            self.heard = None;
            app.heard(None);
        }
        // The system's media controls follow a jam's playback; another device's they do not show.
        if moved || (e.is_some() && playback.shown_to_system()) {
            s.mpris_changed();
        }
        self.playback = playback;
    }

    /// The device playing while it is another one, which the controls reach.
    fn there(&self) -> Option<&Elsewhere> {
        match &self.playback {
            Playback::There(e) => Some(e),
            _ => None,
        }
    }

    /// Reads the playback the player shows into `app`: this computer's, the device's playing elsewhere
    /// or the jam's, with its queue, copying strings only when the song or the queue changed.
    fn follow(&mut self, app: &mut App) {
        let Some(s) = &self.session else { return };
        let devices_open = app.full && app.panel == Panel::Devices;
        s.output_picker(devices_open);
        // A guest follows its jam all along.
        if let Some(r) = s.remote().filter(|_| !s.guest) {
            r.watch(devices_open);
        }
        let now = s.now();
        app.follow_now(Now { state: now.state, position_ms: now.position_ms, at: now.at, speed: now.pace, mixing: now.mixing, buffering: now.buffering });
        let id = now.song.as_ref().map(|s| s.id.clone());
        if id != self.heard {
            self.heard = id.clone();
            app.heard(now.song);
            app.lyrics_now(s.lyrics_heard(id));
        }
        if s.queue_stale(self.queue.as_ref()) {
            let q = s.queue_shown(self.queue.as_ref());
            app.queue = Some(nori_core::playlist::PlaylistView { songs: q.songs.to_vec(), ..q.view.clone() });
            self.queue = Some(q);
        }
        let here = matches!(self.playback, Playback::Here);
        match &self.heard {
            Some(id) if here && app.transition_shown() => {
                let note = self.root.app.planner.transition_note(id);
                if note != app.transition {
                    app.transition = note;
                }
                app.mixed_in = if app.now.mixing { self.root.app.planner.transition_into(id) } else { None };
            }
            _ if !here => {
                app.transition = None;
                app.mixed_in = None;
            }
            _ => {}
        }
        if app.lyrics_shown() {
            self.lyrics_step(app);
        }
    }

    /// Advances the lyrics clock and schedules its next wake.
    fn lyrics_step(&self, app: &mut App) {
        let Some(l) = app.lyrics.found() else {
            app.lyrics_wake = None;
            return;
        };
        let now = Instant::now();
        let force = app.lyrics_wake.is_some_and(|t| t <= now);
        let (_, wait) = l.advance(app.now.position(now), app.prefs.lyrics_sweep, force);
        app.lyrics_wake = wait.map(|ms| now + std::time::Duration::from_millis(ms));
    }

    fn carry_out(&mut self, app: &mut App) {
        // A command can add one: a provider turned on is added at once.
        while !app.cmds.is_empty() {
            for c in std::mem::take(&mut app.cmds) {
                self.carry(app, c);
            }
        }
        // The core warms the songs ahead's lyrics only while the panel is shown.
        let viewed = app.lyrics_shown();
        if let (Some(s), true) = (&self.session, viewed != self.lyrics_viewed) {
            s.lyrics_view(viewed);
            self.lyrics_viewed = viewed;
        }
    }

    fn carry(&mut self, app: &mut App, c: Cmd) {
        crate::term::debug!("carried out {}", c.brief());
        match c {
            Cmd::Quit => {
                app.quit = true;
                return;
            }
            Cmd::Mouse(on) => {
                crate::term::set_mouse(on);
                self.root.app.settings.keep_app_value(own::MOUSE, on.to_string());
                app.mouse = on;
                app.settings.invalidate();
                return;
            }
            Cmd::Device(name) => {
                self.root.app.settings.keep_app_value(own::DEVICE, name.clone());
                app.settings.own.device = name.clone();
                app.settings.invalidate();
                let said = if name.is_empty() { "the system default".to_string() } else { name };
                app.say(format!("Output device: {said}, from the next start"), false);
                return;
            }
            Cmd::Paint(on) => {
                self.root.app.settings.keep_app_value(own::PAINT, on.to_string());
                app.paint = on;
                app.retheme();
                app.settings.invalidate();
                return;
            }
            Cmd::Images(on) => {
                self.root.app.settings.keep_app_value(own::IMAGES, on.to_string());
                app.images = on;
                app.settings.invalidate();
                if on && self.art.is_none() {
                    let picker = self.picker.clone().unwrap_or_else(Picker::halfblocks);
                    app.protocol = protocol_name(picker.protocol_type());
                    self.art = Some(Art::new(picker));
                }
                if !on {
                    self.art = None;
                    self.tickets.clear();
                }
                app.say(if on { "Covers on (from the next song; restart to fetch covers again)" } else { "Covers off" }, false);
                return;
            }
            Cmd::OfferSignIn => {
                let Some(offer) = self.session.as_ref().map(|s| s.client.sign_in_offer()) else { return };
                return self.offer_sign_in(app, offer);
            }
            Cmd::Plugin(c) => return self.plugin(app, c),
            Cmd::Backup { restore: false, path, password } => {
                let tx = self.tx.clone();
                return self.root.make_backup(path.into(), password, move |r| {
                    let _ = tx.send(Msg::BackedUp(r));
                });
            }
            Cmd::Backup { restore: true, path, password } => {
                let tx = self.tx.clone();
                return self.root.restore_backup(path.into(), password, move |r| {
                    let _ = tx.send(Msg::Restored(r.map(Box::new)));
                });
            }
            Cmd::SignInWindow => {
                let (Some(s), Some(crate::app::Overlay::Account { kind, pasting, flow, window_failed, .. })) = (&self.session, &mut app.overlay) else { return };
                #[cfg(feature = "sign-in-window")]
                {
                    if self.sign_in_window.as_mut().is_some_and(|w| w.open_now()) {
                        return;
                    }
                    let (h, tx) = ((**s).clone(), self.tx.clone());
                    let title = format!("Sign in to {}", crate::text::provider_of(&app.settings.own.kinds, kind).name);
                    self.sign_in_window = nori_sign_in_window::SignInWindow::open(app.settings.own.kinds.clone(), kind.clone(), title, move |_, cookies| {
                        let _ = tx.send(Msg::SignedIn(h.sign_in(cookies)));
                    });
                    if self.sign_in_window.is_some() {
                        // Its cookies are the answer: one is on its way (already so when a key opened it).
                        flow.on(SignInEvent::Sent);
                        return;
                    }
                }
                let _ = (s, kind);
                *pasting = true;
                flow.on(SignInEvent::Unsent);
                *window_failed = true;
                return;
            }
            Cmd::SignInCode => {
                let Some(s) = &self.session else { return };
                self.codes += 1;
                let heard = Heard { session: (**s).clone(), tx: self.tx.clone(), code: self.codes };
                self.code = Some((self.codes, s.client.clone().sign_in_with_code(Arc::new(heard))));
                return;
            }
            Cmd::SignIn(cookies) => {
                let (Some(s), tx) = (&self.session, self.tx.clone()) else { return };
                let h = (**s).clone();
                nori_host::spawn("nori-sign-in", move || {
                    let _ = tx.send(Msg::SignedIn(h.sign_in(cookies)));
                });
                return;
            }
            Cmd::SignOut => {
                if let Some(s) = &self.session {
                    s.sign_out();
                }
                return self.reopen(app);
            }
            Cmd::StopReminding => {
                if let Some(s) = &self.session {
                    s.client.stop_reminding();
                }
                return;
            }
            Cmd::JoinJam(link) => {
                let (http, tx, remote, settings) = (self.http.clone(), self.tx.clone(), self.session.as_ref().and_then(|s| s.remote()), self.root.app.settings.clone());
                nori_host::spawn("nori-jam-join", move || {
                    let joined = nori_host::jam_join(http, &settings, link, &nori_host::device_name(), remote);
                    let _ = tx.send(Msg::Joined(joined.map_err(|e| crate::text::jam_join_failed(&e))));
                });
                return;
            }
            Cmd::Login(draft) => {
                let (root, http, tx) = (self.root.clone(), self.http.clone(), self.tx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::LoggedIn(crate::backend::check_login(&root, http, *draft)));
                });
                return;
            }
            Cmd::Setting(name, value) => {
                // The first screen has no session, and its plugins' switch still works.
                let changed = match &self.session {
                    Some(s) => s.setting(&name, &value),
                    None => self.root.app.settings.edit_by_name(&name, &value),
                };
                if changed.is_none() {
                    app.say(format!("{name}: not a setting"), true);
                }
                prefs_changed(app);
                // Remote control or jams may have come or gone.
                self.remote_read(app);
                return;
            }
            Cmd::Keep(pref) => {
                let settings = &self.root.app.settings;
                let mut prefs = settings.current().unwrap_or_default();
                prefs.list_prefs.insert(pref.key, pref.value);
                settings.put(prefs);
                return prefs_changed(app);
            }
            Cmd::RenameProfile(id, name) => {
                nori_host::profile_renamed(&self.root.app.settings, &id, &name);
                return prefs_changed(app);
            }
            Cmd::ChooseIdentity(index) => {
                let (Some(s), tx) = (&self.session, self.tx.clone()) else { return };
                let h = (**s).clone();
                nori_host::spawn("nori-identity", move || {
                    let _ = tx.send(Msg::IdentityChosen(h.choose_identity(index).map_err(|e| crate::text::net_error(&e))));
                });
                return;
            }
            Cmd::SwitchServer(id) => {
                let settings = &self.root.app.settings;
                let Some(p) = nori_host::profile_picked(settings, &id) else { return };
                app.prefs_changed(settings.current().unwrap_or_default());
                self.open(app, p, None);
                app.go(View::Home);
                return;
            }
            _ => {}
        }
        let Some(s) = &self.session else { return };
        match c {
            Cmd::Load(req) => s.load(req),
            Cmd::Play { from: Some(page), shuffle: true, .. } if s.shuffle_page(&page) => {}
            Cmd::Play { songs, start, shuffle, from } => s.play(songs, start, shuffle, from),
            Cmd::PlayFetch(what, true) if s.shuffle_page(&what.origin()) => {}
            Cmd::PlayFetch(what, shuffle) => s.play_later(what, shuffle),
            Cmd::Enqueue(songs, next) => s.enqueue(songs, next),
            Cmd::EnqueueFetch(what, next) => s.enqueue_later(what, next),
            Cmd::Toggle => s.toggle(),
            Cmd::Next => s.next(),
            Cmd::Previous => s.previous(),
            Cmd::Seek(ms) => s.seek(ms),
            Cmd::Volume(v) => {
                s.set_volume(v);
                // A guest's volume is this computer's, while the player shows the jam.
                if !matches!(self.playback, Playback::There(_)) {
                    self.root.app.settings.keep_app_value(own::VOLUME, s.volume().to_string());
                }
                app.volume = v;
                app.settings.invalidate();
            }
            Cmd::Jump(i) => s.jump(i),
            Cmd::Remove(i) => s.remove(i),
            Cmd::Undo => s.undo(),
            Cmd::Clear => s.clear_upcoming(),
            Cmd::Move(from, to) => s.move_song(from, to),
            Cmd::Shuffle(on) => s.shuffle(on),
            Cmd::CycleRepeat => {
                s.cycle_repeat();
                app.say(crate::text::repeat(s.now().repeat), false);
            }
            Cmd::Download(songs) => s.download(songs),
            Cmd::ReadDownloads(ids) => {
                let states = s.download_states(&ids.iter().map(String::as_str).collect::<Vec<_>>());
                app.knows_downloads(ids, states);
            }
            Cmd::Sleep(choice) => s.sleep_pick(choice),
            Cmd::ExcludeFromMixes(id, on) => match s.core.mix_excluded_set(id.clone(), on) {
                Ok(()) if on => _ = app.excluded.insert(id),
                Ok(()) => _ = app.excluded.remove(&id),
                Err(e) => app.say(e.to_string(), true),
            },
            Cmd::AddToPlaylist { id, songs } => {
                let name = app.library.playlists.ready().and_then(|v| v.iter().find(|p| p.id == id)).map(|p| p.name.clone()).unwrap_or_default();
                self.playlist_write(s, nori_core::client::Write::AddToPlaylist { id, song_ids: songs }, crate::text::added_to(&name));
            }
            Cmd::NewPlaylist { name, songs } => {
                let said = crate::text::created(&name);
                self.playlist_write(s, nori_core::client::Write::CreatePlaylist { name, song_ids: songs }, said);
            }
            Cmd::RedrawMix(mix) => {
                let (client, tx, id) = (s.client.clone(), self.tx.clone(), s.id);
                nori_host::spawn("nori-mix", move || {
                    block_on(client.mix_ensure(mix.clone(), true));
                    let _ = tx.send(Msg::From(id, Box::new(Msg::Reload(crate::backend::Req::Mix(mix)))));
                });
            }
            Cmd::SaveMix { title, songs } => {
                let (client, tx, id) = (s.client.clone(), self.tx.clone(), s.id);
                nori_host::spawn("nori-save-mix", move || {
                    use nori_core::cache_policy::{Page, Read};
                    let send = |m: Msg| {
                        let _ = tx.send(Msg::From(id, Box::new(m)));
                    };
                    // Named after the mix, counted on where the account has that name (`saved_mix_name`).
                    let names = match block_on(client.read_now(Read::PlaylistList)) {
                        Ok(Page::Playlists { v }) => v.into_iter().map(|p| p.name).collect(),
                        _ => Vec::new(),
                    };
                    let name = nori_core::menus::saved_mix_name(title, names);
                    let ids = songs.iter().map(|s| s.id.clone()).collect();
                    match block_on(client.write(nori_core::client::Write::CreatePlaylist { name: name.clone(), song_ids: ids })) {
                        Ok(()) => {
                            send(Msg::Note { text: crate::text::created(&name), error: false });
                            send(Msg::Reload(crate::backend::Req::Playlists));
                        }
                        Err(e) => send(Msg::Note { text: crate::text::net_error(&e), error: true }),
                    }
                });
            }
            Cmd::LookForOffer => {
                let (h, tx, id) = ((**s).clone(), self.tx.clone(), s.id);
                nori_host::spawn("nori-offer", move || {
                    let _ = tx.send(Msg::From(id, Box::new(Msg::Offer(h.queue_offer()))));
                });
            }
            Cmd::TakeOffer(offer) => s.queue_offer_closed(offer, true),
            Cmd::DismissOffer(offer) => s.queue_offer_closed(offer, false),
            Cmd::ShuffleLibrary { albums } => {
                let (h, tx, id) = ((**s).clone(), self.tx.clone(), s.id);
                nori_host::spawn("nori-shuffle", move || {
                    use nori_core::{OriginKind, PageOrigin};
                    let (songs, kind) = if albums { (block_on(h.client.shuffle_albums()), OriginKind::ShuffleAlbums) } else { (block_on(h.client.shuffle_all()), OriginKind::ShuffleSongs) };
                    match songs {
                        Ok(songs) => h.play(songs, 0, false, Some(PageOrigin::new(kind, ""))),
                        Err(e) => {
                            let _ = tx.send(Msg::From(id, Box::new(Msg::Note { text: crate::text::net_error(&e), error: true })));
                        }
                    }
                });
            }
            Cmd::DownloadFetch(what) => s.download_later(what),
            Cmd::DownloadRemove(ids) => {
                ids.iter().for_each(|id| s.download_remove(id));
                s.load(crate::backend::Req::Downloads);
            }
            Cmd::Star(nori_core::client::Starrable::Song, id, on) => s.star_song(id, on, self.there()),
            Cmd::Star(kind, id, on) => s.star(kind, id, on),
            Cmd::Pin(id, on) => nori_core::browse::pin_playlist(&self.root.app.settings, id, on),
            Cmd::Pick(device) => {
                if let Some(r) = s.remote() {
                    r.pick(device);
                }
            }
            Cmd::CastOn(output) => {
                if let Some(r) = s.remote() {
                    r.cast_on(output);
                }
            }
            Cmd::Pair(link) => s.pair(link),
            Cmd::Forget(id) => {
                if let Some(r) = s.remote() {
                    r.forget(id);
                }
            }
            Cmd::JamStart => s.jam_open(),
            Cmd::JamEnd => {
                if let Some(r) = s.remote() {
                    r.jam_close();
                }
            }
            Cmd::JamDecide(request, accept) => {
                if let Some(r) = s.remote() {
                    r.jam_act(Op::Decide { request, accept });
                }
            }
            Cmd::LeaveJam => s.jam_leave(),
            Cmd::JamAlong(on) => {
                if let Some(r) = s.remote() { r.jam_along(on); }
            }
            Cmd::JamRemoteSongs(on) => {
                if let Some(r) = s.remote() { r.jam_remote_songs(on); }
            }
            Cmd::Listen(on) => {
                if let Some(r) = s.remote() {
                    r.listen(on);
                }
            }
            Cmd::Level(level, v) => sound_edited(s, app, self.root.app.settings.edit_level(level, v).map(|(e, _)| e)),
            Cmd::Graphic(i, gain) => sound_edited(s, app, self.root.app.settings.edit_graphic(i, gain).map(|(e, _)| e)),
            Cmd::Band(i, band) => sound_edited(s, app, self.root.app.settings.edit_band(i, band).map(|(e, _)| e)),
            Cmd::Sound(tool) => {
                let effect = match tool.tool().map(|t| self.root.app.settings.sound_tool(t)) {
                    Some(Ok(change)) => change.map(|c| c.effect),
                    Some(Err(e)) => {
                        app.say(format!("{e:?}"), true);
                        None
                    }
                    None => None,
                };
                sound_edited(s, app, effect);
            }
            Cmd::Action(c) => s.action(c),
            Cmd::Curve => s.curve_answer(),
            Cmd::CheckUpdate => s.update_check(true),
            Cmd::SoundPage(page, on) => s.sound_page(page, on),
            Cmd::SearchTyped(text) => {
                let v = s.search_typed(&text);
                app.search.view = Some(v);
            }
            Cmd::SearchServer(q) => {
                s.search_server(q);
            }
            Cmd::LyricsRefresh => s.lyrics_refresh(),
            Cmd::Cover(art) => {
                if let Some(t) = s.cover(art, COVER_PX) {
                    self.tickets.push(t);
                    if self.tickets.len() > 4 {
                        drop(self.tickets.remove(0));
                    }
                }
            }
            Cmd::Quit | Cmd::Keep(_) | Cmd::Mouse(_) | Cmd::Images(_) | Cmd::Paint(_) | Cmd::Device(_) | Cmd::Login(_) | Cmd::SwitchServer(_) | Cmd::RenameProfile(..) | Cmd::ChooseIdentity(_) | Cmd::Setting(..) | Cmd::JoinJam(_) | Cmd::OfferSignIn | Cmd::SignInWindow | Cmd::SignIn(_) | Cmd::SignInCode | Cmd::SignOut | Cmd::StopReminding | Cmd::Plugin(_) | Cmd::Backup { .. } => {}
        }
    }

}

impl Runner {
    /// Changes a playlist on the server, then says `said` and has the playlists read again.
    fn playlist_write(&self, s: &Session, w: nori_core::client::Write, said: String) {
        let (client, tx, id) = (s.client.clone(), self.tx.clone(), s.id);
        nori_host::spawn("nori-playlist", move || {
            let send = |m: Msg| {
                let _ = tx.send(Msg::From(id, Box::new(m)));
            };
            match block_on(client.write(w)) {
                Ok(()) => {
                    send(Msg::Note { text: said, error: false });
                    send(Msg::Reload(crate::backend::Req::Playlists));
                }
                Err(e) => send(Msg::Note { text: crate::text::net_error(&e), error: true }),
            }
        });
    }
}

/// After a sound edit: applies its effect (None: nothing changed).
fn sound_edited(s: &Session, app: &mut App, effect: Option<u32>) {
    if let Some(effect) = effect {
        s.applied(effect);
    }
    prefs_changed(app);
}

/// The browser shows what is searched for now.
fn narrowed(app: &mut App) {
    app.settings.invalidate();
    app.dirty = true;
}

fn prefs_changed(app: &mut App) {
    if let Some(p) = app.shared.settings.current() {
        app.prefs_changed(p);
    }
}

/// `m` as the open session (`open`) sent it; None for what an earlier session left in flight.
fn current(open: Option<u64>, m: Msg) -> Option<Msg> {
    match m {
        Msg::From(id, m) if Some(id) == open => Some(*m),
        Msg::From(..) => None,
        m => Some(m),
    }
}

/// What the runner's `code`th code sign-in says, as its messages; signed in, the session finishes it as
/// any sign-in.
struct Heard {
    session: nori_host::session::Handle,
    tx: Sender<Msg>,
    code: u32,
}

impl nori_core::providers::code::CodeHeard for Heard {
    fn heard(&self, event: SignInEvent) {
        let _ = self.tx.send(match event {
            SignInEvent::Answered { result } => {
                self.session.signed_in(&result);
                Msg::SignedIn(result)
            }
            event => Msg::CodeStep { code: self.code, event },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_session_messages_dropped() {
        let note = |id| Msg::From(id, Box::new(Msg::Note { text: "x".into(), error: false }));
        assert!(matches!(current(Some(2), note(2)), Some(Msg::Note { .. })));
        assert!(current(Some(2), note(1)).is_none());
        assert!(current(None, note(1)).is_none());
        assert!(matches!(current(Some(2), Msg::Resize), Some(Msg::Resize)));
    }
}
