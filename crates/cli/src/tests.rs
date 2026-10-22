//! Screens drawn into a TestBackend with keys and clicks dispatched. `NORI_TUI_DUMP=<dir>` writes the
//! drawn screens as text.

use std::time::{Duration, Instant};

use nori_core::settings::StoredPrefs;
use nori_core::{Album, Song};
use nori_engine::State;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::Terminal;

use crate::app::{App, Button, Cmd, Focus, Hit, Item, ListRef, Overlay, Panel, Target, View};
use crate::backend::{Data, Msg, Req};
use crate::settings_view::{Facts, Line, Row, SettingsView, Switch, GROUPS};

fn app() -> App {
    let mut a = App::new(StoredPrefs::default());
    a.server = "music.example".into();
    a
}

#[test]
fn youtube_radio_uses_the_playing_song() {
    let mut a = app();
    a.rules = nori_core::providers::rules_of(nori_core::settings::SavedServer { kind: nori_core::settings::ServerKind::YouTubeMusic, ..Default::default() }, false);
    a.song = Some(Song { id: "yt-song".into(), title: "Song".into(), ..Default::default() });
    let screen = draw(&mut a, 120, 36);
    assert!(!screen.contains("Radio"), "no words beside the player's buttons:\n{screen}");
    key(&mut a, KeyCode::Char('t'));
    assert!(matches!(a.cmds.last(), Some(Cmd::PlayFetch(crate::backend::Fetch::Radio(id), false)) if id == "yt-song"));
}

fn draw(a: &mut App, w: u16, h: u16) -> String {
    draw_with(a, w, h, None)
}

fn draw_with(a: &mut App, w: u16, h: u16, art: Option<&mut crate::art::Art>) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::draw(f, a, art)).unwrap();
    let buf = t.backend().buffer();
    LAST.with(|l| *l.borrow_mut() = Some(buf.clone()));
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut x = 0;
        while x < buf.area.width {
            let symbol = buf[(x, y)].symbol();
            out.push_str(symbol);
            // A wide character covers the next cell as well.
            x += if unicode_width::UnicodeWidthStr::width(symbol) == 2 { 2 } else { 1 };
        }
        out.push('\n');
    }
    out
}

thread_local! {
    /// The last screen drawn, with its colors, for the HTML dump.
    static LAST: std::cell::RefCell<Option<ratatui::buffer::Buffer>> = const { std::cell::RefCell::new(None) };
}

/// Writes a drawn screen to $NORI_TUI_DUMP, if set: as text, and as an HTML page with its colors.
fn dump(name: &str, text: &str) {
    if let Some(dir) = std::env::var_os("NORI_TUI_DUMP") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.txt")), text);
        if let Some(buf) = LAST.with(|l| l.borrow().clone()) {
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.html")), html(&buf));
        }
    }
}

/// A screen as a page of colored cells, to look at in a browser.
fn html(buf: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::{Color, Modifier};
    let hex = |c: Color, default: &str| match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("var(--c{i})"),
        _ => default.to_string(),
    };
    let mut out = String::from("<!doctype html><meta charset=utf-8><body style='margin:0;background:#000'><pre style='margin:0;font:14px/17px Menlo,monospace'>");
    for y in 0..buf.area.height {
        let mut x = 0;
        while x < buf.area.width {
            let c = &buf[(x, y)];
            let wide = unicode_width::UnicodeWidthStr::width(c.symbol()) == 2;
            let (mut fg, mut bg) = (hex(c.fg, "#ccc"), hex(c.bg, "#000"));
            if c.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let style = format!("color:{fg};background:{bg}{}{}{}", if c.modifier.contains(Modifier::BOLD) { ";font-weight:700" } else { "" }, if c.modifier.contains(Modifier::UNDERLINED) { ";text-decoration:underline" } else { "" }, if c.modifier.contains(Modifier::DIM) { ";opacity:.6" } else { "" });
            let sym = c.symbol().replace('&', "&amp;").replace('<', "&lt;");
            let sym = if sym.starts_with('\x1b') { " ".to_string() } else { sym };
            out.push_str(&format!("<span style='{style}'>{sym}</span>"));
            x += if wide { 2 } else { 1 };
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

fn key(a: &mut App, code: KeyCode) {
    a.handle(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn chars(a: &mut App, s: &str) {
    for c in s.chars() {
        key(a, KeyCode::Char(c));
    }
}

#[test]
fn downloads_open_saved_album_tracks() {
    let mut a = app();
    a.view = View::Downloads;
    a.focus = Focus::Main;
    let tracks = ["Saved one", "Saved two"].map(|title| Song {
        id: title.into(), title: title.into(), album_id: Some("saved-album".into()),
        album: "Saved album".into(), ..Default::default()
    }).to_vec();
    let stored = nori_core::library::downloaded_library(tracks.clone());
    a.handle(Msg::Data(Req::Downloads, Ok(Data::Downloads(Box::new(crate::backend::Downloads { stored, ..Default::default() })))));
    let screen = draw(&mut a, 120, 30);
    assert!(screen.contains("Saved album") && !screen.contains("Saved one"), "{screen}");
    a.downloads_sel.at = a.download_rows().iter().position(|r| matches!(r, crate::app::DownloadRow::Album(_))).unwrap();
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Load(Req::DownloadedAlbum("saved-album".into()))));
    let detail = nori_core::library::downloaded_album(tracks).unwrap();
    a.handle(Msg::Data(Req::DownloadedAlbum("saved-album".into()), Ok(Data::Album(Box::new(detail)))));
    let screen = draw(&mut a, 120, 30);
    assert!(screen.contains("Saved one") && screen.contains("Saved two"), "{screen}");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.cmds.last(), Some(Cmd::Play { songs, from: Some(origin), .. }) if songs.len() == 2 && origin.kind == nori_core::OriginKind::Downloads));
    key(&mut a, KeyCode::Esc);
    assert!(a.page().is_none());
}

fn click(a: &mut App, x: u16, y: u16) {
    let m = |kind| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    a.handle(Msg::Mouse(m(MouseEventKind::Down(MouseButton::Left))));
    a.handle(Msg::Mouse(m(MouseEventKind::Up(MouseButton::Left))));
}

/// Whether the seal of `button`, as last drawn, is filled: the symbol on the accent.
fn seal_filled(a: &App, button: crate::app::Button) -> bool {
    let r = hit_rect(a, Hit::Button(button));
    LAST.with(|l| {
        let buf = l.borrow();
        let buf = buf.as_ref().expect("drawn");
        let cells: Vec<_> = (r.x..r.x + r.width).map(|x| &buf[(x, r.y)]).collect();
        assert!(cells.iter().any(|c| c.symbol() == "好"), "the seal at {r:?}");
        cells.iter().any(|c| c.symbol() == "好" && c.bg == a.look.accent)
    })
}

const SPEAKERS: [&str; 4] = ["🔇", "🔈", "🔉", "🔊"];

fn hit_rect(a: &App, h: Hit) -> Rect {
    a.hits.iter().find(|(_, x)| *x == h).map(|(r, _)| *r).unwrap_or_else(|| panic!("{h:?} is not on screen"))
}

fn song(id: &str, title: &str, secs: u32) -> Song {
    Song { id: id.into(), title: title.into(), artist: "Artist".into(), album: "Album".into(), duration: secs, ..Default::default() }
}

fn album(id: &str, name: &str) -> Album {
    Album { id: id.into(), name: name.into(), artist: "Someone".into(), year: 2001, ..Default::default() }
}

fn queue_of(n: usize, index: i32) -> nori_core::playlist::PlaylistView {
    let songs: Vec<Song> = (0..n).map(|i| song(&format!("s{i}"), &format!("Song {i}"), 100)).collect();
    nori_core::playlist::PlaylistView { songs, len: n as u32, list_rev: 1, order: (0..n as u32).collect(), queued: vec![3], index, shuffle: false, repeat: 0, bridging: false, rev: 1 }
}

fn playlists(a: &mut App, names: &[&str]) {
    let v = names.iter().enumerate().map(|(i, n)| nori_core::Playlist { id: format!("pl-{i}"), name: n.to_string(), ..Default::default() }).collect();
    a.handle(Msg::Data(Req::Playlists, Ok(Data::Playlists(v))));
}

#[test]
fn window_layout() {
    let mut a = app();
    a.go(View::Home);
    playlists(&mut a, &["Morning", "Late night"]);
    let s = draw(&mut a, 120, 36);
    for word in ["Search", "Home", "Albums", "Artists", "Songs", "Playlists", "Favorites", "✱", "Downloads", "Equalizer", "music.example", "space play", "Nothing playing", "⏮", "⏭", "╭", "╰"] {
        assert!(s.contains(word), "{word} missing:\n{s}");
    }
    for gone in ["LIBRARY", "PLAYLISTS", "nori", "Morning"] {
        assert!(!s.contains(gone), "{gone} is the sidebar's:\n{s}");
    }
    assert!(s.lines().next().unwrap().contains('╭'), "the outline starts on the first row:\n{s}");
    dump("empty", &s);
    dump("empty-80x24", &draw(&mut a, 80, 24));
}

#[test]
fn small_terminals() {
    let mut a = app();
    a.go(View::Settings);
    a.say("A note far longer than the page is wide, so it takes the whole width", false);
    let s = draw(&mut a, 40, 14);
    assert!(s.contains("│ A note far"), "{s}");

    // Tiny terminal no panic.
    let mut a = app();
    a.song = Some(song("1", "One", 200));
    a.queue = Some(queue_of(5, 1));
    for (w, h) in [(20, 6), (40, 10), (1, 1), (80, 4), (60, 20), (200, 3)] {
        for n in 0..9 {
            a.do_action(crate::keys::Action::Go(n));
            draw(&mut a, w, h);
        }
        a.devices.on = true;
        a.devices.jam = Some(jam());
        for p in [Panel::Queue, Panel::Lyrics, Panel::Devices] {
            a.set_panel(p);
            draw(&mut a, w, h);
            a.set_panel(p);
        }
        a.full = true;
        draw(&mut a, w, h);
        a.full = false;
    }

    // Narrow: the tabs are icons, the player keeps its song, transport and seek bar.
    let mut a = app();
    a.go(View::Home);
    a.song = Some(song("1", "One", 200));
    let s = draw(&mut a, 70, 24);
    assert!(s.contains('⌂') && s.contains('◫') && !s.contains("Albums") && (s.contains('⏸') || s.contains('▶')) && s.contains("0:00"), "{s}");
    assert!(!s.contains('⤢') && !s.contains('❝') && !SPEAKERS.iter().any(|v| s.contains(v)), "no buttons after the seal or volume this narrow:\n{s}");
    dump("narrow", &s);
    // Below 60 columns the player is the song and play.
    let s = draw(&mut a, 50, 20);
    assert!(s.contains("One") && !s.contains('⏮') && !s.contains("0:00"), "{s}");
}

#[test]
fn keys_and_focus() {
    let mut a = app();
    key(&mut a, KeyCode::Char('2'));
    assert_eq!(a.view, View::Albums);
    assert!(a.cmds.contains(&Cmd::Load(Req::Albums { offset: 0 })));
    key(&mut a, KeyCode::Char('3'));
    assert_eq!(a.view, View::Artists);
    assert!(a.cmds.contains(&Cmd::Load(Req::Artists)));
    a.cmds.clear();
    key(&mut a, KeyCode::Char(' '));
    key(&mut a, KeyCode::Char('n'));
    key(&mut a, KeyCode::Char('p'));
    key(&mut a, KeyCode::Char('+'));
    assert_eq!(a.cmds, [Cmd::Toggle, Cmd::Next, Cmd::Previous, Cmd::Volume(1.0)]);
    key(&mut a, KeyCode::Char('-'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Volume(0.95)));
    // The search field takes every key while it is typed in, and the server is asked once typing pauses.
    key(&mut a, KeyCode::Char('/'));
    assert_eq!(a.view, View::Search);
    chars(&mut a, "n q");
    assert_eq!(a.search.text, "n q", "n and q are letters in the field");
    assert!(!a.quit);
    assert_eq!(a.cmds.last(), Some(&Cmd::SearchTyped("n q".into())));
    let due = a.search.ask_at.expect("the server is asked later");
    a.tick(due + Duration::from_millis(1));
    assert_eq!(a.cmds.last(), Some(&Cmd::SearchServer("n q".into())));
    key(&mut a, KeyCode::Esc);
    key(&mut a, KeyCode::Char('?'));
    assert!(matches!(a.overlay, Some(Overlay::Help { .. })));
    let s = draw(&mut a, 100, 70);
    assert!(s.contains("Play or pause") && s.contains("Take it out") && s.contains("queue or its lyrics"), "{s}");
    dump("help", &s);
    key(&mut a, KeyCode::Char('x'));
    assert!(a.overlay.is_none());
    key(&mut a, KeyCode::Char('q'));
    assert!(a.quit);

    // Tab moves between the full player's panels, and has nothing to move on a page.
    let mut a = app();
    a.go(View::Home);
    key(&mut a, KeyCode::Tab);
    assert!(!a.full && a.focus == Focus::Main);
    key(&mut a, KeyCode::Char('F'));
    assert_eq!((a.full, a.panel, a.focus), (true, Panel::Lyrics, Focus::Panel));
    key(&mut a, KeyCode::Tab);
    assert_eq!(a.panel, Panel::Playing, "the devices come only with remote control on");
    key(&mut a, KeyCode::Tab);
    assert_eq!(a.panel, Panel::Queue);
    key(&mut a, KeyCode::Tab);
    assert_eq!(a.panel, Panel::Lyrics);
    key(&mut a, KeyCode::Esc);
    assert_eq!((a.full, a.focus), (false, Focus::Main));

    // The Playlists tab lists them: a queues one, enter opens it as a page.
    key(&mut a, KeyCode::Char('5'));
    assert_eq!(a.view, View::Playlists);
    assert!(a.cmds.contains(&Cmd::Load(Req::Playlists)));
    playlists(&mut a, &["Morning"]);
    let s = draw(&mut a, 120, 36);
    assert!(s.contains("Morning"), "{s}");
    a.cmds.clear();
    key(&mut a, KeyCode::Char('a'));
    assert!(matches!(a.cmds.last(), Some(Cmd::EnqueueFetch(crate::backend::Fetch::Playlist(id), false)) if id == "pl-0"), "{:?}", a.cmds);
    key(&mut a, KeyCode::Enter);
    assert!(a.cmds.contains(&Cmd::Load(Req::Playlist("pl-0".into()))));
    assert!(a.page().is_some() && a.view == View::Playlists);
    key(&mut a, KeyCode::Esc);
    assert!(a.page().is_none());
    // Favorites asks for the starred songs.
    key(&mut a, KeyCode::Char('6'));
    assert!(a.cmds.contains(&Cmd::Load(Req::Favorites)));
    a.handle(Msg::Data(Req::Favorites, Ok(Data::Favorites(vec![Song { starred: true, ..song("f1", "Kept", 100) }]))));
    let s = draw(&mut a, 120, 36);
    assert!(s.contains("Favorites") && s.contains("Kept"), "{s}");
    // 7 to 9: downloads, the equalizer, settings.
    for (k, v) in [('7', View::Downloads), ('8', View::Equalizer), ('9', View::Settings)] {
        key(&mut a, KeyCode::Char(k));
        assert_eq!(a.view, v);
    }

    // Focus redraw.
    let mut a = app();
    a.dirty = false;
    a.handle(Msg::Focus(false));
    assert!(!a.dirty, "focus lost changes nothing");
    a.handle(Msg::Focus(true));
    assert!(a.dirty, "focus back redraws");
    // A no-op message after a real change in the same batch keeps the redraw.
    a.dirty = false;
    a.handle(Msg::Engine(nori_engine::Event::State(State::Paused)));
    a.handle(Msg::Focus(false));
    assert!(a.dirty);
}

#[test]
fn album_grid_keys_and_clicks() {
    let mut a = app();
    a.go(View::Albums);
    let albums: Vec<Album> = (0..100).map(|i| album(&format!("al-{i}"), &format!("Album {i}"))).collect();
    a.handle(Msg::Data(Req::Albums { offset: 0 }, Ok(Data::Albums(albums))));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("Album 0") && s.contains("Someone · 2001") && s.contains("╭"), "{s}");
    dump("albums", &s);
    let cols = a.shown.cols;
    assert!(cols >= 3, "{cols} cards a row");
    // → along the row, ↓ a row down; seeking stays on , and .
    key(&mut a, KeyCode::Right);
    assert_eq!(a.library.albums_sel.at, 1);
    key(&mut a, KeyCode::Down);
    assert_eq!(a.library.albums_sel.at, 1 + cols);
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Seek(_))));
    // One click on a card opens its album; esc comes back.
    let card = hit_rect(&a, Hit::Row(ListRef::Albums, 2));
    a.cmds.clear();
    click(&mut a, card.x + 2, card.y + 1);
    assert!(a.cmds.contains(&Cmd::Load(Req::Album("al-2".into()))), "{:?}", a.cmds);
    key(&mut a, KeyCode::Esc);
    assert!(a.page().is_none());
    // The wheel moves a row at a time.
    draw(&mut a, 160, 40);
    let list = hit_rect(&a, Hit::List(ListRef::Albums));
    let at = a.library.albums_sel.at;
    let wheel = MouseEvent { kind: MouseEventKind::ScrollDown, column: list.x + 1, row: list.y + 1, modifiers: KeyModifiers::NONE };
    a.handle(Msg::Mouse(wheel));
    assert_eq!(a.library.albums_sel.at, at + cols);
    // The tabs are clicked too.
    let r = hit_rect(&a, Hit::Tab(View::Songs));
    click(&mut a, r.x + 3, r.y);
    assert_eq!(a.view, View::Songs);
}

#[test]
fn a_feed_shelf_shows_songs_and_playlists_too() {
    let mut a = app();
    a.go(View::Home);
    let songs = vec![song("s1", "Quick One", 200), song("s2", "Quick Two", 180)];
    let list = nori_core::Playlist { id: "p1".into(), name: "Mixed Bag".into(), ..Default::default() };
    let cards = vec![Item::Song(songs.clone(), 0), Item::Song(songs.clone(), 1), Item::Playlist(list)];
    a.handle(Msg::Data(Req::Home, Ok(Data::HomeRow(0, "Quick picks".into(), cards))));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("Quick Two") && s.contains("Mixed Bag"), "{s}");
    key(&mut a, KeyCode::Right);
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert!(a.cmds.iter().any(|c| matches!(c, Cmd::Play { songs: v, start: 1, .. } if v.len() == 2) || matches!(c, Cmd::Play { songs: v, .. } if v[0].id == "s2")), "the shelf's songs from the second: {:?}", a.cmds);
    key(&mut a, KeyCode::Right);
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert!(a.cmds.contains(&Cmd::Load(Req::Playlist("p1".into()))), "{:?}", a.cmds);
}

#[test]
fn home_shelves() {
    let mut a = app();
    a.go(View::Home);
    let shelf = |n: usize, p: &str| (0..n).map(|i| Item::Album(album(&format!("{p}-{i}"), &format!("{p} {i}")))).collect::<Vec<_>>();
    let id = |a: &App| match a.home.item() { Some(Item::Album(al)) => al.id.clone(), _ => String::new() };
    a.handle(Msg::Data(Req::Home, Ok(Data::HomeRow(0, "Recently added".into(), shelf(12, "New")))));
    a.handle(Msg::Data(Req::Home, Ok(Data::HomeRow(2, "Most played".into(), shelf(3, "Top")))));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("Recently added") && s.contains("Most played") && s.contains("New 0") && s.contains("Top 2"), "{s}");
    assert!(s.contains("Listen now"), "{s}");
    dump("home", &s);
    key(&mut a, KeyCode::Right);
    key(&mut a, KeyCode::Right);
    assert_eq!(id(&a), "New-2");
    key(&mut a, KeyCode::Down);
    assert_eq!(id(&a), "Top-0", "down to the next shelf with albums on it");
    key(&mut a, KeyCode::Up);
    assert_eq!(id(&a), "New-2", "each shelf keeps its place");
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert!(a.cmds.contains(&Cmd::Load(Req::Album("New-2".into()))));
    // Far along a shelf, it scrolls to keep the album in view.
    key(&mut a, KeyCode::Esc);
    for _ in 0..9 {
        key(&mut a, KeyCode::Right);
    }
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("New 11") && !s.contains("New 0 "), "{s}");
}

#[test]
fn mouse() {
    let mut a = app();
    a.song = Some(song("1", "One", 200));
    a.now.state = State::Playing;
    a.now.at = Instant::now();
    let s = draw(&mut a, 160, 30);
    assert!(s.contains("One") && s.contains("3:20"), "{s}");
    dump("player", &s);
    let bar = hit_rect(&a, Hit::Seek);
    let at = |share: f32| bar.x + ((bar.width - 1) as f32 * share) as u16;
    let m = |kind, x| Msg::Mouse(MouseEvent { kind, column: x, row: bar.y, modifiers: KeyModifiers::NONE });
    a.handle(m(MouseEventKind::Down(MouseButton::Left), at(0.25)));
    a.handle(m(MouseEventKind::Drag(MouseButton::Left), at(0.5)));
    assert!(matches!(a.drag, Some(crate::app::Drag::Seek(s)) if (s - 0.5).abs() < 0.02), "the bar follows the drag");
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Seek(_))), "nothing is sought while dragging");
    a.handle(m(MouseEventKind::Up(MouseButton::Left), at(0.5)));
    let Some(Cmd::Seek(ms)) = a.cmds.last() else { panic!("{:?}", a.cmds) };
    assert!((ms - 100_000).abs() < 3_000, "{ms}");
    // Buttons in the bar.
    let next = hit_rect(&a, Hit::Button(crate::app::Button::Next));
    click(&mut a, next.x + 1, next.y);
    assert_eq!(a.cmds.last(), Some(&Cmd::Next));
    // The volume: a click where it should be, the wheel over it.
    let vol = hit_rect(&a, Hit::Volume);
    click(&mut a, vol.x + vol.width / 2, vol.y);
    assert!(matches!(a.cmds.last(), Some(Cmd::Volume(v)) if (v - 0.5).abs() < 0.06), "{:?}", a.cmds.last());
    // The panel's switches.
    let lyrics = hit_rect(&a, Hit::Button(crate::app::Button::Panel(Panel::Lyrics)));
    click(&mut a, lyrics.x + 1, lyrics.y);
    assert_eq!((a.full, a.panel), (true, Panel::Lyrics));

    // Mouse off ignores clicks.
    let mut a = app();
    draw(&mut a, 160, 30);
    key(&mut a, KeyCode::Char('m'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Mouse(false)));
    let r = hit_rect(&a, Hit::Tab(View::Settings));
    click(&mut a, r.x + 1, r.y);
    assert_eq!(a.view, View::Home);

    // Song table double click plays.
    let mut a = app();
    a.go(View::Songs);
    let songs: Vec<Song> = (0..30).map(|i| Song { starred: i == 2, ..song(&format!("s{i}"), &format!("Song {i}"), 200) }).collect();
    a.handle(Msg::Data(Req::Songs { offset: 0 }, Ok(Data::Songs(songs, true))));
    a.heard(Some(song("s1", "Song 1", 200)));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("Title") && s.contains("Artist") && s.contains("Album") && s.contains("Time"), "{s}");
    let line = |name: &str| s.lines().find(|l| l.contains(name)).unwrap_or_else(|| panic!("{name} missing:\n{s}")).to_string();
    assert!(line("Song 1").contains('▶') && line("Song 2").contains('好') && !line("Song 3").contains('好'), "the song heard and the favorite are marked:\n{s}");
    dump("songs", &s);
    let row = hit_rect(&a, Hit::Row(ListRef::Songs, 4));
    a.cmds.clear();
    click(&mut a, row.x + 5, row.y);
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Play { .. })), "one click selects");
    click(&mut a, row.x + 5, row.y);
    assert!(matches!(a.cmds.last(), Some(Cmd::Play { start: 4, .. })), "{:?}", a.cmds);
}

#[test]
fn settings_page() {
    let mut a = app();
    a.go(View::Settings);
    a.settings.set_facts(Facts { devices: vec!["hw:0".into()], ..Facts::default() });
    let s = draw(&mut a, 170, 260);
    dump("settings", &s);
    let pages = a.settings.pages(&a.prefs.clone()).to_vec();
    for (g, page) in GROUPS.iter().zip(&pages) {
        assert!(s.contains(g.title), "{}: group missing", g.title);
        for section in &page.sections {
            if !section.title.is_empty() {
                assert!(s.contains(&section.title), "{}: section {} missing", g.title, section.title);
            }
            for row in &section.rows {
                let title = match row {
                    Row::Toggle { title, .. } | Row::Choice { title, .. } | Row::Link { title, .. } | Row::Action { title, .. } => title.trim(),
                    _ => continue,
                };
                assert!(s.contains(title), "{}: {title} missing:\n{s}", g.title);
            }
        }
    }
    // What only a phone can do, or a desk has no use for, is not listed here.
    for phone in ["System audio effects", "Save battery", "Swipe", "Moving covers", "Black background", "Covers fetched ahead", "offload", "Search delay", "bitrate cap"] {
        assert!(!s.contains(phone), "{phone} listed in a terminal:\n{s}");
    }
    // The page never rests on a heading; a switch shows its state, and enter asks the core to change it by
    // its own name.
    let lines = SettingsView::lines(&pages);
    assert!(matches!(lines[a.settings.row.at], Line::Row(_)));
    a.cmds.clear();
    let at = lines.iter().position(|l| matches!(l, Line::Row(Row::Toggle { switch: Switch::Setting(name), .. }) if name == "autoMix")).unwrap();
    while a.settings.row.at < at {
        key(&mut a, KeyCode::Down);
        assert!(matches!(lines[a.settings.row.at], Line::Row(_)), "no stop on a heading");
    }
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Setting("autoMix".into(), "true".into())));
    // ← and → change a choice (the crossfade, below AutoMix); here they do not seek.
    let at = lines.iter().position(|l| matches!(l, Line::Row(Row::Choice { target: Target::Setting(name), .. }) if name == "crossfadeSec")).unwrap();
    while a.settings.row.at < at {
        key(&mut a, KeyCode::Down);
    }
    a.cmds.clear();
    key(&mut a, KeyCode::Right);
    assert!(matches!(a.cmds.last(), Some(Cmd::Setting(n, _)) if n == "crossfadeSec"), "{:?}", a.cmds);
    // The output device is the terminal's own: kept for the next start.
    let at = lines.iter().position(|l| matches!(l, Line::Row(Row::Choice { target: Target::Device, .. }))).unwrap();
    key(&mut a, KeyCode::Char('g'));
    while a.settings.row.at < at {
        key(&mut a, KeyCode::Down);
    }
    a.cmds.clear();
    key(&mut a, KeyCode::Right);
    assert_eq!(a.cmds.last(), Some(&Cmd::Device("hw:0".into())));
    // Transparent on stops painting the page.
    let at = lines.iter().position(|l| matches!(l, Line::Row(Row::Toggle { switch: Switch::Transparent, .. }))).unwrap();
    key(&mut a, KeyCode::Char('g'));
    while a.settings.row.at < at {
        key(&mut a, KeyCode::Down);
    }
    a.cmds.clear();
    key(&mut a, KeyCode::Right);
    assert_eq!(a.cmds.last(), Some(&Cmd::Paint(false)));
}

#[test]
fn load_error_shown() {
    let mut a = app();
    a.go(View::Albums);
    a.handle(Msg::Data(Req::Albums { offset: 0 }, Err("The server did not answer: HTTP 522".into())));
    let s = draw(&mut a, 120, 24);
    assert!(s.contains("Could not load this page") && s.contains("HTTP 522") && s.contains("R tries again"), "{s}");
    dump("error", &s);
    a.cmds.clear();
    key(&mut a, KeyCode::Char('R'));
    assert!(a.cmds.contains(&Cmd::Load(Req::Albums { offset: 0 })));
    a.unreachable = Some("The server did not answer: HTTP 522".into());
    let s = draw(&mut a, 120, 24);
    assert!(s.contains("unreachable"), "{s}");
}

#[test]
fn queue_panel_editing() {
    let mut a = app();
    a.queue = Some(queue_of(5, 1));
    key(&mut a, KeyCode::Char('Q'));
    assert_eq!((a.full, a.panel, a.focus), (true, Panel::Queue, Focus::Panel));
    let s = draw(&mut a, 160, 30);
    assert!(s.contains("▶ Song 1") && s.contains("+ Song 3"), "{s}");
    dump("queue", &s);
    assert_eq!(a.queue_sel.at, 1, "the queue opens on the song playing");
    key(&mut a, KeyCode::Char('j'));
    key(&mut a, KeyCode::Char('d'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Remove(2)));
    key(&mut a, KeyCode::Char('u'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Restore("s2".into())), "u puts it back");
    let sent = a.cmds.len();
    key(&mut a, KeyCode::Char('u'));
    assert_eq!(a.cmds.len(), sent, "once");
    key(&mut a, KeyCode::Char('K'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Move(2, 1)));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Jump(1)));
    key(&mut a, KeyCode::Char('s'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Shuffle(true)));
    key(&mut a, KeyCode::Char('r'));
    assert_eq!(a.cmds.last(), Some(&Cmd::CycleRepeat));
    // Q again goes back to the page, which has the keys.
    key(&mut a, KeyCode::Char('Q'));
    assert_eq!((a.full, a.focus), (false, Focus::Main));
}

#[test]
fn full_player_shows_the_song_and_the_mix() {
    let mut a = app();
    let next = song("s2", "Second", 180);
    crate::backend::app().register(vec![next.clone()]);
    a.heard(Some(Song { year: 2020, suffix: "flac".into(), ..song("s1", "First", 200) }));
    a.now.state = State::Playing;
    a.now.mixing = true;
    a.transition = Some(nori_core::automix::planner::TransitionNote {
        outgoing_id: "s1".into(),
        incoming_id: "s2".into(),
        kind: nori_core::automix::planner::TransitionKind::BeatMatched,
        start_ms: 190_000,
        duration_ms: 8_000,
        tempo_ratio: 1.02,
        reason: "bars aligned".into(),
    });
    assert!(!a.transition_shown(), "nothing to say about it on a page");
    // F: the player over the whole window.
    key(&mut a, KeyCode::Char('F'));
    assert!(a.full && a.lyrics_shown() && a.transition_shown());
    let s = draw(&mut a, 170, 40);
    assert!(s.contains("First") && s.contains("Mixing into the next") && s.contains("beat-matched") && s.contains("Next: “Second”") && s.contains("FLAC"), "{s}");
    assert!(!s.contains("Listen now") && !s.contains("Albums"), "the player replaces the page and its tabs:\n{s}");
    dump("full", &s);
    dump("full-80x24", &draw(&mut a, 80, 24));
    key(&mut a, KeyCode::Esc);
    assert!(!a.full);
    // The bar's buttons after the seal: the song alone, the queue, the lyrics, then full screen as it was.
    draw(&mut a, 170, 40);
    let playing = hit_rect(&a, Hit::Button(Button::Panel(Panel::Playing)));
    click(&mut a, playing.x + 1, playing.y);
    assert_eq!((a.full, a.panel), (true, Panel::Playing));
    let s = draw(&mut a, 170, 40);
    assert!(s.contains("First") && s.contains("Mixing into the next") && s.contains("♫ Playing") && s.contains("≡ Queue") && !s.contains("Looking for lyrics"), "the song alone:\n{s}");
    dump("full-playing", &s);
    let queue = hit_rect(&a, Hit::Button(Button::Panel(Panel::Queue)));
    click(&mut a, queue.x + 1, queue.y);
    assert_eq!(a.panel, Panel::Queue);
    key(&mut a, KeyCode::Esc);
    draw(&mut a, 170, 40);
    // The last: the song's title opens the full player too.
    let full = a.hits.iter().rev().find(|h| h.1 == Hit::Button(Button::Full)).expect("full screen").0;
    click(&mut a, full.x + 1, full.y);
    assert_eq!((a.full, a.panel), (true, Panel::Queue), "full screen opens on the panel last shown");
}

#[test]
fn lyrics_panel() {
    use nori_core::{LyricLine, LyricWord, Lyrics};
    let mut a = app();
    a.heard(Some(song("s1", "First", 200)));
    key(&mut a, KeyCode::Char('L'));
    assert!(a.cmds.contains(&Cmd::Lyrics("s1".into())));
    let words = vec![LyricWord { start_ms: 1000, end_ms: 1500, start: 0, end: 5 }, LyricWord { start_ms: 1500, end_ms: 2000, start: 6, end: 11 }];
    let lines = vec![
        LyricLine { start_ms: 1000, end_ms: 2000, text: "Hello world".into(), words, ..Default::default() },
        LyricLine { start_ms: 3000, end_ms: 4000, text: "Second line".into(), ..Default::default() },
    ];
    let pick = nori_core::race::LyricsPick { lyrics: Lyrics { synced: true, word_timed: true, lines, ..Default::default() }, origin: nori_core::lyrics_sources::LyricsOrigin::Server };
    a.handle(Msg::Lyrics { song: "s1".into(), pick });
    let l = a.lyrics.as_ref().unwrap();
    l.advance(1250, true, true);
    let s = draw(&mut a, 160, 30);
    assert!(s.contains("Hello world") && s.contains("Second line"), "{s}");
    dump("lyrics", &s);
    // Enter on a line seeks to it.
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Seek(3000)));
}

#[test]
fn equalizer_screen() {
    let mut a = app();
    a.prefs.eq_enabled = true;
    a.prefs.eq_graphic[2] = 12.0;
    a.prefs.eq_graphic[5] = -6.0;
    a.go(View::Equalizer);
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Tuning(_))), "opening it leaves the output as it is");
    let s = draw(&mut a, 160, 44);
    assert!(s.contains("┃") && s.contains("━●━") && s.contains("1k") && s.contains("Presets ▾") && s.contains("Equalizer on") && s.contains("Limiter"), "{s}");
    dump("equalizer", &s);
    // ← and → walk the chips and the faders and change nothing; redrawing asks nothing either.
    let before = a.cmds.len();
    for _ in 0..8 {
        key(&mut a, KeyCode::Right);
        draw(&mut a, 160, 44);
    }
    key(&mut a, KeyCode::Left);
    assert_eq!(a.cmds.len(), before, "{:?}", &a.cmds[before..]);
    // ↑ on a fader raises its band half a decibel.
    let rows = crate::settings_view::eq_rows(&a.prefs);
    let first = rows.iter().position(|r| matches!(r, crate::settings_view::EqRow::Slider(0))).unwrap();
    while a.eq_sel.at > first {
        key(&mut a, KeyCode::Left);
    }
    while a.eq_sel.at < first {
        key(&mut a, KeyCode::Right);
    }
    key(&mut a, KeyCode::Up);
    assert_eq!(a.cmds.last(), Some(&Cmd::Graphic(0, 0.5)));
    // A click at the top of a fader's track puts it at the top of the range.
    draw(&mut a, 160, 44);
    let fader = hit_rect(&a, Hit::Row(ListRef::Eq, first + 3));
    let top = a.eq_track.0;
    click(&mut a, fader.x + 1, top);
    assert!(matches!(a.cmds.last(), Some(Cmd::Graphic(3, db)) if *db == nori_core::settings::EQ_RANGES.gain.max), "{:?}", a.cmds.last());
    // A click on a switch switches it.
    let on = hit_rect(&a, Hit::Row(ListRef::Eq, 0));
    click(&mut a, on.x + 2, on.y + 1);
    assert_eq!(a.cmds.last(), Some(&Cmd::Setting("eq".into(), "false".into())));
    // The first sound really changed there asks for the shallow buffer, once.
    assert!(a.sound_edited());
    assert!(!a.sound_edited());
    a.go(View::Home);
    assert!(a.cmds.contains(&Cmd::Tuning(false)));
    // Left without changing anything: nothing to ask back.
    a.cmds.clear();
    a.go(View::Equalizer);
    a.go(View::Home);
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Tuning(_))), "{:?}", a.cmds);
    // With the equalizer off its page's limiter and balance are still heard: the same.
    a.prefs.eq_enabled = false;
    a.go(View::Equalizer);
    assert!(a.sound_edited());
    a.go(View::Home);
    assert!(a.cmds.contains(&Cmd::Tuning(false)));
    // The parametric one draws its own bands, and d takes the one chosen out.
    let mut p = StoredPrefs { eq_enabled: true, eq_mode: nori_core::settings::EqMode::Parametric, ..StoredPrefs::default() };
    p.eq_bands = nori_core::settings::graphic();
    a.prefs_changed(p);
    a.go(View::Equalizer);
    let s = draw(&mut a, 160, 44);
    assert!(s.contains("Add a band") && s.contains("Parametric"), "{s}");
    let rows = crate::settings_view::eq_rows(&a.prefs);
    a.eq_sel.at = rows.iter().position(|r| matches!(r, crate::settings_view::EqRow::Band(1))).unwrap();
    key(&mut a, KeyCode::Char('d'));
    assert_eq!(a.cmds.last(), Some(&Cmd::Sound(crate::app::SoundToolCmd::RemoveBand(1))));
}

#[test]
fn seeking() {
    let mut a = app();
    a.go(View::Songs);
    a.song = Some(song("1", "One", 200));
    let now = Instant::now();
    a.now = crate::app::Now { state: State::Playing, position_ms: 10_000, at: now, ..Default::default() };
    // A click in the middle of the seek bar.
    draw(&mut a, 160, 30);
    let bar = hit_rect(&a, Hit::Seek);
    click(&mut a, bar.x + bar.width / 2, bar.y);
    assert!(matches!(a.cmds.last(), Some(Cmd::Seek(ms)) if (ms - 100_000).abs() < 3_000));
    assert!((a.now.position(Instant::now()) - 100_000).abs() < 3_000, "shown at once: {}", a.now.position(Instant::now()));
    a.follow_now(crate::app::Now { state: State::Playing, position_ms: 120_000, at: Instant::now(), ..Default::default() });
    assert!((a.now.position(Instant::now()) - 120_000).abs() < 500);

    // Seek bounds.
    let mut a = app();
    // On a list ← and → seek (on the cards of Home and Albums they move: , and . seek there).
    a.go(View::Songs);
    a.cmds.clear();
    key(&mut a, KeyCode::Right);
    assert!(a.cmds.is_empty(), "nothing playing, nothing to seek");
    a.song = Some(song("1", "One", 200));
    a.now.state = State::Paused;
    a.now.position_ms = 3_000;
    key(&mut a, KeyCode::Left);
    assert_eq!(a.cmds.last(), Some(&Cmd::Seek(0)));
    a.handle(Msg::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)));
    assert_eq!(a.cmds.last(), Some(&Cmd::Seek(30_000)));
    a.go(View::Home);
    key(&mut a, KeyCode::Char('.'));
    assert!(matches!(a.cmds.last(), Some(Cmd::Seek(_))), ". seeks over the cards too");
}

/// Signed out of a provider whose account would carry likes, the seal says where to sign in.
#[test]
fn a_signed_out_like_says_where_to_sign_in() {
    let mut a = app();
    a.rules = nori_core::providers::rules_of(nori_core::settings::SavedServer { kind: nori_core::settings::ServerKind::YouTubeMusic, ..Default::default() }, false);
    a.go(View::Songs);
    a.handle(Msg::Data(Req::Songs { offset: 0 }, Ok(Data::Songs(vec![song("yt-1", "One", 200)], false))));
    key(&mut a, KeyCode::Char('f'));
    assert!(matches!(a.cmds.last(), Some(Cmd::OfferSignIn)));
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Star(..))));
}

fn account(user: &str, ended: bool) -> Overlay {
    Overlay::Account { kind: nori_core::settings::ServerKind::YouTubeMusic, user: user.into(), ended, pasting: false, text: String::new(), error: None, busy: false }
}

/// The account sheet signs in with the provider's page, or with cookies pasted; an ended session's can
/// be told not to come back; a signed-in one signs out.
#[test]
fn the_account_sheet_signs_in_and_out() {
    let mut a = app();
    a.overlay = Some(account("", true));
    let s = draw(&mut a, 100, 30);
    assert!(s.contains("Sign in to YouTube Music") && s.contains("YouTube signed you out") && s.contains("don't show again"), "{s}");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.cmds.last(), Some(Cmd::SignInWindow)) && matches!(a.overlay, Some(Overlay::Account { busy: true, .. })));
    key(&mut a, KeyCode::Char('p'));
    assert!(matches!(a.overlay, Some(Overlay::Account { pasting: false, .. })), "no keys while the window is open");
    a.handle(Msg::SignedIn(nori_core::providers::SignInResult::Refused));
    assert!(draw(&mut a, 100, 30).contains("didn't accept"));

    key(&mut a, KeyCode::Char('p'));
    a.handle(Msg::Paste("SID=x; SAPISID=y".into()));
    let s = draw(&mut a, 100, 30);
    assert!(s.contains("paste it here") && !s.contains("SAPISID"), "pasted cookies stay hidden:\n{s}");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.cmds.last(), Some(Cmd::SignIn(c)) if c == "SID=x; SAPISID=y"));
    a.handle(Msg::SignedIn(nori_core::providers::SignInResult::SignedIn { name: "Ann".into() }));
    assert!(a.overlay.is_none());

    a.overlay = Some(account("", true));
    key(&mut a, KeyCode::Char('d'));
    assert!(matches!(a.cmds.last(), Some(Cmd::StopReminding)) && a.overlay.is_none());

    a.overlay = Some(account("Ann", false));
    assert!(draw(&mut a, 100, 30).contains("Signed in as Ann"));
    key(&mut a, KeyCode::Char('d'));
    assert!(a.overlay.is_some(), "only an ended session's sheet can be quieted");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.cmds.last(), Some(Cmd::SignOut)) && a.overlay.is_none());
}

#[test]
fn star_marks_from_the_core_redraw_the_seals() {
    let mut a = app();
    a.go(View::Songs);
    a.handle(Msg::Data(Req::Songs { offset: 0 }, Ok(Data::Songs(vec![song("1", "One", 200), Song { starred: true, ..song("2", "Two", 200) }], false))));
    a.heard(Some(song("1", "One", 200)));
    let s = draw(&mut a, 170, 40);
    let line = |s: &str, name: &str| s.lines().find(|l| l.contains(name)).unwrap_or_else(|| panic!("{name} missing:\n{s}")).to_string();
    assert!(!line(&s, "One").contains('好') && line(&s, "Two").contains('好'), "{s}");
    assert!(!seal_filled(&a, crate::app::Button::StarSong), "the player's seal is quiet:\n{s}");
    // Another device stars the song playing and unstars the other.
    let mut marks = nori_core::stars::StarMarks::default();
    marks.mark(nori_core::client::Starrable::Song, "1".into(), true);
    marks.mark(nori_core::client::Starrable::Song, "2".into(), false);
    a.handle(Msg::Starred(marks));
    let s = draw(&mut a, 170, 40);
    assert!(line(&s, "One").contains('好') && !line(&s, "Two").contains('好'), "the row:\n{s}");
    assert!(seal_filled(&a, crate::app::Button::StarSong), "and the player's seal, filled:\n{s}");
}

#[test]
fn buttons() {
    let mut a = app();
    a.go(View::Albums);
    a.open_album("al-3".into());
    let songs = vec![song("1", "One", 200), song("2", "Two", 200)];
    a.handle(Msg::Data(Req::Album("al-3".into()), Ok(Data::Album(Box::new(nori_core::AlbumDetail::new(album("al-3", "Three"), songs, vec![]))))));
    a.heard(Some(song("1", "One", 200)));
    let s = draw(&mut a, 170, 40);
    assert!(s.contains("好 Favorite") && s.contains("↓ Download"), "{s}");
    a.cmds.clear();
    let star = hit_rect(&a, Hit::Button(crate::app::Button::Star));
    click(&mut a, star.x + 1, star.y);
    assert_eq!(a.cmds.last(), Some(&Cmd::Star(nori_core::client::Starrable::Album, "al-3".into(), true)));
    draw(&mut a, 170, 40);
    assert!(seal_filled(&a, crate::app::Button::Star), "shown at once");
    let down = hit_rect(&a, Hit::Button(crate::app::Button::Download));
    click(&mut a, down.x + 1, down.y);
    assert!(matches!(a.cmds.last(), Some(Cmd::DownloadFetch(crate::backend::Fetch::Album(id))) if id == "al-3"));
    // The seal by the song in the player.
    let seal = hit_rect(&a, Hit::Button(crate::app::Button::StarSong));
    click(&mut a, seal.x, seal.y);
    assert_eq!(a.cmds.last(), Some(&Cmd::Star(nori_core::client::Starrable::Song, "1".into(), true)));
    draw(&mut a, 170, 40);
    assert!(seal_filled(&a, crate::app::Button::Star) && seal_filled(&a, crate::app::Button::StarSong), "the album's and the player's seal fill at once");

    // Play button follows state.
    let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
    let mut frame = |a: &mut App| {
        t.draw(|f| crate::ui::draw(f, a, None)).unwrap();
        a.dirty = false;
        let buf = t.backend().buffer();
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n").collect::<String>()
    };
    let mut a = app();
    a.heard(Some(song("s1", "First", 200)));
    a.now.state = State::Paused;
    assert_eq!(button(&frame(&mut a)), "play");
    // Space asks the engine, and is drawn.
    key(&mut a, KeyCode::Char(' '));
    assert!(a.cmds.contains(&Cmd::Toggle), "{:?}", a.cmds);
    assert!(a.dirty);
    // The engine's answer alone, with no key pressed, redraws the button.
    a.handle(Msg::Engine(nori_engine::Event::State(State::Playing)));
    assert!(a.dirty, "an engine event is drawn");
    assert_eq!(button(&frame(&mut a)), "pause");
    a.handle(Msg::Engine(nori_engine::Event::State(State::Paused)));
    assert!(a.dirty);
    assert_eq!(button(&frame(&mut a)), "play");
}

#[test]
fn covers_sent_once() {
    use ratatui_image::picker::Picker;
    let mut a = app();
    a.go(View::Albums);
    let albums: Vec<Album> = (0..60).map(|i| Album { cover_art: Some(format!("c{i}")), ..album(&format!("al-{i}"), &format!("Album {i}")) }).collect();
    a.handle(Msg::Data(Req::Albums { offset: 0 }, Ok(Data::Albums(albums))));
    let mut art = crate::art::Art::new(Picker::halfblocks());
    a.cmds.clear();
    let s = draw_with(&mut a, 170, 40, Some(&mut art));
    dump("albums-covers", &s);
    let asked: Vec<&Cmd> = a.cmds.iter().filter(|c| matches!(c, Cmd::Thumb(_))).collect();
    assert!(!asked.is_empty() && asked.len() < 60, "only the covers on screen: {}", asked.len());
    assert!(a.cmds.contains(&Cmd::Thumb("c0".into())));
    // Asked once: the next frame asks nothing again.
    a.cmds.clear();
    draw_with(&mut a, 170, 40, Some(&mut art));
    assert!(!a.cmds.iter().any(|c| matches!(c, Cmd::Thumb(_))));
    // Off in the settings: plain cards, nothing asked.
    a.card_covers = false;
    a.thumbs_asked.clear();
    let s = draw_with(&mut a, 170, 40, Some(&mut art));
    assert!(s.contains("╭") && !a.cmds.iter().any(|c| matches!(c, Cmd::Thumb(_))), "{s}");

    // Cover sent once per change.
    use ratatui_image::picker::ProtocolType;
    let image = |v: u8| std::sync::Arc::new(nori_covers::memory::Image { width: 64, height: 64, pixels: vec![v; 64 * 64 * 4].into_boxed_slice() });
    for protocol in [ProtocolType::Sixel, ProtocolType::Kitty, ProtocolType::Iterm2] {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol);
        let mut art = crate::art::Art::new(picker);
        let mut t = Terminal::new(TestBackend::new(140, 40)).unwrap();
        let mut a = app();
        a.heard(Some(Song { cover_art: Some("c1".into()), ..song("s1", "First", 200) }));
        assert!(a.cmds.contains(&Cmd::Cover { art: "c1".into(), colours: true }), "the heard song's cover is asked for");
        // What the terminal is sent in a frame: ratatui's diff, as written to it.
        fn sent(t: &mut Terminal<TestBackend>, a: &mut App, art: &mut crate::art::Art) -> usize {
            let before = t.backend().buffer().clone();
            t.draw(|f| crate::ui::draw(f, a, Some(art))).unwrap();
            let after = t.backend().buffer();
            before.diff(after).into_iter().filter(|(_, _, c)| c.symbol().starts_with('\x1b')).count()
        }
        art.put("c1".into(), &image(100));
        assert!(sent(&mut t, &mut a, &mut art) > 0, "{protocol:?}: the first cover is sent");
        // (kitty's next frame swaps the transmission for a bare placement: one cell, once)
        sent(&mut t, &mut a, &mut art);
        assert_eq!(sent(&mut t, &mut a, &mut art), 0, "{protocol:?}: and not again while it stays");
        a.heard(Some(Song { cover_art: Some("c2".into()), ..song("s2", "Second", 200) }));
        art.put("c2".into(), &image(200));
        assert!(sent(&mut t, &mut a, &mut art) > 0, "{protocol:?}: the next song's cover is sent");
        // A pane back from another tmux window: every picture made again and the whole screen written
        // (term::repaint: a blank frame, then the next draw writes everything).
        art.resend();
        t.draw(|_| {}).unwrap();
        assert!(sent(&mut t, &mut a, &mut art) > 0, "{protocol:?}: sent again after resend");
    }
}

/// The player bar's play/pause button, as drawn (the controls' middle symbol).
fn button(s: &str) -> &'static str {
    let bar = s.lines().rev().find(|l| l.contains('⏮')).expect("the controls are drawn");
    if bar.contains('⏸') {
        "pause"
    } else if bar.contains('▶') {
        "play"
    } else {
        panic!("no play or pause button: {bar}")
    }
}

#[test]
fn queue_origin_is_page() {
    use nori_core::{OriginKind, PageOrigin, Playlist, PlaylistDetail};
    let mut a = app();
    a.go(View::Albums);
    a.open_album("al-3".into());
    let songs = vec![song("1", "One", 200), song("2", "Two", 200)];
    a.handle(Msg::Data(Req::Album("al-3".into()), Ok(Data::Album(Box::new(nori_core::AlbumDetail::new(album("al-3", "Three"), songs.clone(), vec![]))))));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("ALBUM") && s.contains("Three") && s.contains("▶ Play") && s.contains("⤮ Shuffle"), "{s}");
    dump("album", &s);
    // A row tapped plays the album's songs from it, as the album's own queue; so does the page's Play.
    a.cmds.clear();
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Char('x'));
    let from = |c: &Cmd| match c {
        Cmd::Play { from, start, .. } => Some((from.clone(), *start)),
        _ => None,
    };
    let album_page = Some(PageOrigin::new(OriginKind::Album, "al-3"));
    assert_eq!(a.cmds.iter().filter_map(from).collect::<Vec<_>>(), [(album_page.clone(), 1), (album_page, 0)]);
    // A playlist's page is its own, not the album's its songs come from.
    key(&mut a, KeyCode::Esc);
    a.open_playlist("pl-1".into());
    let pl = PlaylistDetail::new(Playlist { id: "pl-1".into(), ..Default::default() }, songs);
    a.handle(Msg::Data(Req::Playlist("pl-1".into()), Ok(Data::Playlist(Box::new(pl)))));
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.iter().filter_map(from).collect::<Vec<_>>(), [(Some(PageOrigin::new(OriginKind::Playlist, "pl-1")), 0)]);
}

/// An artist with no albums (a YouTube Music profile) lists its songs; Enter plays them from the row,
/// as the artist's queue.
#[test]
fn an_artist_without_albums_lists_its_songs() {
    use nori_core::{Artist, ArtistDetail, OriginKind, PageOrigin};
    let mut a = app();
    a.open_artist("ar-1".into());
    let songs = vec![song("v1", "First Video", 200), song("v2", "Second Video", 300)];
    let d = ArtistDetail::new(Artist { id: "ar-1".into(), name: "Studio".into(), ..Default::default() }, vec![], songs);
    a.handle(Msg::Data(Req::Artist("ar-1".into()), Ok(Data::Artist(Box::new(d)))));
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("2 songs") && s.contains("First Video") && s.contains("Second Video"), "{s}");
    a.cmds.clear();
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    let played: Vec<_> = a.cmds.iter().filter_map(|c| match c {
        Cmd::Play { songs, start, from, .. } => Some((songs.len(), *start, from.clone())),
        _ => None,
    }).collect();
    assert_eq!(played, [(2, 1, Some(PageOrigin::new(OriginKind::Artist, "ar-1")))]);
}

#[test]
fn search_results_list() {
    let mut a = app();
    key(&mut a, KeyCode::Char('/'));
    chars(&mut a, "one");
    let found = nori_core::SearchResult {
        songs: vec![song("s1", "One", 200)],
        albums: vec![album("al-1", "One Album")],
        artists: vec![nori_core::Artist { id: "ar-1".into(), name: "The Ones".into(), ..Default::default() }],
        playlists: vec![nori_core::Playlist { id: "yt-VLone".into(), name: "One Mix".into(), comment: Some("Ann • 17 views".into()), ..Default::default() }],
    };
    a.search.view = Some(nori_core::search::SearchView { query: "one".into(), shown: Some(found), from_server: true, ..Default::default() });
    key(&mut a, KeyCode::Enter);
    let s = draw(&mut a, 160, 40);
    assert!(s.contains("Songs") && s.contains("One Album") && s.contains("The Ones") && s.contains("From the server"), "{s}");
    assert!(s.contains("One Mix") && s.contains("Ann • 17 views"), "an uncounted playlist shows its own line: {s}");
    dump("search", &s);
    assert!(matches!(a.selected(), Some(crate::app::Item::Song(..))), "the first song, not its title");
    key(&mut a, KeyCode::Down);
    assert!(matches!(a.selected(), Some(crate::app::Item::Album(_))), "over the albums' title");
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert!(a.cmds.contains(&Cmd::Load(Req::Album("al-1".into()))));
}

#[test]
fn idle() {
    let mut a = app();
    let now = Instant::now();
    assert_eq!(a.next_wake(now), None, "idle: no timer at all");
    a.now.state = State::Paused;
    assert_eq!(a.next_wake(now), None, "paused: no timer at all");
    a.now = crate::app::Now { state: State::Playing, position_ms: 12_300, at: now, speed: 1.0, mixing: false, buffering: false };
    let wake = a.next_wake(now).unwrap();
    let ms = wake.duration_since(now).as_millis();
    assert!((690..=710).contains(&ms), "the next whole second of the song: {ms}");
    a.now.buffering = true;
    assert_eq!(a.next_wake(now), None, "waiting for the network: the clock stands still");
    a.now.buffering = false;
    a.lyrics_wake = Some(now + Duration::from_millis(50));
    a.handle(Msg::Focus(false));
    assert_eq!(a.next_wake(now), None, "out of sight: neither the clock nor the lyrics wake");
    a.dirty = false;
    a.handle(Msg::Focus(true));
    assert!(a.dirty && a.next_wake(now).is_some(), "back in sight: drawn at once, the clock running again");

    // Pause freezes clock.
    let mut a = app();
    let start = Instant::now() - Duration::from_secs(5);
    a.now = crate::app::Now { state: State::Playing, position_ms: 10_000, at: start, ..Default::default() };
    a.handle(Msg::Engine(nori_engine::Event::State(State::Paused)));
    let at = a.now.position(Instant::now());
    assert!((14_900..16_000).contains(&at), "stopped about 15 s in, not back at 10 s: {at}");
    assert_eq!(a.now.position(Instant::now() + Duration::from_secs(60)), at, "paused: the clock stands, a minute on too");
}

#[test]
fn clock_wakes_only_where_drawn() {
    let mut a = app();
    let now = Instant::now();
    a.now = crate::app::Now { state: State::Playing, position_ms: 12_300, at: now, speed: 1.0, mixing: false, buffering: false };
    a.song = Some(Song { id: "s".into(), title: "Song".into(), duration: 200, ..Default::default() });
    draw(&mut a, 60, 14);
    assert_eq!(a.next_wake(now), None, "a bar without a clock needs no tick");
    draw(&mut a, 100, 40);
    assert!(a.next_wake(now).is_some(), "the bar's clock ticks each second");
    draw(&mut a, 60, 14);
    assert_eq!(a.next_wake(now), None, "a resize back drops the tick");
}

#[test]
fn terminal_replies() {
    use crate::term::sixel_feature;
    // Ghostty (no sixel) under tmux: tmux claims sixel but would show a placeholder.
    assert!(!sixel_feature("bpaste,ccolour,clipboard,cstyle,focus,RGB,title"));
    assert!(!sixel_feature(""));
    assert!(sixel_feature("256,bpaste,ccolour,clipboard,cstyle,extkeys,focus,mouse,rectfill,RGB,sixel,strikethrough,title"));

    // Late query reply swallowed.
    // A late kitty graphics reply: alt+_, its letters, alt+\\. Only the key after it counts.
    let mut r = crate::runner::Replies::default();
    let k = |c: char, m: KeyModifiers| KeyEvent::new(KeyCode::Char(c), m);
    let mut taken = Vec::new();
    let mut keys = vec![k('_', KeyModifiers::ALT)];
    keys.extend("Gi=31;OK".chars().map(|c| k(c, KeyModifiers::NONE)));
    keys.push(k('\\', KeyModifiers::ALT));
    keys.push(k('n', KeyModifiers::NONE));
    for key in keys {
        if !r.swallows(&key) {
            taken.push(key.code);
        }
    }
    assert_eq!(taken, [KeyCode::Char('n')]);

    // Graphics cell does not hide frame.
    use ratatui_image::picker::{Picker, ProtocolType};
    for protocol in [ProtocolType::Sixel, ProtocolType::Kitty, ProtocolType::Iterm2, ProtocolType::Halfblocks] {
        let mut a = app();
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol);
        let mut art = crate::art::Art::new(picker);
        let px = vec![120u8; 64 * 64 * 4].into_boxed_slice();
        art.put("c1".into(), &std::sync::Arc::new(nori_covers::memory::Image { width: 64, height: 64, pixels: px }));
        a.heard(Some(Song { cover_art: Some("c1".into()), ..song("s1", "First", 200) }));
        a.queue = Some(queue_of(5, 0));
        // The TestBackend is written through ratatui's diff, as a terminal is: whatever the diff skips is
        // missing from it.
        let s = draw_with(&mut a, 140, 40, Some(&mut art));
        assert!(s.contains("First") && s.contains("3:20") && s.contains("space play"), "{protocol:?} hid the frame:\n{s}");
    }
}

fn device(id: &str, name: &str, kind: nori_core::remote::wire::DeviceKind, playing: Option<&str>) -> nori_core::remote::RemoteDevice {
    use nori_core::remote::wire::{DeviceState, Entry};
    let state = playing.map(|title| DeviceState { playing: true, index: Some(0), entries: vec![Entry { title: title.into(), artist: "Artist".into(), ..Default::default() }], ..Default::default() });
    nori_core::remote::RemoteDevice { id: id.into(), name: name.into(), kind, state, age_ms: 0, nearby: true, refused: None }
}

/// A hosted jam with two listeners and two requests, the first a provider's song.
fn jam() -> nori_core::remote::JamView {
    use nori_core::remote::wire::{Entry, JamMember, Pending, Role};
    let member = |id: &str, name: &str, role| JamMember { id: id.into(), name: name.into(), role };
    let ask = |request, from: &str, title: &str, provider| Pending { request, from: from.to_lowercase(), from_name: from.into(), song: Entry { title: title.into(), ..Default::default() }, provider };
    nori_core::remote::JamView {
        hosting: true,
        remote_songs: false,
        link: Some("http://octo:5274/nori/jam#s=http%3A%2F%2Focto%3A5274&k=c1a952e53165572a6349ca04fa2e3b8a".into()),
        you: "me".into(),
        members: vec![member("me", "Mac", Role::Host), member("gus", "gus", Role::Guest), member("dee", "Dee", Role::Admin)],
        pending: vec![ask(7, "Gus", "Wish", true), ask(12, "Dee", "Blue", false)],
        queue: None,
        age_ms: 0,
        refused: None,
        along: true,
        listening: nori_core::remote::Listening::Watching,
    }
}

#[test]
fn room_settings_replace_the_invite() {
    let mut a = app();
    a.devices.jam = Some(jam());
    a.overlay = Some(Overlay::Invite { link: a.devices.jam.as_ref().unwrap().link.clone().unwrap() });
    key(&mut a, KeyCode::Char('s'));
    let screen = draw(&mut a, 100, 30);
    assert!(screen.contains("Allow remote songs: off"));
    key(&mut a, KeyCode::Char('l'));
    assert!(matches!(a.cmds.last(), Some(Cmd::JamAlong(false))));
    key(&mut a, KeyCode::Char('r'));
    assert!(matches!(a.cmds.last(), Some(Cmd::JamRemoteSongs(true))));
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.overlay, Some(Overlay::Invite { .. })));
}

#[test]
fn devices_panel() {
    use nori_core::remote::wire::DeviceKind;
    let mut a = app();
    key(&mut a, KeyCode::Char('C'));
    assert!(!a.full, "nothing to show while remote control and jams are off");
    assert!(a.note.as_ref().is_some_and(|n| n.0 == crate::text::REMOTE_OFF));
    key(&mut a, KeyCode::Char('F'));
    assert!(!draw(&mut a, 160, 30).contains("⇄ Devices"), "no tab either");
    key(&mut a, KeyCode::Esc);

    a.devices.on = true;
    a.devices.jams = true;
    a.devices.list = vec![device("desk", "Desk", DeviceKind::Desktop, Some("Wish")), device("pixel", "Pixel", DeviceKind::Phone, None)];
    key(&mut a, KeyCode::Char('C'));
    assert_eq!((a.full, a.panel, a.focus), (true, Panel::Devices, Focus::Panel));
    let s = draw(&mut a, 160, 30);
    dump("devices", &s);
    let row = |name: &str| s.lines().find(|l| l.contains(name)).unwrap_or_else(|| panic!("{name} missing:\n{s}")).to_string();
    assert!(row("This computer").contains("✓ This computer"), "this computer first, ticked while it plays:\n{s}");
    let under = |name: &str| s.lines().skip_while(|l| !l.contains(name)).nth(1).unwrap_or_default().to_string();
    assert!(row("Desk").contains("computer") && under("Desk").contains("  Wish · Artist"), "what each plays, under it:\n{s}");
    assert!(row("Pixel").contains("phone") && under("Pixel").contains("  Not playing"), "{s}");
    assert!(s.contains("Start a jam") && s.contains("Devices") && s.contains('⇄'), "{s}");

    // Enter moves the music there; the keys step device by device.
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Pick(Some("desk".into()))));
    a.devices.active = Some(("desk".into(), "Desk".into()));
    let s = draw(&mut a, 160, 30);
    assert!(s.lines().any(|l| l.contains("✓ Desk")) && !s.contains("✓ This computer"), "the tick follows the music:\n{s}");
    key(&mut a, KeyCode::Char('g'));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Pick(None)), "back here");
    key(&mut a, KeyCode::Char('G'));
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.overlay, Some(Overlay::Join { .. })), "the last row joins someone's jam");
    key(&mut a, KeyCode::Esc);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::JamStart));

    // Reopened, it selects the device playing.
    key(&mut a, KeyCode::Char('C'));
    key(&mut a, KeyCode::Char('C'));
    assert_eq!(a.devices.sel.at, 1);
    // Clicking a device twice moves the music there.
    draw(&mut a, 160, 30);
    let pixel = a.devices.rows().iter().position(|r| *r == crate::app::DeviceRow::Playing(1)).unwrap();
    let r = hit_rect(&a, Hit::Row(ListRef::Devices, pixel));
    click(&mut a, r.x + 2, r.y);
    click(&mut a, r.x + 2, r.y);
    assert_eq!(a.cmds.last(), Some(&Cmd::Pick(Some("pixel".into()))));
}

#[test]
fn another_device_playing_shows_in_the_player_bar() {
    let mut a = app();
    a.devices.on = true;
    a.devices.active = Some(("desk".into(), "Desk".into()));
    a.heard(Some(song("s1", "First", 200)));
    a.now = crate::app::Now { state: State::Playing, position_ms: 61_000, at: Instant::now(), ..Default::default() };
    a.volume = 0.4;
    // The device shows the song unstarred, though this session marked it.
    a.marks.mark(nori_core::client::Starrable::Song, "s1".into(), true);
    a.devices.hearts.insert("s1".into(), false);
    let s = draw(&mut a, 160, 30);
    dump("playing-on", &s);
    assert!(s.contains("⇄ Playing on Desk") && !seal_filled(&a, crate::app::Button::StarSong) && s.contains("First") && s.contains("1:01"), "{s}");
    let volume = s.lines().find_map(|l| l.split_once("🔉")).expect("the volume").1.to_string();
    assert_eq!(volume.matches('━').count(), 5, "40 % of 12 cells:{volume}");
    // The heart goes to the device, and fills at once.
    let heart = hit_rect(&a, Hit::Button(crate::app::Button::StarSong));
    click(&mut a, heart.x, heart.y);
    assert_eq!(a.cmds.last(), Some(&Cmd::Star(nori_core::client::Starrable::Song, "s1".into(), true)));
    draw(&mut a, 160, 30);
    assert!(seal_filled(&a, crate::app::Button::StarSong));
    // This computer's volume, set from afar, does not show over the device's.
    a.handle(Msg::Volume(0.9));
    assert_eq!(a.volume, 0.4);
    // The line opens the devices.
    let line = hit_rect(&a, Hit::Button(crate::app::Button::Panel(Panel::Devices)));
    click(&mut a, line.x + 3, line.y);
    assert_eq!((a.full, a.panel), (true, Panel::Devices));
}

#[test]
fn jam_header_and_requests_in_the_queue() {
    let mut a = app();
    a.devices.on = true;
    a.devices.jams = true;
    a.devices.jam = Some(jam());
    a.devices.added.insert("s2".into(), "Gus".into());
    a.queue = Some(queue_of(5, 1));
    key(&mut a, KeyCode::Char('Q'));
    let s = draw(&mut a, 160, 30);
    dump("jam-queue", &s);
    let row = |text: &str| s.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text} missing:\n{s}")).to_string();
    assert!(row("Jam · 2 listening").contains("invite") && row("gus, Dee (admin)").contains("  gus"), "{s}");
    assert!(row("Wish · asked by Gus").contains("? Wish") && row("Blue · asked by Dee").contains("? Blue"), "{s}");
    assert!(!s.contains("⏎ add · d no"), "the keys only on the request chosen:\n{s}");
    // The note is whole under the provider's song and wraps in a narrower panel.
    let note = |s: &str| -> Vec<String> { s.lines().map(|l| l.split('│').nth(1).unwrap_or("").trim().to_string()).collect() };
    let panel = note(&s);
    let at = panel.iter().position(|l| l.starts_with('☁')).expect("the note");
    assert_eq!(panel[at], format!("☁ {}", crate::text::JAM_DOWNLOADS), "{s}");
    assert!(panel[at - 1].contains("Wish") && s.matches('☁').count() == 1, "only under the provider's song:\n{s}");
    let narrow = draw(&mut a, 50, 40);
    let panel = note(&narrow);
    let at = panel.iter().position(|l| l.starts_with('☁')).unwrap_or_else(|| panic!("the note:\n{narrow}"));
    assert_eq!(format!("{} {}", panel[at], panel[at + 1]), format!("☁ {}", crate::text::JAM_DOWNLOADS), "wrapped:\n{narrow}");
    assert!(row("Song 2").contains("Song 2 · Gus  Artist") && !row("Song 3").contains("· Gus"), "who asked for it:\n{s}");
    assert!(s.contains("End the jam") && s.contains("◉ Jam · 2 listening"), "{s}");

    // Opens on the song playing, below the jam's rows.
    assert_eq!(a.queue_rows()[a.queue_sel.at], crate::app::QueueRow::Song(1));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Jump(1)));
    let at = |a: &mut App, row| a.queue_sel.at = a.queue_rows().iter().position(|r| *r == row).unwrap();
    at(&mut a, crate::app::QueueRow::Ask(0));
    let s = draw(&mut a, 160, 30);
    assert!(s.lines().any(|l| l.contains("Wish · asked by Gus") && l.contains("⏎ add · d no")) && s.matches("⏎ add").count() == 1, "{s}");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::JamDecide(7, true)));
    at(&mut a, crate::app::QueueRow::Downloads(0, 0));
    key(&mut a, KeyCode::Char('d'));
    assert_eq!(a.cmds.last(), Some(&Cmd::JamDecide(7, false)), "its note is the request's");
    at(&mut a, crate::app::QueueRow::Ask(1));
    key(&mut a, KeyCode::Char('d'));
    assert_eq!(a.cmds.last(), Some(&Cmd::JamDecide(12, false)));
    assert!(a.taken.is_none(), "a refusal is no song taken out");
    at(&mut a, crate::app::QueueRow::End);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::JamEnd));
    at(&mut a, crate::app::QueueRow::Jam);
    key(&mut a, KeyCode::Enter);
    assert!(matches!(&a.overlay, Some(Overlay::Invite { link }) if link.starts_with("http://octo:5274/nori/jam#")));

    // The player bar says the jam is on.
    a.overlay = None;
    key(&mut a, KeyCode::Esc);
    assert!(draw(&mut a, 160, 30).contains("◉ Jam · 2 listening ─"));
}

#[test]
fn a_jam_starts_around_the_song_picked() {
    let mut a = app();
    a.go(View::Songs);
    let songs = vec![song("1", "One", 200), song("2", "Two", 200)];
    a.handle(Msg::Data(Req::Songs { offset: 0 }, Ok(Data::Songs(songs.clone(), false))));
    key(&mut a, KeyCode::Down);
    a.cmds.clear();
    key(&mut a, KeyCode::Char('i'));
    assert!(a.cmds.is_empty() && a.note.as_ref().is_some_and(|n| n.0 == crate::text::JAMS_OFF), "jams off");
    a.devices = crate::app::Devices { on: true, jams_unsupported: true, ..Default::default() };
    key(&mut a, KeyCode::Char('i'));
    assert!(a.cmds.is_empty() && a.note.as_ref().is_some_and(|n| n.0 == crate::text::JAM_UNSUPPORTED), "no relay");

    a.devices = crate::app::Devices { on: true, jams: true, ..Default::default() };
    key(&mut a, KeyCode::Char('i'));
    let origin = Some(nori_core::PageOrigin::new(nori_core::OriginKind::Songs, ""));
    assert_eq!(a.cmds, [Cmd::Play { songs, start: 1, shuffle: false, from: origin }, Cmd::JamStart]);
    // Opened, it shows its invite over the queue.
    a.devices.jam = Some(jam());
    a.handle(Msg::Jam(Ok(())));
    assert!(matches!(a.overlay, Some(Overlay::Invite { .. })) && a.panel == Panel::Queue);
    // A jam on already: the invite again, nothing replayed.
    a.overlay = None;
    a.cmds.clear();
    key(&mut a, KeyCode::Char('i'));
    assert!(a.cmds.is_empty() && matches!(a.overlay, Some(Overlay::Invite { .. })));
    a.handle(Msg::Jam(Err("HTTP 404".into())));
    assert!(a.note.as_ref().is_some_and(|n| n.1 && n.0.ends_with("(HTTP 404)")));
}

/// The screen as a camera sees it on a dark terminal: a cell's upper and lower halves lit where the code
/// is drawn white, every other cell dark; `px` pixels a module.
fn photographed(buf: &ratatui::buffer::Buffer, px: usize) -> (usize, usize, Vec<u8>) {
    use ratatui::style::Color;
    let (w, h) = (buf.area.width as usize * px, buf.area.height as usize * 2 * px);
    let lit = |c: Color| if c == Color::Rgb(255, 255, 255) { 255 } else { 0 };
    let mut out = vec![0u8; w * h];
    for (y, row) in out.chunks_mut(w).enumerate() {
        for (x, p) in row.iter_mut().enumerate() {
            let cell = &buf[((x / px) as u16, (y / (2 * px)) as u16)];
            let upper = y % (2 * px) < px;
            *p = match cell.symbol() {
                "▀" if upper => lit(cell.fg),
                "▀" => lit(cell.bg),
                _ => 0,
            };
        }
    }
    (w, h, out)
}

#[test]
fn the_invite_code_scans_from_the_screen() {
    let link = jam().link.unwrap();
    let mut a = app();
    a.overlay = Some(Overlay::Invite { link: link.clone() });
    let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
    t.draw(|f| crate::ui::draw(f, &mut a, None)).unwrap();
    let buf = t.backend().buffer().clone();
    let s: String = (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n").collect();
    dump("invite", &s);
    assert!(s.contains(&link) && s.contains("Scan with a phone"), "{s}");
    let (w, h, px) = photographed(&buf, 4);
    let mut image = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| px[y * w + x]);
    let grids = image.detect_grids();
    assert_eq!(grids.len(), 1, "one code on the screen");
    assert_eq!(grids[0].decode().map(|(_, text)| text).ok(), Some(link.clone()));

    // Too small a window: the link alone, and why.
    let s = draw(&mut a, 60, 20);
    assert!(s.contains(crate::text::INVITE_ROOM) && !s.contains('▀'), "{s}");
}

/// A jam Desk hosts and this terminal is a guest in, as Gus: Dee listens too, and each asked for a song.
fn guest_jam() -> nori_core::remote::JamView {
    use nori_core::remote::wire::{Entry, JamMember, Pending, Role};
    let member = |id: &str, name: &str, role| JamMember { id: id.into(), name: name.into(), role };
    let ask = |request, from: &str, id: &str, title: &str| Pending { request, from: from.to_lowercase(), from_name: from.into(), song: Entry { id: id.into(), title: title.into(), ..Default::default() }, provider: true };
    nori_core::remote::JamView {
        hosting: false,
        remote_songs: false,
        link: None,
        you: "gus".into(),
        members: vec![member("desk", "Desk", Role::Host), member("gus", "Gus", Role::Guest), member("dee", "Dee", Role::Guest)],
        pending: vec![ask(1, "Gus", "2", "Two"), ask(2, "Dee", "1", "One")],
        queue: None,
        age_ms: 0,
        refused: None,
        along: true,
        listening: nori_core::remote::Listening::Watching,
    }
}

/// An app open on a jam guest's profile, in `guest_jam`.
fn guest() -> App {
    let mut a = app();
    a.rules = nori_core::browse::profile_rules(true);
    a.devices.jam = Some(guest_jam());
    a
}

#[test]
fn joining_a_jam_with_its_invite_link() {
    let link = "https://octo.example/nori/jam#s=https%3A%2F%2Focto.example&k=c1a952e5";
    let mut a = app();
    key(&mut a, KeyCode::Char('o'));
    chars(&mut a, "hello");
    key(&mut a, KeyCode::Enter);
    let s = draw(&mut a, 120, 30);
    dump("join", &s);
    assert!(s.contains(crate::text::JOIN_HOW) && s.contains("hello▏") && s.contains("That is not a jam invite"), "{s}");
    assert!(a.cmds.is_empty());
    a.handle(Msg::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    a.handle(Msg::Paste(link.into()));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds, [Cmd::JoinJam(link.into())]);
    assert!(draw(&mut a, 120, 30).contains(crate::text::JOINING));
    key(&mut a, KeyCode::Char('x'));
    assert!(matches!(&a.overlay, Some(Overlay::Join { text, .. }) if text == link), "no typing while it joins");
    a.handle(Msg::Joined(Err(crate::text::jam_join_failed(&nori_host::JoinError::Failed(nori_core::transport::NetError::Http { status: 404 })))));
    let s = draw(&mut a, 120, 30);
    assert!(s.contains("Couldn't join the jam (HTTP 404)") && !s.contains(crate::text::JOINING), "{s}");
    key(&mut a, KeyCode::Esc);
    assert!(a.overlay.is_none());

    // The link pasted anywhere offers to join; other text searches.
    a.handle(Msg::Paste(link.into()));
    assert!(matches!(&a.overlay, Some(Overlay::Join { text, .. }) if text == link));
    a.overlay = None;
    a.handle(Msg::Paste("blue".into()));
    assert!(a.overlay.is_none() && a.view == View::Search && a.search.text == "blue");

    // And from the settings, under Other devices.
    a.go(View::Settings);
    let pages = a.settings.pages(&a.prefs.clone()).to_vec();
    let at = SettingsView::lines(&pages).iter().position(|l| matches!(l, Line::Row(Row::Button { title, .. }) if title == crate::text::JAM_JOIN)).expect("the button");
    while a.settings.row.at < at {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.overlay, Some(Overlay::Join { .. })));
}

#[test]
fn a_guest_browses_the_hosts_library_and_asks_for_songs() {
    let mut a = guest();
    a.go(View::Songs);
    assert!(!a.cmds.contains(&Cmd::Load(Req::Playlists)), "the host's playlists are not the guest's");
    let songs = vec![song("1", "One", 200), song("2", "Two", 200), song("3", "Three", 200)];
    a.marks.mark(nori_core::client::Starrable::Song, "3".into(), true);
    a.handle(Msg::Data(Req::Songs { offset: 0 }, Ok(Data::Songs(songs.clone(), true))));
    let s = draw(&mut a, 160, 30);
    dump("guest-songs", &s);
    let row = |text: &str| s.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text} missing:\n{s}")).to_string();
    assert!(row("Two").contains("Asked"), "the song this guest asked for:\n{s}");
    assert!(!row("One ").contains("Asked") && !row("Three").contains('好'), "another's request, and no seals:\n{s}");
    for gone in ["Downloads", "Playlists", "Favorites"] {
        assert!(!s.contains(gone), "{gone} is the account's:\n{s}");
    }
    assert!(s.contains("Equalizer") && s.contains('✱'), "this computer's own, a guest's too:\n{s}");
    assert!(s.contains("◉ Jam · Desk · 2 listening") && !s.contains('⏮') && !s.contains('⤮'), "the jam, and no controls:\n{s}");

    // Picking a song plays it, which the session asks the host for; the account's keys do nothing.
    a.cmds.clear();
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds, [Cmd::Play { songs, start: 0, shuffle: false, from: Some(nori_core::PageOrigin::new(nori_core::OriginKind::Songs, "")) }]);
    a.cmds.clear();
    for k in ['f', 'D', ' ', 'n', 's', 'i', '5'] {
        key(&mut a, KeyCode::Char(k));
    }
    assert!(a.cmds.is_empty() && a.view == View::Songs, "{:?} {:?}", a.cmds, a.view);
    key(&mut a, KeyCode::Char('a'));
    assert_eq!(a.cmds, [Cmd::Enqueue(vec![song("1", "One", 200)], false)]);
}

/// A jam guest's player offers what its role reaches (Spotify's Jam): listening along, play and pause of
/// its own, saying so while it paused here; an admin's skips are the host's too. Shuffle and repeat
/// stay the host's.
#[test]
fn a_guests_controls_go_by_its_role() {
    use nori_core::remote::wire::Role;
    use nori_core::remote::{Controls, JamControls};
    let mut a = guest();
    a.go(View::Songs);
    a.devices.controls = Some(JamControls { controls: Controls::of(Role::Guest, true),
        asks: true,
        playing: false, paused_here: true });
    let s = draw(&mut a, 160, 30);
    dump("guest-paused-here", &s);
    assert!(s.contains(" ▶ ") && !s.contains('⏮') && !s.contains('⤮') && s.contains(crate::text::JAM_PAUSED_HERE), "play only, and paused here:\n{s}");
    a.cmds.clear();
    for k in [' ', 'n', 's'] {
        key(&mut a, KeyCode::Char(k));
    }
    assert_eq!(a.cmds, [Cmd::Toggle], "its play, not a skip");

    a.devices.controls = Some(JamControls { controls: Controls::of(Role::Admin, true),
        asks: false, playing: true, paused_here: false });
    let s = draw(&mut a, 160, 30);
    assert!(s.contains('⏮') && s.contains('⏭') && !s.contains('⤮') && s.contains("◉ Jam · Desk"), "the host's skips:\n{s}");
    a.cmds.clear();
    for k in [' ', 'n', 's'] {
        key(&mut a, KeyCode::Char(k));
    }
    assert_eq!(a.cmds, [Cmd::Toggle, Cmd::Next]);
}

#[test]
fn a_guests_queue_shows_the_jam() {
    let mut a = guest();
    a.devices.added.insert("s2".into(), "Gus".into());
    a.queue = Some(queue_of(4, 1));
    key(&mut a, KeyCode::Char('Q'));
    let s = draw(&mut a, 160, 30);
    dump("guest-queue", &s);
    let row = |text: &str| s.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text} missing:\n{s}")).to_string();
    let order: Vec<usize> = ["◉ Jam · Desk · 2 listening", "Gus, Dee", "♪ Listen here", "You asked for", "? Two · waiting for Desk", "✕ Leave the jam", "Song 2 · Gus"].iter().map(|t| s.find(t).unwrap_or_else(|| panic!("{t} missing:\n{s}"))).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "in this order:\n{s}");
    assert!(!s.contains("One") && !row("◉ Jam").contains("invite") && !s.contains("⏎ add"), "only its own request, nothing to decide:\n{s}");

    // Opens on the song playing, which a guest cannot jump to or take out.
    assert_eq!(a.queue_rows()[a.queue_sel.at], crate::app::QueueRow::Song(1));
    for k in [KeyCode::Enter, KeyCode::Char('d'), KeyCode::Char('J')] {
        key(&mut a, k);
    }
    assert!(a.cmds.is_empty(), "{:?}", a.cmds);
    let at = |a: &mut App, row| a.queue_sel.at = a.queue_rows().iter().position(|r| *r == row).unwrap();
    at(&mut a, crate::app::QueueRow::Ask(0));
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Char('d'));
    assert!(a.cmds.is_empty(), "the host decides");

    // Listen here: asked for, then playing here, or why not.
    at(&mut a, crate::app::QueueRow::Listen);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Listen(true)));
    a.devices.jam.as_mut().unwrap().listening = nori_core::remote::Listening::HostOff;
    let s = draw(&mut a, 160, 30);
    assert!(s.contains("♪ Listen here") && s.contains("⏎ stop") && s.contains("The host doesn't let guests"), "{s}");
    a.devices.jam.as_mut().unwrap().listening = nori_core::remote::Listening::Playing;
    let s = draw(&mut a, 160, 30);
    assert!(s.contains("♪ Playing here") && s.contains("⏎ stop") && !s.contains("The host doesn't"), "{s}");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::Listen(false)));

    // Leave.
    at(&mut a, crate::app::QueueRow::End);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.cmds.last(), Some(&Cmd::LeaveJam));
    // The player bar's jam opens the queue.
    key(&mut a, KeyCode::Esc);
    a.panel = Panel::Lyrics;
    draw(&mut a, 160, 30);
    let line = hit_rect(&a, Hit::Button(crate::app::Button::Panel(Panel::Queue)));
    click(&mut a, line.x + 3, line.y);
    assert_eq!((a.full, a.panel), (true, Panel::Queue));
}
