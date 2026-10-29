//! Drawing: a line of tabs, the page, two lines of the song playing and a line of keys; the full player
//! takes the window with its song and a panel. The page is painted in the season's colors. Every clickable
//! area is recorded in `App::hits` as it is drawn. Lists draw only visible rows.

use std::borrow::Cow;
use std::collections::HashSet;
use std::time::Instant;

use nori_core::client::Starrable;
use nori_core::stars::StarMarks;
use nori_core::Song;
use nori_engine::State;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};
use ratatui::Frame;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use nori_core::browse::AlbumSort;

use crate::app::{App, Button, DeviceRow, Focus, HomeLine, Hit, Item, LibLine, ListRef, Load, Overlay, Page, Panel, Place, QueueRow, SearchRow, Sel, View, LOGIN_FIELDS, PANELS};
use crate::art::{Art, Theme};
use crate::keys::{Scope, BINDINGS};
use crate::settings_view::{self, EqRow, Line as SLine, SettingsView};
use crate::text::clock;

/// `s` cut to `w` columns, with an ellipsis if cut.
pub fn fit(s: &str, w: usize) -> Cow<'_, str> {
    if s.width() <= w {
        return Cow::Borrowed(s);
    }
    if w == 0 {
        return Cow::Borrowed("");
    }
    let mut out = String::with_capacity(w + 3);
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        used += cw;
        out.push(c);
    }
    out.push('…');
    Cow::Owned(out)
}

/// `s` cut or space-padded to exactly `w` columns.
fn pad(s: &str, w: usize) -> String {
    let cut = fit(s, w);
    let used = cut.width();
    let mut out = cut.into_owned();
    out.extend(std::iter::repeat_n(' ', w.saturating_sub(used)));
    out
}

/// `left` and right-aligned `right` in `w` columns, cutting `left` to fit.
fn spread<'a>(left: Vec<Span<'a>>, right: Span<'a>, w: usize) -> Line<'a> {
    let rw = right.content.width();
    let room = w.saturating_sub(rw + 1);
    let mut used = 0;
    let mut spans = Vec::with_capacity(left.len() + 2);
    for s in left {
        let sw = s.content.width();
        if used + sw <= room {
            used += sw;
            spans.push(s);
        } else {
            let cut = fit(&s.content, room - used).into_owned();
            used += cut.width();
            spans.push(Span::styled(cut, s.style));
            break;
        }
    }
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used + rw))));
    spans.push(right);
    Line::from(spans)
}

/// Renders `w` clipped to the frame.
fn put<W: Widget>(f: &mut Frame, w: W, r: Rect) {
    let r = r.intersection(f.area());
    if r.width > 0 && r.height > 0 {
        f.render_widget(w, r);
    }
}

fn text(f: &mut Frame, r: Rect, s: &str, style: Style) {
    put(f, Paragraph::new(Span::styled(fit(s, r.width as usize).into_owned(), style)), Rect { height: r.height.min(1), ..r });
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// A row `n` rows into `r`.
fn row_of(r: Rect, n: u16) -> Rect {
    Rect { y: r.y + n, height: 1, ..r }
}

/// `r` without its first `n` rows.
fn below(r: Rect, n: u16) -> Rect {
    Rect { y: r.y + n.min(r.height), height: r.height.saturating_sub(n), ..r }
}

/// A scrolling list with the selection kept in view and a scrollbar when it overflows.
#[allow(clippy::too_many_arguments)]
fn list<'a>(f: &mut Frame, area: Rect, sel: &mut Sel, len: usize, lref: ListRef, hits: &mut Vec<(Rect, Hit)>, t: &Theme, focused: bool, row: &dyn Fn(usize, usize) -> Line<'a>) {
    hits.push((area, Hit::List(lref)));
    let h = area.height as usize;
    sel.fit(h, len);
    let width = if len > h { area.width.saturating_sub(1) } else { area.width };
    for (n, i) in (sel.top..len.min(sel.top + h)).enumerate() {
        let r = Rect { x: area.x, y: area.y + n as u16, width, height: 1 };
        let mut line = row(i, width as usize);
        if i == sel.at {
            // The raised surface under every span, which keep their colors.
            let st = t.select(focused);
            line.spans.iter_mut().for_each(|s| s.style = s.style.patch(st));
            line = line.style(st);
        }
        put(f, Paragraph::new(line), r);
        hits.push((r, Hit::Row(lref, i)));
    }
    if len > h && area.width > 2 && h > 0 {
        let x = area.x + area.width - 1;
        let thumb = (h * h / len).clamp(1, h);
        let y0 = (sel.top * h) / len.max(1);
        for y in 0..h {
            let lit = y >= y0 && y < y0 + thumb;
            put(f, Paragraph::new(Span::styled(if lit { "┃" } else { "│" }, Style::default().fg(if lit { t.quiet } else { t.hair }))), Rect { x, y: area.y + y as u16, width: 1, height: 1 });
        }
    }
}

fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

/// A titled box on the raised surface over `r`, outlined; returns its inner area.
fn popup(f: &mut Frame, r: Rect, title: Span, t: &Theme) -> Rect {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title).border_style(Style::default().fg(t.line)).style(t.raised());
    let inner = block.inner(r);
    put(f, Clear, r);
    put(f, block, r);
    inner
}

/// A bold page title with a quiet line under it (none when empty); returns the area below plus a blank line.
fn heading(f: &mut Frame, area: Rect, title: &str, caption: &str, t: &Theme) -> Rect {
    if area.height < 3 {
        return area;
    }
    text(f, area, title, bold().fg(t.ink));
    if caption.is_empty() {
        return below(area, 2);
    }
    text(f, row_of(area, 1), caption, t.dim());
    below(area, 3)
}

// ---- songs as a table ----

/// Song table column widths in `w`: number, title, artist, album, time. Album only if asked and room;
/// artist dropped first.
fn columns(w: usize, album: bool) -> [usize; 5] {
    let num = 4;
    let time = 6;
    let rest = w.saturating_sub(num + time);
    if album && rest >= 66 {
        let title = rest * 44 / 100;
        let artist = rest * 28 / 100;
        [num, title, artist, rest - title - artist, time]
    } else if rest >= 34 {
        let title = rest * 60 / 100;
        [num, title, rest - title, 0, time]
    } else {
        [num, rest, 0, 0, time]
    }
}

fn table_head(f: &mut Frame, area: Rect, t: &Theme, album: bool) -> Rect {
    if area.height < 3 {
        return area;
    }
    let c = columns(area.width as usize, album);
    let mut s = String::new();
    s.push_str(&pad(" #", c[0]));
    s.push_str(&pad("Title", c[1]));
    if c[2] > 0 {
        s.push_str(&pad("Artist", c[2]));
    }
    if c[3] > 0 {
        s.push_str(&pad("Album", c[3]));
    }
    s.push_str(&format!("{:>w$}", "Time ", w = c[4]));
    put(f, Paragraph::new(Span::styled(s, t.dim())), row_of(area, 0));
    below(area, 1)
}

/// What a song row marks after its title: that a jam guest asked for the song, else its seal where the
/// profile has favorites, as the session's star marks show it, else that it is a provider's.
struct Marks<'a> {
    stars: &'a StarMarks,
    hearts: bool,
    asked: HashSet<&'a str>,
}

impl<'a> Marks<'a> {
    fn of(stars: &'a StarMarks, rules: &nori_core::browse::ProfileRules, devices: &'a crate::app::Devices) -> Marks<'a> {
        Marks { stars, hearts: rules.stars, asked: devices.asked() }
    }

    fn mark(&self, s: &Song) -> (&'static str, bool) {
        if self.asked.contains(s.id.as_str()) {
            (crate::text::ASKED, false)
        } else if self.hearts && self.stars.starred(Starrable::Song, &s.id, s.starred) {
            (SEAL, true)
        } else if s.is_provider() {
            (crate::text::REMOTE, false)
        } else {
            ("", false)
        }
    }
}

/// The seal: a liked song's mark in rows.
const SEAL: &str = "好";

/// A song table row; `number` 0 shows none, `playing` shows a marker instead; its mark as `marks` say.
fn song_line(s: &Song, marks: &Marks, number: usize, w: usize, t: &Theme, playing: bool, album: bool) -> Line<'static> {
    let c = columns(w, album);
    let num = if playing {
        Span::styled(pad("  ▶", c[0]), t.current())
    } else if number == 0 {
        Span::raw(pad("", c[0]))
    } else {
        Span::styled(pad(&format!("{number:>3}"), c[0]), t.dim())
    };
    let (mark, sealed) = marks.mark(s);
    let title_style = if playing { t.current() } else { Style::default().fg(t.ink) };
    let tw = c[1].saturating_sub(mark.width() + 2);
    let mut spans = vec![num, Span::styled(pad(&s.title, tw), title_style), Span::styled(pad(mark, mark.width() + 2), if sealed { Style::default().fg(t.accent) } else { t.dim() })];
    if c[2] > 0 {
        spans.push(Span::styled(pad(&s.artist, c[2].saturating_sub(2)), t.dim()));
        spans.push(Span::raw("  "));
    }
    if c[3] > 0 {
        spans.push(Span::styled(pad(&s.album, c[3].saturating_sub(2)), t.dim()));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(format!("{:>w$} ", clock(s.duration as i64 * 1000), w = c[4] - 1), t.dim()));
    Line::from(spans)
}

/// An album, artist, playlist or song as one row: its name, then what it is by, and its year or count at
/// the end.
fn item_line(item: &Item, w: usize, t: &Theme, starred: bool, playing: bool) -> Line<'static> {
    let (name, by, end) = match item {
        Item::Album(a) | Item::DownloadedAlbum(a) => (a.name.as_str(), a.artist.clone(), if a.year > 0 { a.year.to_string() } else { String::new() }),
        Item::Song(songs, at) => (songs[*at].title.as_str(), songs[*at].artist.clone(), clock(songs[*at].duration as i64 * 1000)),
        Item::Playlist(p) => (p.name.as_str(), p.owner.clone().unwrap_or_default(), crate::text::playlist_line(p)),
        Item::Mix(m) => (crate::text::mix_name(m.name), crate::text::mix_line(m.favourites).to_string(), String::new()),
        Item::Artist(a) => (a.name.as_str(), String::new(), a.album_count.map(crate::text::albums).unwrap_or_default()),
    };
    let name_w = (w * 11 / 20).max(12);
    let seal = if starred { format!(" {SEAL}") } else { String::new() };
    let name_style = if playing { t.current() } else { Style::default().fg(t.ink) };
    let left = vec![Span::styled(pad(&format!("{}{seal}", fit(name, name_w.saturating_sub(seal.width() + 2))), name_w), name_style), Span::styled(by, t.dim())];
    spread(left, Span::styled(format!("{end} "), t.dim()), w)
}

// ---- the frame ----

#[allow(clippy::needless_option_as_deref)]
pub fn draw(f: &mut Frame, app: &mut App, mut art: Option<&mut Art>) {
    app.hits.clear();
    app.shown.clock = false;
    let area = f.area();
    let t = app.look;
    put(f, Block::default().style(t.ground()), area);
    if app.view == View::Login {
        login(f, area, app, &t);
        overlay(f, area, app, &t);
        crate::art::finish(f.buffer_mut(), app.colors);
        return;
    }
    let rows = crate::chrome::rows(area);
    if app.full {
        let body = Rect { height: area.height.saturating_sub(rows.foot.height), ..area };
        full_player(f, body, app, art.as_deref_mut());
    } else {
        tab_line(f, rows.head, app, &t);
        page_view(f, rows.page, app, &t);
        now_playing(f, rows.now, app, &t);
    }
    foot(f, rows.foot, app, &t);
    overlay(f, area, app, &t);
    crate::art::finish(f.buffer_mut(), app.colors);
}

/// The tabs, the one open in the accent; on the right the server, and how it answers when it does not.
fn tab_line(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let open = crate::chrome::tab_name(app.view);
    let tabs = crate::chrome::tabs(area);
    let used = tabs.last().map_or(area.x, |(_, r)| r.x + r.width);
    for (view, r) in tabs {
        let name = crate::chrome::tab_name(view);
        put(f, Paragraph::new(Span::styled(name, if name == open { t.current() } else { t.dim() })), r);
        // The library's tab opens the section last open.
        let to = if view == View::Albums { app.library() } else { view };
        app.hits.push((r, Hit::Tab(to)));
    }
    let (state, colour) = if app.offline {
        (crate::text::OFFLINE, Some(Color::Yellow))
    } else if app.unreachable.is_some() {
        (crate::text::UNREACHABLE, Some(Color::LightRed))
    } else {
        ("", None)
    };
    let server = fit(&app.server, 24).into_owned();
    let w = (server.width() + if state.is_empty() { 0 } else { state.width() + 2 }) as u16;
    if used + 4 + w > area.x + area.width {
        return;
    }
    let mut spans = Vec::new();
    if let Some(c) = colour.filter(|_| !t.mono) {
        spans.push(Span::styled(format!("{state}  "), Style::default().fg(c)));
    } else if !state.is_empty() {
        spans.push(Span::styled(format!("{state}  "), bold()));
    }
    spans.push(Span::styled(server, t.dim()));
    put(f, Paragraph::new(Line::from(spans)), Rect { x: area.x + area.width - w - 1, width: w, ..area });
}

/// The keys of what has them, or the last note while it shows. A click shows every key.
fn foot(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let r = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    app.hits.push((r, Hit::Button(Button::Help)));
    if let Some((msg, error, _)) = &app.note {
        let style = if *error { Style::default().fg(Color::LightRed) } else { Style::default().fg(t.ink) };
        return text(f, r, msg, style);
    }
    let mut spans = Vec::new();
    for (k, word) in hints(app) {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(*k, Style::default().fg(t.ink)));
        spans.push(Span::styled(format!(" {word}"), t.dim()));
    }
    let line = Line::from(spans);
    let line = if line.width() > r.width as usize { spread(line.spans, Span::raw(""), r.width as usize) } else { line };
    put(f, Paragraph::new(line), r);
}

/// The keys the line along the bottom shows for what has them.
fn hints(app: &App) -> &'static [(&'static str, &'static str)] {
    use crate::text::hints as h;
    if app.view == View::Search && app.search.editing && !app.full {
        return h::TYPING;
    }
    if app.full {
        return match app.panel {
            Panel::Queue => h::QUEUE,
            Panel::Lyrics => h::LYRICS,
            Panel::Devices => h::DEVICES,
            Panel::Playing => h::PLAYING,
        };
    }
    if app.page().is_some() {
        return h::PAGE;
    }
    match app.view {
        View::Home if app.home.line() == Some(crate::app::HomeLine::Offer) => h::OFFER,
        View::Home => h::HOME,
        View::Settings => h::SETTINGS,
        View::Equalizer => h::EQUALIZER,
        View::Downloads => h::DOWNLOADS,
        View::Search => h::RESULTS,
        View::Albums | View::Songs => h::SORTED,
        View::History => h::HISTORY,
        _ => h::LIBRARY,
    }
}

// ---- the page ----

fn page_view(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let mut area = Rect { x: area.x + 1, width: area.width - 2, ..area };
    if let Some(e) = &app.unreachable {
        text(f, area, &crate::text::stored_only(e), Style::default().fg(Color::LightRed));
        area = below(area, 1);
    }
    let inner = below(area, 1);
    let focused = app.focus == Focus::Main;
    if !app.pages.is_empty() {
        return page(f, inner, app, t, focused);
    }
    match app.view {
        View::Home => home(f, inner, app, t, focused),
        View::Search => search(f, inner, app, t, focused),
        View::Albums | View::Artists | View::Songs | View::Playlists => {
            let body = sections(f, inner, app, t);
            library_section(f, body, app, t, focused);
        }
        View::History => history(f, inner, app, t, focused),
        View::Favorites => favorites(f, inner, app, t, focused),
        View::Downloads => downloads(f, inner, app, t, focused),
        View::Equalizer => equalizer(f, inner, app, t, focused),
        View::Settings => settings(f, inner, app, t, focused),
        View::Login => {}
    }
}

/// The library's sections along the top of each, the open one bold and underlined; returns the area under
/// them and a blank line.
fn sections(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) -> Rect {
    let mut x = area.x;
    for v in app.sections() {
        let name = v.name();
        let w = name.width() as u16;
        if x + w > area.x + area.width {
            break;
        }
        let style = if v == app.view { bold().fg(t.ink).add_modifier(Modifier::UNDERLINED) } else { t.dim() };
        let r = Rect { x, width: w, ..row_of(area, 0) };
        put(f, Paragraph::new(Span::styled(name, style)), r);
        app.hits.push((r, Hit::Tab(v)));
        x += w + 3;
    }
    below(area, 2)
}

fn failed(f: &mut Frame, area: Rect, t: &Theme, e: &str) {
    let text = vec![
        Line::from(Span::styled("Could not load this page", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(e.to_string(), Style::default().fg(t.ink))),
        Line::from(""),
        Line::from(Span::styled("R tries again", t.dim())),
    ];
    put(f, Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true }), Rect { height: area.height.min(6), ..area });
}

fn loading(f: &mut Frame, area: Rect, t: &Theme) {
    text(f, area, "Loading…", t.dim());
}

fn empty(f: &mut Frame, area: Rect, t: &Theme, text: &str) {
    put(f, Paragraph::new(Span::styled(text.to_string(), t.dim())).wrap(ratatui::widgets::Wrap { trim: true }), Rect { height: area.height.min(2), ..area });
}

fn home(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let area = heading(f, area, crate::text::LISTEN_NOW, "", t);
    let lines = app.home.lines();
    if lines.is_empty() {
        match &app.home.error {
            Some(e) => failed(f, area, t, e),
            None => loading(f, area, t),
        }
        return;
    }
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { home, hits, marks, rules, .. } = app;
    let h = &*home;
    let row = |i: usize, w: usize| match lines[i] {
        HomeLine::Offer => offer_line(h.offer.as_ref(), w, t),
        HomeLine::Shelf(at) => {
            let (title, _) = h.shelf(at).unwrap_or_default();
            let more = if h.count(at) > crate::app::HOME_ENTRIES { crate::text::SHOW_ALL } else { "" };
            spread(vec![Span::styled(title, bold().fg(t.ink))], Span::styled(format!("{more} "), t.dim()), w)
        }
        HomeLine::Entry(at, n) => {
            let Some(item) = h.shelf(at).and_then(|(_, v)| v.into_iter().nth(n)) else { return Line::from("") };
            let mut line = item_line(&item, w.saturating_sub(2), t, starred(&item, marks, rules.stars), is_playing(&item, playing.as_deref()));
            line.spans.insert(0, Span::raw("  "));
            line
        }
        HomeLine::Gap => Line::from(""),
    };
    let mut sel = home.sel;
    list(f, area, &mut sel, lines.len(), ListRef::Home, hits, t, focused, &row);
    home.sel = sel;
    // The offer's own words act on it too.
    if let Some(r) = app.hits.iter().find(|(_, h)| *h == Hit::Row(ListRef::Home, 0)).map(|(r, _)| *r).filter(|_| app.home.offer.is_some() && sel.top == 0) {
        let words = [(Span::styled(crate::text::RESUME, bold().fg(t.ink)), Button::Resume), (Span::styled(crate::text::CLOSE, t.dim()), Button::Dismiss)];
        let w: u16 = words.iter().map(|(s, _)| s.width() as u16 + 3).sum::<u16>() - 3;
        let at = Rect { x: r.x + r.width.saturating_sub(w + 2), width: w, ..r };
        buttons(f, &mut app.hits, at, &words);
    }
}

/// The queue another device left, offered to carry on here: what it is called, the song, and where it
/// stands; its Resume and Close are drawn over its end.
fn offer_line(offer: Option<&nori_core::library::QueueOffer>, w: usize, t: &Theme) -> Line<'static> {
    let Some(o) = offer else { return Line::from("") };
    let Some(song) = o.songs.get(o.index as usize) else { return Line::from("") };
    let place = crate::text::offer_place(&song.artist, o.index + 1, o.songs.len());
    let left = vec![Span::styled(format!("{}  ", crate::text::CONTINUE), t.dim()), Span::styled(song.title.clone(), bold().fg(t.ink)), Span::styled(format!("  {place}"), t.dim())];
    let room = crate::text::RESUME.width() + crate::text::CLOSE.width() + 6;
    spread(left, Span::raw(" ".repeat(room)), w)
}

/// Whether `item` shows its seal: an album or artist starred, as this session marked it.
fn starred(item: &Item, marks: &StarMarks, stars: bool) -> bool {
    stars
        && match item {
            Item::Album(a) | Item::DownloadedAlbum(a) => marks.starred(Starrable::Album, &a.id, a.starred),
            Item::Artist(a) => marks.starred(Starrable::Artist, &a.id, a.starred),
            Item::Song(songs, at) => marks.starred(Starrable::Song, &songs[*at].id, songs[*at].starred),
            Item::Playlist(_) | Item::Mix(_) => false,
        }
}

fn is_playing(item: &Item, playing: Option<&str>) -> bool {
    matches!(item, Item::Song(songs, at) if Some(songs[*at].id.as_str()) == playing)
}

/// How far a list has loaded: its length once read, or why it was not.
fn state<T>(load: &Load<Vec<T>>) -> Result<Option<usize>, String> {
    match load {
        Load::Ready(v) => Ok(Some(v.len())),
        Load::Failed(e) => Err(e.clone()),
        _ => Ok(None),
    }
}

/// A library section: how many it holds and its order on one line, then its places, then its entries;
/// in Artist order the albums under a line for each artist.
fn library_section(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let view = app.view;
    let lines = app.library_lines();
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { library: l, hits, marks, rules, devices, .. } = app;
    let stars = rules.stars;
    let seals = &*marks;
    let marks = Marks::of(marks, rules, devices);
    let (loaded, more, one, many) = match view {
        View::Albums => (state(&l.albums), l.albums_more, "album", "albums"),
        View::Artists => (state(&l.artists), false, "artist", "artists"),
        View::Songs => (state(&l.songs), l.songs_more, "song", "songs"),
        _ => (state(&l.playlists), false, "playlist", "playlists"),
    };
    let (order, favorites) = match view {
        View::Albums => (Some(crate::text::album_sort(l.album_sort)), l.albums_favorites),
        View::Songs => (Some(crate::text::song_sort(&l.song_sort)), l.songs_favorites),
        _ => (None, false),
    };
    let caption = match &loaded {
        Ok(Some(n)) => crate::text::count(*n as u64, one, many) + if more { "+" } else { "" },
        _ => String::new(),
    };
    let only = if favorites { format!("{}   ", crate::text::FAVORITES_ONLY) } else { String::new() };
    let sort = order.map_or_else(String::new, |o| format!("{only}{}  {o} ", crate::text::SORT_BY));
    put(f, Paragraph::new(spread(vec![Span::styled(caption, t.dim())], Span::styled(sort, t.dim()), area.width as usize)), row_of(area, 0));
    let body = below(area, 1);
    match loaded {
        Err(e) => return failed(f, body, t, &e),
        Ok(None) => return loading(f, body, t),
        Ok(Some(_)) if lines.is_empty() => return empty(f, body, t, "Nothing here yet"),
        Ok(Some(_)) => {}
    }
    let size = l.downloads_bytes.map(|b| nori_core::numbers::bytes(b, nori_core::numbers::POINT));
    let albums = l.albums.ready().map_or(&[][..], Vec::as_slice);
    let row = |i: usize, w: usize| match lines[i] {
        LibLine::Place(p) => {
            let end = if p == Place::Downloads { size.clone().unwrap_or_default() } else { String::new() };
            spread(vec![Span::styled(crate::text::place(p), Style::default().fg(t.ink))], Span::styled(format!("{end} "), t.dim()), w)
        }
        LibLine::Gap => Line::from(""),
        LibLine::Shelf(start) => {
            let a = &albums[start];
            let n = albums[start..].iter().take_while(|x| x.artist_id == a.artist_id && x.artist == a.artist).count();
            spread(vec![Span::styled(a.artist.clone(), bold().fg(t.ink))], Span::styled(format!("{} ", crate::text::albums(n as u32)), t.dim()), w)
        }
        LibLine::Entry(k) => match view {
            View::Songs => {
                let v = l.songs.ready().map_or(&[][..], Vec::as_slice);
                song_line(&v[k], &marks, k + 1, w, t, playing.as_deref() == Some(v[k].id.as_str()), true)
            }
            View::Albums if l.album_sort == AlbumSort::ArtistRows => {
                // Under its artist: the album and its year.
                let a = &albums[k];
                let seal = if starred(&Item::Album(a.clone()), seals, stars) { format!(" {SEAL}") } else { String::new() };
                spread(vec![Span::raw("  "), Span::styled(format!("{}{seal}", a.name), Style::default().fg(t.ink))], Span::styled(if a.year > 0 { format!("{} ", a.year) } else { String::new() }, t.dim()), w)
            }
            _ => {
                let item = match view {
                    View::Albums => Item::Album(albums[k].clone()),
                    View::Artists => Item::Artist(l.artists.ready().map(|v| v[k].clone()).unwrap_or_default()),
                    _ => Item::Playlist(l.playlists.ready().map(|v| v[k].clone()).unwrap_or_default()),
                };
                item_line(&item, w, t, starred(&item, seals, stars), false)
            }
        },
    };
    let (mut sel, lref) = match view {
        View::Albums => (l.albums_sel, ListRef::Albums),
        View::Artists => (l.artists_sel, ListRef::Artists),
        View::Songs => (l.songs_sel, ListRef::Songs),
        _ => (l.playlists_sel, ListRef::Playlists),
    };
    list(f, body, &mut sel, lines.len(), lref, hits, t, focused, &row);
    *match view {
        View::Albums => &mut l.albums_sel,
        View::Artists => &mut l.artists_sel,
        View::Songs => &mut l.songs_sel,
        _ => &mut l.playlists_sel,
    } = sel;
}

/// What this device played, newest first.
fn history(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { library: l, hits, marks, rules, devices, .. } = app;
    let marks = Marks::of(marks, rules, devices);
    let body = heading(f, area, View::History.name(), "", t);
    match &l.history {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "Nothing played yet"),
        Load::Ready(v) => {
            let row = |i: usize, w: usize| song_line(&v[i].song, &marks, 0, w, t, playing.as_deref() == Some(v[i].song.id.as_str()), true);
            list(f, body, &mut l.history_sel, v.len(), ListRef::History, hits, t, focused, &row);
        }
        Load::Failed(e) => failed(f, body, t, e),
        _ => loading(f, body, t),
    }
}

fn favorites(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { library: l, hits, marks, rules, devices, .. } = app;
    let marks = Marks::of(marks, rules, devices);
    let caption = l.favorites.ready().map_or(String::new(), |v| crate::text::count(v.len() as u64, "song", "songs"));
    let body = heading(f, area, View::Favorites.name(), &caption, t);
    match &l.favorites {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "No favorites yet · f marks one"),
        Load::Ready(v) => {
            let body = table_head(f, body, t, true);
            let row = |i: usize, w: usize| song_line(&v[i], &marks, i + 1, w, t, playing.as_deref() == Some(v[i].id.as_str()), true);
            list(f, body, &mut l.favorites_sel, v.len(), ListRef::Favorites, hits, t, focused, &row);
        }
        Load::Failed(e) => failed(f, body, t, e),
        _ => loading(f, body, t),
    }
}

/// The search: what is typed after a slash, where it was found, then the results under their kinds.
fn search(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    if area.height < 3 {
        return;
    }
    let s = &app.search;
    let editing = s.editing;
    let cursor = if editing { "▏" } else { "" };
    let room = area.width.saturating_sub(3) as usize;
    // The end of what is typed, where the cursor is.
    let typed = format!("{}{cursor}", s.text);
    let tail: String = typed.chars().rev().take(room).collect::<Vec<_>>().into_iter().rev().collect();
    let prompt = Line::from(vec![Span::styled("/ ", if editing { t.current() } else { t.dim() }), Span::styled(tail, bold().fg(t.ink))]);
    let field = row_of(area, 0);
    put(f, Paragraph::new(prompt), field);
    app.hits.push((field, Hit::SearchField));
    let view = s.view.as_ref();
    let (words, style) = match view {
        Some(v) if v.error.is_some() => (crate::text::search_fallback(v.error.as_ref().and_then(|e| e.reason.as_deref())), Style::default().fg(Color::LightRed)),
        Some(v) if v.searching => ("Asking the server…".to_string(), t.dim()),
        Some(v) if v.nothing_found => ("Nothing found".to_string(), t.dim()),
        Some(v) if !v.query.is_empty() => ((if v.from_server { "From the server" } else { "From the offline index · the server is asked when you pause" }).to_string(), t.dim()),
        _ => ("Songs, albums and artists, from the library".to_string(), t.dim()),
    };
    text(f, row_of(area, 1), &words, style);
    let body = below(area, 3);
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { search: s, hits, marks, rules, devices, .. } = app;
    let stars = rules.stars;
    let seals = &*marks;
    let marks = Marks::of(marks, rules, devices);
    let lit = focused && !s.editing;
    let mut sel = s.sel;
    let rows = s.rows();
    if rows.is_empty() {
        return;
    }
    let len = rows.len();
    let row = |i: usize, w: usize| {
        let entry = |item: Item| {
            let mut line = item_line(&item, w.saturating_sub(2), t, starred(&item, seals, stars), false);
            line.spans.insert(0, Span::raw("  "));
            line
        };
        match &rows[i] {
            SearchRow::Title(title, n) => spread(vec![Span::styled(title.to_string(), bold().fg(t.ink))], Span::styled(format!("{n} "), t.dim()), w),
            SearchRow::Song(song) => song_line(song, &marks, 0, w, t, playing.as_deref() == Some(song.id.as_str()), true),
            SearchRow::Album(a) => entry(Item::Album((*a).clone())),
            SearchRow::Artist(a) => entry(Item::Artist((*a).clone())),
            SearchRow::Playlist(p) => entry(Item::Playlist((*p).clone())),
        }
    };
    list(f, body, &mut sel, len, ListRef::Search, hits, t, lit, &row);
    drop(rows);
    s.sel = sel;
}

/// A cover in `area`, or nothing while it is not loaded.
fn cover(f: &mut Frame, area: Rect, art: Option<&mut Art>, key: Option<&str>) {
    let area = area.intersection(f.area());
    if area.width < 2 || area.height < 1 {
        return;
    }
    let Some(p) = art.zip(key).and_then(|(a, k)| a.get(k)) else { return };
    let resize = Resize::Scale(Some(image::imageops::FilterType::Triangle));
    let encode = ratatui_image::ResizeEncodeRender::needs_resize(p, &resize, area);
    f.render_stateful_widget(StatefulImage::<StatefulProtocol>::default().resize(resize), area, p);
    forced_width(f, area);
    // Encoded for this area now: this frame transmits it.
    if let Some(rect) = encode {
        match p.last_encoding_result() {
            Some(Err(e)) => eprintln!("nori: cover {} would not encode for {}x{} cells: {e}", key.unwrap_or_default(), rect.width, rect.height),
            _ => crate::term::debug!("cover {} encoded for {}x{} cells at {},{}: sent with this frame", key.unwrap_or_default(), rect.width, rect.height, area.x, area.y),
        }
    }
}

/// Marks graphics-protocol cells as one column wide. The picture is one long escape sequence in a single
/// cell, and ratatui 0.30's diff would otherwise skip that many columns, hiding the rest of the frame.
fn forced_width(f: &mut Frame, area: Rect) {
    let one = ratatui::buffer::CellDiffOption::ForcedWidth(std::num::NonZeroU16::MIN);
    let buf = f.buffer_mut();
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if cell.symbol().starts_with('\x1b') {
                cell.set_diff_option(one);
            }
        }
    }
}

/// Words that act, from `x` on row `r`, two columns apart; each one clicked is its button.
fn buttons(f: &mut Frame, hits: &mut Vec<(Rect, Hit)>, r: Rect, words: &[(Span<'static>, Button)]) {
    let mut x = r.x;
    for (word, b) in words {
        let w = word.width() as u16;
        if x + w > r.x + r.width {
            return;
        }
        let at = Rect { x, width: w, ..r };
        put(f, Paragraph::new(word.clone()), at);
        hits.push((at, Hit::Button(*b)));
        x += w + 3;
    }
}

/// An album's, artist's, playlist's or mix's page: its name with the seal; who made it, with Play and
/// Shuffle (and a mix's Save as playlist); the list's head, what it holds and what the core offers to do with
/// it (`menus::page_actions`); then its songs, or an artist's albums under their shelves.
fn page(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let account = app.rules.account;
    let stars = app.rules.stars;
    let actions = app.page_actions();
    let missing = app.page_missing();
    let App { pages, hits, marks, rules, devices, .. } = app;
    let seals = &*marks;
    let marks = Marks::of(marks, rules, devices);
    let Some(p) = pages.last_mut() else { return };
    // star: None when the page cannot be starred.
    let (title, by, caption, star): (String, String, String, Option<bool>) = match p {
        Page::Album { detail: Load::Ready(d), .. } | Page::DownloadedAlbum { detail: Load::Ready(d), .. } => (d.album.name.clone(), d.album.artist.clone(), crate::text::album_caption(d), Some(marks.stars.starred(Starrable::Album, &d.album.id, d.album.starred))),
        Page::Artist { detail: Load::Ready(d), .. } => (d.artist.name.clone(), String::new(), crate::text::artist_caption(d), Some(marks.stars.starred(Starrable::Artist, &d.artist.id, d.artist.starred))),
        Page::Playlist { detail: Load::Ready(d), .. } => (d.playlist.name.clone(), d.playlist.owner.clone().unwrap_or_default(), crate::text::playlist_caption(d), None),
        Page::Row { title, items, .. } => (title.clone(), String::new(), crate::text::count(items.len() as u64, "entry", "entries"), None),
        Page::Mix { detail: Load::Ready(m), .. } => (crate::text::mix_name(m.name).to_string(), String::new(), crate::text::songs_caption(m.songs.len() as u32, m.seconds), None),
        Page::Mix { detail: Load::Failed(e), .. } => return failed(f, area, t, e),
        Page::Album { detail: Load::Failed(e), .. } | Page::DownloadedAlbum { detail: Load::Failed(e), .. } | Page::Artist { detail: Load::Failed(e), .. } | Page::Playlist { detail: Load::Failed(e), .. } => {
            return failed(f, area, t, e);
        }
        _ => return loading(f, area, t),
    };
    if area.height < 5 {
        return;
    }
    // The title, and the seal at its end where the account can star the page's.
    let sealed = star.filter(|_| account && stars);
    let seal_w = if sealed.is_some() { 3 } else { 0 };
    text(f, Rect { width: area.width.saturating_sub(seal_w), ..row_of(area, 0) }, &title, bold().fg(t.ink));
    if let Some(on) = sealed {
        let r = Rect { x: area.x + (title.width() as u16 + 1).min(area.width.saturating_sub(2)), width: 2, ..row_of(area, 0) };
        put(f, Paragraph::new(seal(on, false, t.dim(), t)), r);
        hits.push((r, Hit::Button(Button::Star)));
    }
    // Words that act, right-aligned on `r` beside `left`.
    let mut line = |r: Rect, left: &str, words: Vec<(Span<'static>, Button)>| {
        let w: u16 = words.iter().map(|(w, _)| w.width() as u16 + 3).sum::<u16>().saturating_sub(3);
        text(f, Rect { width: r.width.saturating_sub(w + 2), ..r }, left, t.dim());
        buttons(f, hits, Rect { x: r.x + r.width.saturating_sub(w + 1), width: w, ..r }, &words);
    };
    let mut plays: Vec<(Span<'static>, Button)> = Vec::new();
    if !matches!(p, Page::Row { .. }) {
        plays.push((Span::styled(crate::text::PLAY, bold().fg(t.ink)), Button::PlayAll));
        plays.push((Span::styled(crate::text::SHUFFLE, Style::default().fg(t.ink)), Button::ShuffleAll));
    }
    if matches!(p, Page::Mix { detail: Load::Ready(m), .. } if account && !m.songs.is_empty()) {
        plays.push((Span::styled(crate::text::SAVE_AS_PLAYLIST, Style::default().fg(t.ink)), Button::SaveMix));
    }
    line(row_of(area, 1), &by, plays);
    let head: Vec<(Span<'static>, Button)> = actions.iter().map(|a| (Span::styled(crate::text::page_action(*a, missing), t.dim()), Button::Page(*a))).collect();
    line(row_of(area, 2), &caption, head);
    let body = below(area, 4);
    match p {
        Page::Album { detail: Load::Ready(d), sel, .. } | Page::DownloadedAlbum { detail: Load::Ready(d), sel, .. } => {
            let multi = d.discs.len() > 1;
            let songs = &d.songs;
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| {
                let n = if multi { songs[i].disc_number as usize * 100 + songs[i].track as usize } else { songs[i].track as usize };
                song_line(&songs[i], &marks, if n > 0 { n } else { i + 1 }, w, t, playing.as_deref() == Some(songs[i].id.as_str()), false)
            });
        }
        Page::Playlist { detail: Load::Ready(d), sel, .. } => {
            let songs = &d.songs;
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| song_line(&songs[i], &marks, i + 1, w, t, playing.as_deref() == Some(songs[i].id.as_str()), true));
        }
        Page::Artist { detail: Load::Ready(d), sel, .. } if d.albums.is_empty() && !d.songs.is_empty() => {
            let songs = &d.songs;
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| song_line(&songs[i], &marks, i + 1, w, t, playing.as_deref() == Some(songs[i].id.as_str()), true));
        }
        Page::Artist { detail: Load::Ready(d), sel, .. } => {
            let lines = crate::app::artist_lines(d);
            list(f, body, sel, lines.len(), ListRef::Page, hits, t, focused, &|i, w| match lines[i] {
                crate::app::ArtistLine::Shelf(k) => spread(vec![Span::styled(crate::text::release_shelf(&d.groups[k]), bold().fg(t.ink))], Span::styled(format!("{} ", d.groups[k].albums.len()), t.dim()), w),
                crate::app::ArtistLine::Album(k) => {
                    let a = &d.albums[k];
                    let seal = if starred(&Item::Album(a.clone()), seals, stars) { format!(" {SEAL}") } else { String::new() };
                    spread(vec![Span::raw("  "), Span::styled(format!("{}{seal}", a.name), Style::default().fg(t.ink))], Span::styled(if a.year > 0 { format!("{} ", a.year) } else { String::new() }, t.dim()), w)
                }
            });
        }
        Page::Mix { detail: Load::Ready(m), sel, .. } => {
            let songs = &m.songs;
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| song_line(&songs[i], &marks, i + 1, w, t, playing.as_deref() == Some(songs[i].id.as_str()), true));
        }
        Page::Row { items, sel, .. } => {
            list(f, body, sel, items.len(), ListRef::Page, hits, t, focused, &|i, w| item_line(&items[i], w, t, starred(&items[i], seals, stars), is_playing(&items[i], playing.as_deref())));
        }
        _ => {}
    }
}

fn downloads(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let body = heading(f, area, View::Downloads.name(), "D on a song, an album or a playlist downloads it", t);
    match &app.downloads {
        Load::Failed(e) => return failed(f, body, t, &e.clone()),
        Load::Ready(_) => {}
        _ => return loading(f, body, t),
    }
    let App { downloads, downloads_sel, hits, .. } = app;
    let rows = downloads.ready().map(|d| d.rows()).unwrap_or_default();
    if rows.is_empty() {
        return empty(f, body, t, "Nothing downloaded yet");
    }
    list(f, body, downloads_sel, rows.len(), ListRef::Downloads, hits, t, focused, &|i, w| match &rows[i] {
        crate::app::DownloadRow::Header(title) => Line::from(Span::styled(title.clone(), bold().fg(t.ink))),
        crate::app::DownloadRow::Album(a) => {
            let mut line = item_line(&Item::DownloadedAlbum((*a).clone()), w.saturating_sub(2), t, false, false);
            line.spans.insert(0, Span::raw("  "));
            line
        }
        crate::app::DownloadRow::Song(list, i) => {
            let mut line = item_line(&Item::Song(list.to_vec(), *i), w.saturating_sub(2), t, false, false);
            line.spans.insert(0, Span::raw("  "));
            line
        }
    });
}

/// The equalizer: its switches and choices, band faders, then the rest of the chain. ← → walk them in
/// that order; ↑ ↓ change the selected one.
fn equalizer(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let p = app.prefs.clone();
    let bypass = nori_core::settings::eq_bypass(p.bit_perfect, p.sound_bypass);
    let head: &str = match bypass {
        Some(why) => crate::text::eq_bypass(why),
        None if p.eq_enabled => "On · heard at once while this is open",
        None => "Off · switch it on to hear the bands",
    };
    let area = heading(f, area, View::Equalizer.name(), head, t);
    if area.height < 6 || area.width < 20 {
        return;
    }
    let rows = crate::settings_view::eq_rows(&p);
    app.eq_sel.at = app.eq_sel.at.min(rows.len().saturating_sub(1));
    let chosen = app.eq_sel.at;
    let top: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].above_bands()).collect();
    let bands: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].band(&p).is_some()).collect();
    let under: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].above_bands() && rows[i].band(&p).is_none()).collect();
    let chip = |i: usize| rows[i].chip(&p);
    let top_h = chips(f, area, &top, &chip, chosen, focused, &mut app.hits, t, &|i| lit(&rows[i], &p));
    let under_h = chips_height(area.width, &under, &chip);
    let bottom = area.y + area.height;
    let under_area = Rect { y: bottom.saturating_sub(under_h), height: under_h, ..area };
    chips(f, under_area, &under, &chip, chosen, focused, &mut app.hits, t, &|i| lit(&rows[i], &p));
    let mid = Rect { y: area.y + top_h + 1, height: under_area.y.saturating_sub(area.y + top_h + 2), ..area };
    let on_ = p.eq_enabled && bypass.is_none();
    curve(f, mid, &rows, &bands, &p, chosen, on_, app, t);
}

/// Whether a switch is on.
fn lit(row: &EqRow, p: &nori_core::settings::StoredPrefs) -> bool {
    match row {
        EqRow::Enabled => p.eq_enabled,
        EqRow::AutoPreamp => p.eq_preamp_db.is_none(),
        EqRow::Mono => p.mono,
        EqRow::Limiter => p.limiter,
        _ => false,
    }
}

/// Rows the wrapped controls take in `w` columns.
fn chips_height(w: u16, which: &[usize], label: &dyn Fn(usize) -> String) -> u16 {
    if which.is_empty() {
        return 0;
    }
    let (mut x, mut lines) = (0u16, 1u16);
    for &i in which {
        let cw = label(i).width() as u16 + 2;
        if x > 0 && x + cw > w {
            lines += 1;
            x = 0;
        }
        x += cw + 1;
    }
    lines
}

/// Controls as words, wrapped; the selected one raised, switches that are on in the `on` color. Returns the
/// rows used.
#[allow(clippy::too_many_arguments)]
fn chips(f: &mut Frame, area: Rect, which: &[usize], label: &dyn Fn(usize) -> String, chosen: usize, focused: bool, hits: &mut Vec<(Rect, Hit)>, t: &Theme, on_: &dyn Fn(usize) -> bool) -> u16 {
    let (mut x, mut y) = (area.x, area.y);
    for &i in which {
        let text = label(i);
        let cw = text.width() as u16 + 2;
        if x > area.x && x + cw > area.x + area.width {
            x = area.x;
            y += 1;
        }
        if y >= area.y + area.height.max(1) {
            break;
        }
        let r = Rect { x, y, width: cw.min(area.width), height: 1 };
        let mut style = Style::default().fg(if on_(i) { t.on } else { t.ink });
        if i == chosen {
            style = style.patch(t.select(focused));
        }
        put(f, Paragraph::new(Span::styled(format!(" {text} "), style)), r);
        hits.push((r, Hit::Row(ListRef::Eq, i)));
        x += cw + 1;
    }
    (y + 1).saturating_sub(area.y)
}

/// The curve the equalizer plays, in braille dots (2 across and 4 down a cell), with a point on it for
/// each band: the one chosen in the accent, and a parametric band's own shape shaded. Under it the scale,
/// then the chosen band in words.
#[allow(clippy::too_many_arguments)]
fn curve(f: &mut Frame, area: Rect, rows: &[EqRow], bands: &[usize], p: &nori_core::settings::StoredPrefs, chosen: usize, on_: bool, app: &mut App, t: &Theme) {
    const SCALE_W: u16 = 5;
    if area.height < 6 || area.width < SCALE_W + 20 {
        return;
    }
    let reach = nori_core::dsp::CURVE_DB;
    let h = (area.height - 2).min(21);
    let plot = Rect { x: area.x + SCALE_W + 1, y: area.y, width: area.width - SCALE_W - 2, height: h };
    app.eq_plot = plot;
    app.hits.push((plot, Hit::Curve));
    let (dw, dh) = (plot.width as usize * 2, plot.height as usize * 4);
    let drawing = nori_core::dsp::eq_drawing(p, dw as u32);
    let dot_y = |db: f32| ((1.0 - db.clamp(-reach, reach) / reach) / 2.0 * (dh - 1) as f32).round() as usize;
    let dot_x = |x: f32| ((x.clamp(0.0, 1.0)) * (dw - 1) as f32).round() as usize;
    // Four layers of dots, from the back: the grid, the area under the curve, the chosen band's shape,
    // the curve itself. A cell takes the colour of its frontmost layer.
    let mut layers = vec![vec![0u8; plot.width as usize * plot.height as usize]; 4];
    let mut dot = |layer: usize, x: usize, y: usize| {
        const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
        if x < dw && y < dh {
            layers[layer][(y / 4) * plot.width as usize + x / 2] |= BITS[y % 4][x % 2];
        }
    };
    let zero = dot_y(0.0);
    for x in (0..dw).step_by(2) {
        dot(0, x, zero);
        for db in [-12.0, 12.0] {
            if x % 8 == 0 {
                dot(0, x, dot_y(db));
            }
        }
    }
    let ticks = [(100.0f32, "100"), (1000.0, "1k"), (10_000.0, "10k")];
    for (freq, _) in ticks {
        let x = dot_x(nori_core::dsp::band_position(freq));
        for y in (0..dh).step_by(4) {
            dot(0, x, y);
        }
    }
    let shade = |layer: usize, r: &[f32], dot: &mut dyn FnMut(usize, usize, usize)| {
        for (x, db) in r.iter().enumerate() {
            let y = dot_y(*db);
            for y in y.min(zero)..=y.max(zero) {
                if x % 2 == 0 && y % 2 == 0 {
                    dot(layer, x, y);
                }
            }
        }
    };
    shade(1, &drawing.curve, &mut dot);
    let chosen_band = rows.get(chosen).and_then(|r| r.band(p)).map(|(i, _)| i);
    if let Some(own) = chosen_band.and_then(|i| drawing.each.get(i)) {
        shade(2, own, &mut dot);
    }
    let mut last: Option<usize> = None;
    for (x, db) in drawing.curve.iter().enumerate() {
        let y = dot_y(*db);
        let from = last.unwrap_or(y);
        for y in y.min(from)..=y.max(from) {
            dot(3, x, y);
        }
        last = Some(y);
    }
    let colours = [t.quiet, t.quiet, if on_ { t.accent } else { t.quiet }, if on_ { t.accent } else { t.ink }];
    let buf = f.buffer_mut();
    for cy in 0..plot.height {
        for cx in 0..plot.width {
            let k = cy as usize * plot.width as usize + cx as usize;
            let bits = layers.iter().fold(0u8, |a, l| a | l[k]);
            if bits == 0 {
                continue;
            }
            let front = (0..4).rev().find(|&l| layers[l][k] != 0).unwrap_or(0);
            let style = if front == 0 { t.dim() } else { Style::default().fg(colours[front]) };
            if let Some(cell) = buf.cell_mut((plot.x + cx, plot.y + cy)) {
                cell.set_char(char::from_u32(0x2800 + bits as u32).unwrap_or(' ')).set_style(style);
            }
        }
    }
    // The points, over the curve; each answers a click on it and the cell either side.
    for (k, &i) in bands.iter().enumerate() {
        let Some(point) = drawing.points.get(k) else { continue };
        let cx = plot.x + (dot_x(point.x) / 2) as u16;
        let cy = plot.y + (dot_y(point.db) / 4) as u16;
        let is = i == chosen;
        let style = Style::default().fg(if !on_ { t.quiet } else if is { t.accent } else { t.ink }).add_modifier(Modifier::BOLD);
        put(f, Paragraph::new(Span::styled(if is { "◆" } else { "●" }, style)), Rect { x: cx, y: cy, width: 1, height: 1 });
        app.hits.push((Rect { x: cx.saturating_sub(1), y: cy, width: 3, height: 1 }.intersection(plot), Hit::Row(ListRef::Eq, i)));
    }
    for (db, s) in [(12.0, "+12"), (0.0, "0 dB"), (-12.0, "−12")] {
        let y = plot.y + (dot_y(db) / 4) as u16;
        put(f, Paragraph::new(Span::styled(format!("{s:>w$}", w = SCALE_W as usize), t.dim())), Rect { x: area.x, y, width: SCALE_W, height: 1 });
    }
    let scale_y = plot.y + plot.height;
    for (freq, name) in ticks {
        let x = plot.x + (dot_x(nori_core::dsp::band_position(freq)) / 2) as u16;
        put(f, Paragraph::new(Span::styled(name, t.dim())), Rect { x, y: scale_y, width: name.len() as u16, height: 1 });
    }
    let line = match chosen_band {
        Some(i) => Line::from(vec![Span::styled(crate::text::band_line(p, i), Style::default().fg(if on_ { t.accent } else { t.ink })), Span::styled(format!("    {}", crate::text::eq_curve_keys(p.eq_mode == nori_core::settings::EqMode::Graphic)), t.dim())]),
        None => Line::from(Span::styled(crate::text::eq_curve_keys(p.eq_mode == nori_core::settings::EqMode::Graphic), t.dim())),
    };
    put(f, Paragraph::new(line), Rect { x: plot.x, y: scale_y + 1, width: plot.width, height: 1 });
}

fn settings(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let prefs = app.prefs.clone();
    let own = app.settings.own.clone();
    app.settings.own = settings_view::Own { mouse: app.mouse, images: app.images, paint: app.paint, protocol: app.protocol.to_string(), dark: app.dark, ..own };
    let body = heading(f, area, View::Settings.name(), "", t);
    let App { settings: v, hits, .. } = app;
    let pages = v.pages(&prefs).to_vec();
    let lines = SettingsView::lines(&pages);
    let len = lines.len();
    if v.row.at == 0 && len > 0 {
        v.skip_titles(true);
    }
    // The groups down the left when wide enough.
    let body = if body.width >= 96 && !v.alone {
        let at = v.group_at();
        for (g, group) in settings_view::GROUPS.iter().enumerate() {
            let r = Rect { x: body.x, y: body.y + g as u16, width: 18, height: 1 };
            if r.y >= body.y + body.height {
                break;
            }
            let style = if g == at { bold().fg(t.ink) } else { t.dim() };
            put(f, Paragraph::new(Span::styled(group.title, style)), r);
            hits.push((r, Hit::Group(g)));
        }
        Rect { x: body.x + 20, width: (body.width - 20).min(110), ..body }
    } else {
        Rect { width: body.width.min(110), ..body }
    };
    list(f, body, &mut v.row, len, ListRef::Settings, hits, t, focused, &|i, w| setting_line(&lines[i], w, t));
}

/// One settings page line.
fn setting_line(line: &SLine<'_>, w: usize, t: &Theme) -> Line<'static> {
    let row = match line {
        SLine::Group(title, summary) => return Line::from(vec![Span::styled(title.to_string(), bold().fg(t.ink)), Span::styled(format!("   {summary}"), t.dim())]),
        SLine::Title(title) => return Line::from(Span::styled(format!("  {}", title.to_uppercase()), t.dim())),
        SLine::Row(r) => *r,
    };
    let (title, detail) = settings_view::row_words(row);
    let live = settings_view::row_enabled(row);
    let text_style = if live { Style::default().fg(t.ink) } else { t.dim() };
    // A season: its kanji in its accent, then its name; the one worn ticked, its name in ink.
    if let settings_view::Row::Season { kanji, name, accent, worn, .. } = row {
        return Line::from(vec![
            Span::styled(if *worn { "  ✓ " } else { "    " }, t.current()),
            Span::styled(*kanji, bold().fg(crate::art::argb(*accent))),
            Span::styled(format!("  {name}"), if *worn { bold().fg(t.ink) } else { t.dim() }),
        ]);
    }
    // A few options, all on view: the chosen one in ink, underlined.
    if let settings_view::Row::Choice { options, shown, chips: true, .. } = row {
        let mut spans = vec![Span::styled(format!("    {}", pad(&title, 24)), text_style)];
        for (label, _) in options {
            let style = if label == shown && live { bold().fg(t.ink).add_modifier(Modifier::UNDERLINED) } else { t.dim() };
            spans.push(Span::styled(label.clone(), style));
            spans.push(Span::raw("  "));
        }
        return Line::from(spans);
    }
    if let Some((share, centred)) = settings_view::slider_share(row) {
        let bar_w = (w / 3).clamp(8, 30);
        let at = (share * (bar_w - 1) as f32).round() as usize;
        let mut bar = String::with_capacity(bar_w * 3);
        for i in 0..bar_w {
            bar.push(if i == at {
                '●'
            } else if centred && i == bar_w / 2 {
                '┼'
            } else if i < at {
                '━'
            } else {
                '─'
            });
        }
        bar.push(' ');
        return spread(vec![Span::styled(format!("    {title}"), text_style)], Span::styled(bar, Style::default().fg(t.ink)), w);
    }
    if title.is_empty() && !detail.is_empty() {
        return Line::from(Span::styled(fit(&format!("    {detail}"), w).into_owned(), t.dim().add_modifier(Modifier::ITALIC)));
    }
    let value = match row {
        settings_view::Row::Toggle { on, .. } | settings_view::Row::Ranked { on, .. } | settings_view::Row::Plugin { on, .. } => crate::text::on_off(*on).to_string(),
        _ => settings_view::row_value(row),
    };
    let mut left = vec![Span::styled(format!("    {title}"), text_style)];
    if !detail.is_empty() {
        left.push(Span::styled(format!("  {detail}"), t.dim()));
    }
    let switched = matches!(row, settings_view::Row::Toggle { on: true, .. } | settings_view::Row::Ranked { on: true, .. } | settings_view::Row::Plugin { on: true, .. });
    let value_style = if !live { t.dim() } else if switched { Style::default().fg(t.on) } else { Style::default().fg(t.ink) };
    spread(left, Span::styled(value + " ", value_style), w)
}

/// Where the music plays: this computer first, then the account's other devices with what each plays,
/// the one playing ticked; then the jam.
fn devices(f: &mut Frame, area: Rect, app: &mut App, focused: bool, t: &Theme) {
    let rows = app.devices.rows();
    let body = Rect { height: area.height.min(rows.len() as u16), ..area };
    let App { devices, hits, .. } = app;
    let d = &*devices;
    let active = d.active.as_ref().map(|(id, _)| id.as_str());
    let tick = |on: bool| Span::styled(if on { "✓ " } else { "  " }, t.current());
    let name = |s: &str, on: bool| Span::styled(s.to_string(), if on { t.current() } else { bold().fg(t.ink) });
    let listeners = d.listeners().len();
    let mut sel = d.sel;
    list(f, body, &mut sel, rows.len(), ListRef::Devices, hits, t, focused, &|i, w| match rows[i] {
        DeviceRow::Here => {
            let here = active.is_none() && d.cast.is_none();
            Line::from(vec![tick(here), name(crate::text::THIS_COMPUTER, here)])
        }
        DeviceRow::Cast => {
            let c = d.cast.as_ref().expect("a cast row has its cast");
            spread(vec![tick(true), name(&c.device, true)], Span::styled(crate::text::cast_line(c), t.dim()), w)
        }
        DeviceRow::Device(k) => {
            let x = &d.list[k];
            let on = active == Some(x.id.as_str());
            spread(vec![tick(on), name(&x.name, on)], Span::styled(crate::text::device_kind(x.kind), t.dim()), w)
        }
        DeviceRow::Playing(k) => {
            let x = &d.list[k];
            Line::from(Span::styled(fit(&format!("  {}", crate::text::device_line(x)), w).into_owned(), if x.refused.is_some() { Style::default().fg(Color::LightRed) } else { t.dim() }))
        }
        DeviceRow::Jam if d.jam.is_some() => spread(vec![Span::raw("  "), name(&crate::text::jam_strip(listeners), true)], Span::styled(crate::text::JAM_INVITE, t.dim()), w),
        DeviceRow::Jam => Line::from(vec![Span::raw("  "), Span::styled(crate::text::JAM_START, Style::default().fg(t.ink))]),
        DeviceRow::Join => Line::from(vec![Span::raw("  "), Span::styled(crate::text::JAM_JOIN, Style::default().fg(t.ink))]),
        DeviceRow::Pair => Line::from(format!("  {}", crate::text::PAIR_DEVICES)),
        DeviceRow::ResetPairing => Line::from(format!("  {}", crate::text::FORGET_DEVICES)),
        DeviceRow::Controlled(k) => {
            let x = &d.controlled[k];
            spread(vec![Span::raw("  "), name(&x.name, false)], Span::styled(crate::text::battery(&x.battery), t.dim()), w)
        }
        DeviceRow::Noise(k) => {
            let x = &d.controlled[k];
            spread(vec![Span::raw("  "), Span::styled(crate::text::NOISE_CONTROL, Style::default().fg(t.ink))], Span::styled(x.noise.map_or("", crate::text::noise), t.dim()), w)
        }
        DeviceRow::PluginName(c) => Line::from(Span::styled(format!("  {}", d.plugins[c].name.to_uppercase()), t.dim())),
        DeviceRow::Plugin(c, r) => d.plugin_row(c, r).map_or_else(Line::default, |row| setting_line(&SLine::Row(&row), w, t)),
        DeviceRow::PluginHint(c, r) => Line::from(Span::styled(fit(&format!("    {}", d.plugins[c].rows[r].hint.as_deref().unwrap_or_default()), w).into_owned(), t.dim().add_modifier(Modifier::ITALIC))),
    });
    devices.sel = sel;
    let mut y = body.y + body.height + 1;
    let notes = [(devices.jams_unsupported, crate::text::JAM_UNSUPPORTED)];
    for (_, note) in notes.into_iter().filter(|(on, _)| *on) {
        for l in wrap(note, area.width as usize) {
            if y >= area.y + area.height {
                return;
            }
            text(f, Rect { y, height: 1, ..area }, &l, t.dim());
            y += 1;
        }
        y += 1;
    }
}

/// The current mix and the planned transition.
fn mix_lines(app: &App, w: usize, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // While mixing, `song` is the louder of the two.
    if app.now.mixing {
        let from = app.mixed_in.as_ref().and_then(|n| crate::backend::app().song(&n.outgoing_id)).map(|s| s.title);
        let heard = match &from {
            Some(from) => format!("Mixing in from “{from}”"),
            None => "Mixing into the next".to_string(),
        };
        lines.push(Line::from(Span::styled(fit(&heard, w).into_owned(), Style::default().fg(t.ink))));
        if let Some(n) = &app.mixed_in {
            lines.push(Line::from(Span::styled(fit(&format!("  {}", how_mixed(n)), w).into_owned(), t.dim())));
        }
    }
    if let Some(n) = &app.transition {
        let next = crate::backend::app().song(&n.incoming_id).map(|s| s.title).unwrap_or_default();
        lines.push(Line::from(Span::styled(fit(&format!("Next: “{next}”"), w).into_owned(), Style::default().fg(t.ink))));
        lines.push(Line::from(Span::styled(fit(&format!("  {}", how_mixed(n)), w).into_owned(), t.dim())));
        if !n.reason.is_empty() {
            lines.push(Line::from(Span::styled(fit(&format!("  {}", n.reason), w).into_owned(), t.dim())));
        }
    } else if app.prefs.auto_mix || app.prefs.crossfade_sec > 0 {
        let what = if app.prefs.auto_mix { "AutoMix" } else { "Crossfade" };
        lines.push(Line::from(Span::styled(fit(&format!("{what} on · planned as this plays"), w).into_owned(), t.dim())));
    }
    lines
}

/// A transition in words: kind, length, start and tempo ratio.
fn how_mixed(n: &nori_core::automix::planner::TransitionNote) -> String {
    if n.duration_ms <= 0 {
        return "Gapless".to_string();
    }
    let tempo = if (n.tempo_ratio - 1.0).abs() > 0.001 { format!(", tempo ×{:.3}", n.tempo_ratio) } else { String::new() };
    format!("{}, {:.1} s from {}{tempo}", words_kind(n.kind), n.duration_ms as f32 / 1000.0, clock(n.start_ms))
}

/// A planner transition kind in words.
fn words_kind(kind: nori_core::automix::planner::TransitionKind) -> &'static str {
    use nori_core::automix::planner::TransitionKind as K;
    match kind {
        K::BeatMatched => "AutoMix: beat-matched mix",
        K::EchoOut => "AutoMix: echo out",
        K::MixRampFade => "AutoMix: fade",
        K::EqualPowerFade => "Crossfade",
        K::Gapless => "Gapless",
    }
}

/// The queue: its head with how much is left and shuffle and repeat, then the jam's rows and the songs.
fn queue(f: &mut Frame, area: Rect, app: &mut App, focused: bool, t: &Theme) {
    let rows = app.queue_rows();
    if rows.is_empty() {
        let lines = vec![Line::from(Span::styled("The queue is empty", bold())), Line::from(Span::styled("enter plays, a adds to the queue", t.dim()))];
        put(f, Paragraph::new(lines), Rect { height: area.height.min(2), ..area });
        return;
    }
    if let Some(q) = &app.queue {
        let secs: u64 = q.songs.iter().map(|s| s.duration as u64).sum();
        let head = row_of(area, 0);
        let left = vec![Span::styled(crate::text::UP_NEXT, bold().fg(t.ink)), Span::styled(format!("  {}", crate::text::songs_caption(q.len, secs)), t.dim())];
        put(f, Paragraph::new(Line::from(left)), head);
        // Shuffle and repeat are the queue's, the host's in a jam.
        if !app.guest() {
            let (shuffle, repeat) = (q.shuffle, q.repeat);
            let lit = |on: bool| if on { t.current() } else { t.dim() };
            let words = [
                (Span::styled(crate::text::SHUFFLE, lit(shuffle)), Button::Shuffle),
                (Span::styled(crate::text::repeat_word(repeat), lit(repeat != nori_core::rules::REPEAT_OFF)), Button::Repeat),
            ];
            let w: u16 = words.iter().map(|(s, _)| s.width() as u16 + 3).sum::<u16>() - 3;
            buttons(f, &mut app.hits, Rect { x: head.x + head.width.saturating_sub(w + 1), width: w, ..head }, &words);
        }
    }
    let body = below(area, 2);
    let listeners: Vec<String> = app.devices.listeners().iter().map(|m| crate::text::jam_member(&m.name, m.role)).collect();
    let note = app.downloads_note();
    let along = app.along_note();
    let chosen = app.queue_sel.at;
    let App { queue, queue_sel, hits, devices, .. } = app;
    let current = queue.as_ref().map_or(-1, |q| q.index);
    let asks = devices.asks();
    let hosting = devices.jam.as_ref().is_some_and(|j| j.hosting);
    let strip = devices.jam_strip().unwrap_or_default();
    let here = devices.listening() != nori_core::remote::Listening::Watching;
    let playing_here = devices.listening() == nori_core::remote::Listening::Playing;
    let cur = t.current();
    list(f, body, queue_sel, rows.len(), ListRef::Queue, hits, t, focused, &|row, w| match rows[row] {
        QueueRow::Jam => spread(vec![Span::styled(strip.clone(), cur)], Span::styled(if hosting { crate::text::JAM_INVITE } else { "" }, t.dim()), w),
        QueueRow::Listeners => Line::from(Span::styled(fit(&format!("  {}", listeners.join(", ")), w).into_owned(), Style::default().fg(t.ink))),
        QueueRow::Listen => {
            // Asked for, it is lit whether or not the host lets it play (the note under it says why not).
            let words = if playing_here { crate::text::PLAYING_HERE } else { crate::text::LISTEN_HERE };
            let (keys, style) = if here { (crate::text::STOP_KEYS, cur) } else { (crate::text::LISTEN_KEYS, Style::default().fg(t.ink)) };
            spread(vec![Span::raw("  "), Span::styled(words, style)], Span::styled(if row == chosen { keys } else { "" }, t.dim()), w)
        }
        QueueRow::AlongNote(n) => Line::from(Span::styled(format!("{:>w$}{}", "", along[n], w = crate::app::QUEUE_NOTE_INDENT), t.dim())),
        QueueRow::Asking => Line::from(Span::styled(format!("  {}", crate::text::YOU_ASKED), t.dim().add_modifier(Modifier::BOLD))),
        // The keys show on the request chosen.
        QueueRow::Ask(k) => {
            let p = asks[k];
            let keys = if hosting && rows.get(chosen).and_then(|r| r.ask()) == Some(k) { crate::text::JAM_DECIDE } else { "" };
            let who = if hosting { crate::text::jam_asked(&p.from_name) } else { crate::text::jam_waiting(devices.host()) };
            let asked = vec![Span::raw("  "), Span::styled(p.song.title.clone(), Style::default().fg(t.ink)), Span::styled(who, t.dim())];
            spread(asked, Span::styled(keys, t.dim()), w)
        }
        QueueRow::Downloads(_, n) => Line::from(Span::styled(format!("{:>w$}{}", "", note[n], w = crate::app::QUEUE_NOTE_INDENT), t.dim())),
        QueueRow::End => Line::from(Span::styled(format!("  {}", if hosting { crate::text::JAM_END } else { crate::text::JAM_LEAVE }), t.dim())),
        QueueRow::Song(i) => {
            let Some(s) = queue.as_ref().and_then(|q| q.songs.get(i)) else { return Line::from("") };
            let playing = i as i32 == current;
            let mark = if playing { "▶ " } else { "  " };
            let title_style = if playing { t.current() } else { Style::default().fg(t.ink) };
            // Who asked for it before the artist, which is cut first.
            let by = devices.added.get(&s.id).map(|by| Span::styled(format!(" · {by}"), t.dim()));
            let spans = [Span::styled(mark, cur), Span::styled(s.title.clone(), title_style)].into_iter().chain(by).chain([Span::styled(format!("  {}", s.artist), t.dim())]);
            spread(spans.collect(), Span::styled(clock(s.duration as i64 * 1000), t.dim()), w)
        }
    });
}

/// `s` word-wrapped to `w` columns.
pub(crate) fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        let ww = word.width();
        if !line.is_empty() && line.width() + 1 + ww > w {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        while line.width() > w {
            let cut = fit(&line, w).trim_end_matches('…').to_string();
            let rest = line[cut.len()..].to_string();
            if cut.is_empty() {
                break;
            }
            out.push(cut);
            line = rest;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

fn lyrics(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    let Some(l) = &app.lyrics else {
        let text = match (&app.song, &app.lyrics_for) {
            (None, _) => "Nothing playing",
            (Some(_), Some(_)) => "Looking for lyrics…",
            _ => "No lyrics",
        };
        return empty(f, area, t, text);
    };
    if l.pick.lyrics.lines.is_empty() {
        return empty(f, area, t, "No lyrics for this song");
    }
    let frame = l.clock.shown();
    let synced = l.pick.lyrics.synced;
    let sweep = app.prefs.lyrics_sweep && l.clock.timing().sweeps();
    let translate = app.prefs.lyrics_translation;
    let active = frame.active;
    // After the last line `active` is past the end; stay on the last line.
    let focus = app.lyrics_sel.unwrap_or((active.max(0) as usize).min(l.pick.lyrics.lines.len().saturating_sub(1)));
    let credit = l.credit();
    let body = if credit.is_some() { Rect { height: area.height.saturating_sub(1), ..area } } else { area };
    let w = body.width as usize;
    // A lyric line's rows: text (0), backing vocals (1), translation (2), each wrapped.
    let block = |i: usize| -> Vec<(String, u8)> {
        let line = &l.pick.lyrics.lines[i];
        let mut v: Vec<(String, u8)> = wrap(&line.text, w.saturating_sub(2)).into_iter().map(|s| (s, 0)).collect();
        if !line.backing.is_empty() {
            v.extend(wrap(&format!("({})", line.backing), w.saturating_sub(2)).into_iter().map(|s| (s, 1)));
        }
        if translate {
            if let Some(tr) = line.translation.as_ref().filter(|x| !x.is_empty()) {
                v.extend(wrap(tr, w.saturating_sub(2)).into_iter().map(|s| (s, 2)));
            }
        }
        v
    };
    // The focused line sits a third of the way down, as on Android.
    let anchor = body.y + body.height / 3;
    let mut y = anchor as i32;
    let mut i = focus as i32;
    while i > 0 && y > body.y as i32 {
        i -= 1;
        y -= block(i as usize).len() as i32 + 1;
    }
    app.hits.push((body, Hit::List(ListRef::Lyrics)));
    let mut at = y;
    for n in (i.max(0) as usize)..l.pick.lyrics.lines.len() {
        if at >= (body.y + body.height) as i32 {
            break;
        }
        let strength = nori_look::lyrics::line_strength(synced, n as i32, active);
        let lit = t.lit(strength);
        let is_active = synced && n as i32 == active;
        let mut sung = if is_active && sweep { l.sung_chars(n, frame.sung) } else { 0 };
        for (part, kind) in block(n) {
            let spans = match kind {
                0 if is_active && sweep => {
                    let chars = part.chars().count();
                    let cut_at = sung.min(chars);
                    sung = sung.saturating_sub(chars + 1);
                    let cut = part.char_indices().nth(cut_at).map_or(part.len(), |(b, _)| b);
                    vec![
                        Span::styled(part[..cut].to_string(), t.current()),
                        Span::styled(part[cut..].to_string(), Style::default().fg(t.lit(nori_look::lyrics::UNSUNG)).add_modifier(Modifier::BOLD)),
                    ]
                }
                0 if is_active => vec![Span::styled(part, t.current())],
                0 => vec![Span::styled(part, Style::default().fg(lit))],
                1 => vec![Span::styled(part, Style::default().fg(t.lit(strength * 0.7)).add_modifier(Modifier::ITALIC))],
                _ => vec![Span::styled(part, Style::default().fg(t.lit(strength * 0.8)))],
            };
            let mut row = Line::from(spans);
            if app.lyrics_sel == Some(n) {
                row = row.style(Style::default().add_modifier(Modifier::UNDERLINED));
            }
            if at >= body.y as i32 && at < (body.y + body.height) as i32 {
                let r = Rect { x: body.x, y: at as u16, width: body.width, height: 1 };
                put(f, Paragraph::new(row), r);
                app.hits.push((r, Hit::Row(ListRef::Lyrics, n)));
            }
            at += 1;
        }
        at += 1;
    }
    if let Some(c) = credit {
        text(f, Rect { y: area.y + area.height - 1, height: 1, ..area }, &c, t.dim());
    }
}

// ---- the full player ----

/// The full-window player: the panels' names along the top, then the cover, the title with its seal, the
/// seek bar, the transport and where the music plays on the left, and the open panel beside it. A narrow
/// window gives the panel the room under the song; Playing is the song alone.
fn full_player(f: &mut Frame, area: Rect, app: &mut App, art: Option<&mut Art>) {
    let t = app.theme;
    put(f, Block::default().style(t.ground()), area);
    if area.width < 10 || area.height < 4 {
        return;
    }
    let inner = Rect { x: area.x + 1, width: area.width - 2, ..area };
    panel_tabs(f, app, &t, row_of(inner, 0));
    let inner = below(inner, 2);
    if app.panel == Panel::Playing {
        let w = (inner.height.saturating_sub(SONG_BELOW).min(22) * 2).max(40).min(inner.width);
        let column = Rect { x: inner.x + (inner.width - w) / 2, width: w, ..inner };
        song_column(f, column, app, &t, art);
        return;
    }
    let wide = inner.width >= 84;
    let left_w = if wide { (inner.width * 2 / 5).clamp(32, 46) } else { inner.width };
    let left = Rect { width: left_w, ..inner };
    let used = song_column(f, left, app, &t, art);
    let panel = if wide { Rect { x: left.x + left.width + 4, width: inner.width.saturating_sub(left.width + 4), ..inner } } else { below(inner, used + 1) };
    if panel.height >= 3 && panel.width >= 20 {
        app.shown.panel_w = panel.width;
        let focused = app.focus == Focus::Panel;
        match app.panel {
            Panel::Lyrics => lyrics(f, panel, app, &t),
            Panel::Queue => queue(f, panel, app, focused, &t),
            Panel::Devices => devices(f, panel, app, focused, &t),
            Panel::Playing => {}
        }
    }
}

/// Rows under the full player's cover: title, artist, quality, a blank, seek, a blank, transport, where it
/// plays, volume.
const SONG_BELOW: u16 = 9;

/// The song in the full player's left column: returns the rows it took.
fn song_column(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, art: Option<&mut Art>) -> u16 {
    let Some(song) = app.song.clone() else {
        text(f, area, "Nothing playing · esc goes back", t.dim());
        return 1;
    };
    let info: Vec<String> = app.slots.info_of(&song.id).iter().flat_map(|g| g.lines.iter().map(|l| crate::text::info_line(&g.name, l))).collect();
    let status = app.slots.status.clone();
    let below = SONG_BELOW + (info.len() + status.len()) as u16;
    let cover_rows = if app.images && art.is_some() { area.height.saturating_sub(below).min(area.width / 2).min(22) } else { 0 };
    let mut y = area.y;
    if cover_rows >= 6 {
        cover(f, Rect { x: area.x, y, width: cover_rows * 2, height: cover_rows }, art, song.cover_art.as_deref());
        y += cover_rows + 1;
    }
    let w = area.width;
    let row = |y: u16| Rect { y, height: 1, ..area };
    // The title with the seal at the right end.
    let sealed = app.rules.stars;
    text(f, Rect { width: w.saturating_sub(if sealed { 4 } else { 0 }), ..row(y) }, &song.title, bold().fg(t.ink));
    if sealed {
        let liked = app.starred(Starrable::Song, &song.id, song.starred);
        let r = Rect { x: area.x + w - 2, width: 2, ..row(y) };
        let flash = liked && app.flash.is_some_and(|(b, _)| b == Button::StarSong);
        put(f, Paragraph::new(seal(liked, flash, Style::default().fg(t.ink), t)), r);
        app.hits.push((Rect { x: r.x - 1, width: 3, ..r }, Hit::Button(Button::StarSong)));
    }
    y += 1;
    let by = [song.artist.as_str(), song.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    text(f, row(y), &by, Style::default().fg(t.ink));
    y += 1;
    if let Some(q) = crate::text::quality(&song) {
        text(f, row(y), &q, t.dim());
        y += 1;
    }
    for l in &info {
        text(f, row(y), l, t.dim());
        y += 1;
    }
    y += 1;
    let time = 5;
    let track_w = w.saturating_sub(2 * (time + 1)).min(60);
    let x = area.x;
    seek_line(f, app, t, [Rect { x, width: time, ..row(y) }, Rect { x: x + time + 1, width: track_w, ..row(y) }, Rect { x: x + time + 2 + track_w, width: time, ..row(y) }]);
    y += 2;
    let buttons = offered_buttons(app);
    let step = 6;
    let span = step * buttons.len().saturating_sub(1) as u16 + 1;
    let x0 = area.x + (time + 1 + track_w.min(w) / 2).saturating_sub(span / 2);
    for (i, b) in buttons.iter().enumerate() {
        transport_button(f, app, t, *b, Rect { x: x0 + i as u16 * step, width: 1, ..row(y) });
    }
    y += 2;
    output_line(f, row(y), app, t);
    y += 1;
    for l in &status {
        text(f, row(y), l, t.dim());
        y += 1;
    }
    volume_line(f, Rect { width: 16.min(w), ..row(y) }, app, t);
    y += 2;
    // The mix being heard or planned, where there is room.
    let lines = mix_lines(app, w as usize, t);
    let room = (area.y + area.height).saturating_sub(y);
    let n = (lines.len() as u16).min(room);
    if n > 0 {
        put(f, Paragraph::new(lines), Rect { y, height: n, ..area });
        y += n;
    }
    y - area.y
}

/// The full player's panels by name, the open one in the accent: the devices only while other devices can
/// play; on the right, the way back.
fn panel_tabs(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let mut x = r.x;
    for (which, name) in PANELS.iter().filter(|(p, _)| *p != Panel::Devices || app.devices.shown()) {
        let w = name.width() as u16;
        if x + w > r.x + r.width {
            break;
        }
        let at = Rect { x, width: w, ..r };
        put(f, Paragraph::new(Span::styled(*name, if *which == app.panel { t.current() } else { t.dim() })), at);
        app.hits.push((at, Hit::Button(Button::Panel(*which))));
        x += w + 2;
    }
    let back = crate::text::BACK;
    let w = back.width() as u16;
    if x + w + 2 <= r.x + r.width {
        let at = Rect { x: r.x + r.width - w, width: w, ..r };
        put(f, Paragraph::new(Span::styled(back, t.dim())), at);
        app.hits.push((at, Hit::Button(Button::Full)));
    }
}

/// Where the music plays: another device by name, in bold ink, or this computer's output.
fn output_line(f: &mut Frame, r: Rect, app: &mut App, t: &Theme) {
    let (name, lit) = match &app.devices.active {
        Some((_, name)) => (crate::text::playing_on(name), true),
        // Headphones a plugin controls by their own name, with their battery.
        None => match app.devices.output_device(&app.output) {
            Some(d) => (d.level.map_or_else(|| d.name.clone(), |l| crate::text::with_level(&d.name, l)), false),
            None => (crate::text::output_name(&app.output), false),
        },
    };
    text(f, r, &name, if lit { bold().fg(t.ink) } else { t.dim() });
    if app.devices.shown() {
        app.hits.push((Rect { width: (name.width() as u16).min(r.width), ..r }, Hit::Button(Button::Panel(Panel::Devices))));
    }
}

/// The volume: its word and a bar the mouse sets.
fn volume_line(f: &mut Frame, r: Rect, app: &mut App, t: &Theme) {
    let word = crate::text::VOLUME;
    let w = word.width() as u16 + 1;
    text(f, Rect { width: w, ..r }, word, t.dim());
    let track = Rect { x: r.x + w, width: r.width.saturating_sub(w), ..r };
    track_bar(f, track, app.volume, t);
    app.volume_rect = track;
    app.hits.push((track, Hit::Volume));
}

// ---- the song playing ----

/// The seal: not liked the symbol in quiet ink, liked the symbol on the accent (`flash`: the one frame of
/// the stamp, inverted). Two cells wide.
/// The seal, `idle` while the song or page is not a favorite: the ink of the row it sits in.
pub fn seal(liked: bool, flash: bool, idle: Style, t: &Theme) -> Span<'static> {
    let style = if liked { t.liked() } else { idle };
    let style = if flash { style.add_modifier(Modifier::REVERSED) } else { style };
    Span::styled("好", style)
}

/// The song playing, along the bottom on the raised paper: play or pause, the song, shuffle and repeat
/// while on and the seal; under them its clock, where it plays and the volume. A click on the song opens
/// the full player.
fn now_playing(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    put(f, Block::default().style(t.raised()), area);
    let bar = crate::chrome::now(area);
    // A jam guest's play is its own listening, with or without a song yet.
    if (app.song.is_some() || app.devices.controls.is_some()) && offered_buttons(app).contains(&Button::Toggle) {
        transport_button(f, app, t, Button::Toggle, bar.toggle);
    }
    let song = app.song.clone();
    // Right-aligned on the song's line: what is on, then the seal.
    let mut marks: Vec<(Span<'static>, Button)> = Vec::new();
    if let Some(strip) = app.devices.jam_strip() {
        marks.push((Span::styled(strip, t.current()), Button::Panel(if app.guest() { Panel::Queue } else { Panel::Devices })));
    }
    if app.queue_shuffled() {
        marks.push((Span::styled(crate::text::SHUFFLE, t.current()), Button::Shuffle));
    }
    if app.repeat() != nori_core::rules::REPEAT_OFF {
        marks.push((Span::styled(crate::text::repeat_word(app.repeat()), t.current()), Button::Repeat));
    }
    if let Some(s) = song.as_ref().filter(|_| app.rules.stars) {
        let liked = app.starred(Starrable::Song, &s.id, s.starred);
        marks.push((seal(liked, liked && app.flash.is_some_and(|(b, _)| b == Button::StarSong), t.dim(), t), Button::StarSong));
    }
    let w: u16 = marks.iter().map(|(s, _)| s.width() as u16 + 3).sum::<u16>().saturating_sub(3);
    let at = Rect { x: bar.marks.x + bar.marks.width.saturating_sub(w), width: w.min(bar.marks.width), ..bar.marks };
    buttons(f, &mut app.hits, at, &marks);
    // The song takes what the marks leave.
    let song_r = Rect { width: at.x.saturating_sub(bar.song.x + 2), ..bar.song };
    match &song {
        Some(s) => {
            let by = [s.artist.as_str(), s.album.as_str()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
            let line = Line::from(vec![Span::styled(s.title.clone(), bold().fg(t.ink)), Span::styled(format!("  {by}"), t.dim())]);
            put(f, Paragraph::new(spread(line.spans, Span::raw(""), song_r.width as usize)), song_r);
            app.hits.push((song_r, Hit::Button(Button::Full)));
        }
        // Where it plays while another device holds the music.
        None => {
            let said = app.devices.active.as_ref().map_or_else(|| crate::text::NOTHING_PLAYING.to_string(), |(_, name)| crate::text::playing_on(name));
            text(f, song_r, &said, t.dim());
        }
    }
    if song.is_none() {
        return;
    }
    if let Some(clock) = bar.clock {
        seek_line(f, app, t, clock);
    }
    if let Some(r) = bar.output {
        output_line(f, r, app, t);
    }
    if let Some(r) = bar.volume {
        volume_line(f, r, app, t);
    }
}

/// The transport buttons the player offers, in order. A jam guest's are those its role offers.
fn offered_buttons(app: &App) -> Vec<Button> {
    let offered = |b: Button| match b {
        Button::Toggle => app.offers(|c| c.play_pause),
        _ => app.offers(|c| c.skip),
    };
    [Button::Previous, Button::Toggle, Button::Next].into_iter().filter(|b| offered(*b)).collect()
}

/// A transport glyph at `r`: bare, reversed for a moment after a press.
fn transport_button(f: &mut Frame, app: &mut App, t: &Theme, b: Button, r: Rect) {
    let playing = app.devices.controls.map_or(app.now.state == State::Playing, |c| c.playing);
    let glyph = match b {
        Button::Previous => "⏮",
        Button::Toggle if app.now.buffering => "⋯",
        Button::Toggle if playing => "⏸",
        Button::Toggle => "▶",
        _ => "⏭",
    };
    let style = Style::default().fg(t.ink).add_modifier(Modifier::BOLD);
    let style = if app.flash.is_some_and(|(p, _)| p == b) { style.add_modifier(Modifier::REVERSED) } else { style };
    let r = Rect { width: glyph.width() as u16, height: 1, ..r };
    put(f, Paragraph::new(Span::styled(glyph, style)), r);
    app.hits.push((Rect { x: r.x.saturating_sub(1), width: r.width + 2, ..r }, Hit::Button(b)));
}

/// The elapsed time, the seek bar and the length in `[a, track, b]`; the bar takes the seek clicks.
fn seek_line(f: &mut Frame, app: &mut App, t: &Theme, [a, track, b]: [Rect; 3]) {
    let len = app.song.as_ref().map_or(0, |s| s.duration as i64 * 1000);
    let pos = match app.drag {
        Some(crate::app::Drag::Seek(share)) => (share * len as f32) as i64,
        _ => app.now.position(Instant::now()).clamp(0, len.max(0)),
    };
    app.shown.clock = true;
    text(f, a, &format!("{:>5}", clock(pos)), t.dim());
    text(f, b, &clock(len), t.dim());
    track_bar(f, track, if len > 0 { pos as f32 / len as f32 } else { 0.0 }, t);
    app.seek_rect = track;
    if app.offers(|c| c.seek) {
        app.hits.push((track, Hit::Seek));
    }
}

/// A bar of `share` in `r`: heavy in ink up to it, a hairline after.
fn track_bar(f: &mut Frame, r: Rect, share: f32, t: &Theme) {
    let filled = ((share.clamp(0.0, 1.0) * r.width as f32).round() as usize).min(r.width as usize);
    let line = Line::from(vec![Span::styled("━".repeat(filled), Style::default().fg(t.ink)), Span::styled("─".repeat(r.width as usize - filled), Style::default().fg(t.hair))]);
    put(f, Paragraph::new(line), r);
}

// ---- the login and what goes on top ----

fn login(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    let servers = app.prefs.servers.len() as u16;
    let kinds = app.login_kinds();
    let fields = app.login_fields();
    let info = kinds.get(app.login.kind).or(kinds.first());
    let words = info.map(|i| app.provider_words(&i.kind));
    let choosing = kinds.len() > 1;
    let h = 11 + 2 * fields.len() as u16 + if choosing { 2 } else { 0 } + if servers > 0 { servers + 2 } else { 0 };
    let r = centred(area, 64, h);
    let inner = popup(f, r, Span::styled(" nori · add a profile ", bold().fg(t.ink)), t);
    let l = &app.login;
    let bottom = inner.y + inner.height;
    let mut y = inner.y + 1;
    if choosing {
        let name = words.as_ref().map_or(String::new(), |w| w.name.clone());
        let line = Line::from(vec![Span::styled(format!("  {:<16}", crate::text::LOGIN_KIND), t.dim()), Span::styled(format!("‹ {name} ›"), bold().fg(t.ink))]);
        put(f, Paragraph::new(line), Rect { x: inner.x, y, width: inner.width, height: 1 });
        y += 2;
    }
    let about = match info.map(|i| &i.kind) {
        Some(nori_core::settings::ServerKind::Plugin { .. }) => crate::text::FROM_A_PLUGIN,
        _ => "Navidrome, octo-fiesta or any Subsonic server",
    };
    text(f, Rect { x: inner.x + 2, y, width: inner.width.saturating_sub(4), height: 1 }, about, t.dim());
    y += 2;
    for &i in &fields {
        if y >= bottom {
            break;
        }
        let label = LOGIN_FIELDS[i];
        let focused = l.focus == i && !l.on_list;
        let value = if i == 3 { "•".repeat(l.fields[i].chars().count()) } else { l.fields[i].clone() };
        let cursor = if focused { "▏" } else { "" };
        let line = Line::from(vec![
            Span::styled(format!("  {label:<16}"), if focused { t.current() } else { t.dim() }),
            Span::styled(format!("{value}{cursor}"), Style::default().fg(t.ink)),
        ]);
        let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
        put(f, Paragraph::new(line), row);
        app.hits.push((row, Hit::LoginField(i)));
        y += 2;
    }
    let button = if l.busy { " Connecting… " } else { " Connect " };
    let br = Rect { x: inner.x + 18, y, width: button.width() as u16, height: 1 };
    put(f, Paragraph::new(Span::styled(button, t.select(true))), br);
    app.hits.push((br, Hit::Button(Button::Connect)));
    let plugins = format!(" {} › ", crate::text::PLUGINS);
    let pr = Rect { x: br.x + br.width + 2, y, width: plugins.width() as u16, height: 1 };
    put(f, Paragraph::new(Span::styled(plugins, t.dim())), pr);
    app.hits.push((pr, Hit::Button(Button::LoginPlugins)));
    y += 2;
    let msg = match &l.error {
        Some(e) => Span::styled(format!("  {e}"), Style::default().fg(Color::LightRed)),
        None => Span::styled(format!("  {}", if choosing { crate::text::LOGIN_KIND_KEYS } else { crate::text::LOGIN_KEYS }), t.dim()),
    };
    put(f, Paragraph::new(msg).wrap(ratatui::widgets::Wrap { trim: false }), Rect { x: inner.x, y, width: inner.width, height: 2.min(bottom.saturating_sub(y)) });
    y += 3;
    if servers > 0 && y + 1 < bottom {
        text(f, Rect { x: inner.x + 2, y, width: inner.width.saturating_sub(2), height: 1 }, "Or use a saved server", bold().fg(t.ink));
        y += 1;
        let area = Rect { x: inner.x + 2, y, width: inner.width.saturating_sub(4), height: servers.min(bottom - y) };
        let App { login, prefs, hits, .. } = app;
        let names: Vec<String> = prefs.servers.iter().map(|s| format!("{}  {}", nori_core::settings::label(&s.name, &s.url), s.user)).collect();
        let focused = login.on_list;
        list(f, area, &mut login.sel, names.len(), ListRef::Profiles, hits, t, focused, &|i, w| Line::from(fit(&names[i], w).into_owned()));
    }
}

/// Light modules around the code, so a camera finds its edges on a dark terminal.
const QR_QUIET: usize = 2;

/// A QR code in half blocks: each cell two modules, one above the other, black on white whatever the
/// terminal's colours, in a light quiet zone.
fn qr_lines(code: &nori_core::remote::QrCode) -> Vec<Line<'static>> {
    let size = code.size as usize;
    let n = size + 2 * QR_QUIET;
    let dark = |x: usize, y: usize| {
        let (x, y) = (x.wrapping_sub(QR_QUIET), y.wrapping_sub(QR_QUIET));
        x < size && y < size && code.dark[y * size + x]
    };
    let colour = |d: bool| if d { Color::Rgb(0, 0, 0) } else { Color::Rgb(255, 255, 255) };
    (0..n.div_ceil(2)).map(|r| Line::from((0..n).map(|x| Span::styled("▀", Style::default().fg(colour(dark(x, 2 * r))).bg(colour(dark(x, 2 * r + 1))))).collect::<Vec<_>>())).collect()
}

fn overlay(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    let missing = app.page_missing();
    let App { overlay, hits, devices, settings, .. } = app;
    let kinds = &*settings.own.kinds;
    let Some(o) = overlay else { return };
    match o {
        Overlay::Help { scroll } => {
            let r = centred(area, 88, area.height.saturating_sub(4));
            let inner = popup(f, r, Span::styled(" Keys · any key closes ", bold().fg(t.ink)), t);
            let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
            let mut lines = Vec::new();
            for scope in Scope::ALL {
                lines.push(Line::from(Span::styled(scope.title(), bold().fg(t.ink))));
                for b in BINDINGS.iter().filter(|b| b.scope == scope && !b.label.is_empty()) {
                    lines.push(Line::from(vec![Span::styled(format!("  {:<22}", b.label), Style::default().fg(t.ink)), Span::styled(b.help, t.dim())]));
                }
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled("The mouse: click the tabs, rows and words; click or drag the seek bar and the volume; the wheel scrolls.", t.dim())));
            *scroll = (*scroll).min(lines.len().saturating_sub(inner.height as usize));
            put(f, Paragraph::new(lines).scroll((*scroll as u16, 0)), inner);
            hits.push((r, Hit::List(ListRef::Help)));
        }
        Overlay::Picker { title, options, sel, .. } => {
            let h = (options.len() as u16 + 2).min(area.height.saturating_sub(4));
            let w = options.iter().map(|o| o.0.width()).max().unwrap_or(10).max(title.width()) as u16 + 8;
            let r = centred(area, w, h);
            let inner = popup(f, r, Span::styled(format!(" {title} "), bold().fg(t.ink)), t);
            hits.push((r, Hit::List(ListRef::Picker)));
            let opts = options.clone();
            list(f, inner, sel, opts.len(), ListRef::Picker, hits, t, true, &|i, w| Line::from(fit(&format!(" {}", opts[i].0), w).into_owned()));
        }
        Overlay::JamSettings => {
            let r = centred(area, 64, 8);
            let inner = popup(f, r, Span::raw(crate::text::JAM_SETTINGS), t);
            let mut lines: Vec<Line> = devices.jam.as_ref().map_or_else(Vec::new, |j| crate::text::room_settings(j.along, j.remote_songs).into_iter().map(Line::from).collect());
            lines.push(Line::from(""));
            lines.push(Line::from(crate::text::JAM_SETTINGS_BACK));
            put(f, Paragraph::new(lines), inner);
        }
        Overlay::Invite { link } => {
            let code = nori_core::remote::qr_code(link.clone());
            let qr = code.as_ref().map(qr_lines).unwrap_or_default();
            let qr_w = qr.first().map_or(0, |l| l.width() as u16);
            let fits = qr_w + 4 <= area.width && qr.len() as u16 + 8 <= area.height;
            let w = (qr_w.max(link.width() as u16) + 6).min(area.width);
            let link_lines = wrap(link, w.saturating_sub(4) as usize);
            let mut lines: Vec<Line> = Vec::new();
            if fits {
                lines.extend(qr);
                lines.push(Line::from(""));
            } else {
                lines.push(Line::from(Span::styled(crate::text::INVITE_ROOM, t.dim())));
            }
            lines.push(Line::from(Span::styled(crate::text::INVITE_HOW, t.dim())));
            lines.push(Line::from(crate::text::JAM_SETTINGS_KEY));
            lines.extend(link_lines.into_iter().map(|l| Line::from(Span::styled(l, Style::default().fg(t.ink)))));
            let r = centred(area, w, lines.len() as u16 + 4);
            let inner = popup(f, r, Span::styled(crate::text::INVITE_TITLE, bold().fg(t.ink)), t);
            put(f, Paragraph::new(lines).alignment(Alignment::Center), Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner });
        }
        Overlay::Join { text: typed, error, busy } => {
            let w = 72.min(area.width);
            let r = centred(area, w, 7);
            let inner = popup(f, r, Span::styled(crate::text::JOIN_TITLE, bold().fg(t.ink)), t);
            let iw = inner.width.saturating_sub(2) as usize;
            // The end of a long link, where typing goes.
            let shown: String = typed.chars().rev().take(iw.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
            let (said, style) = match (error, *busy) {
                (Some(e), _) => (e.clone(), Style::default().fg(Color::LightRed)),
                (None, true) => (crate::text::JOINING.to_string(), t.dim()),
                (None, false) => (String::new(), t.dim()),
            };
            let lines = vec![
                Line::from(Span::styled(crate::text::JOIN_HOW, t.dim())),
                Line::from(Span::styled(format!("{shown}▏"), Style::default().fg(t.ink))),
                Line::from(""),
                Line::from(Span::styled(said, style)),
            ];
            put(f, Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner });
        }
        Overlay::Account { kind, user, ended, pasting, text: typed, error, busy } => {
            let w = 76.min(area.width);
            let iw = w.saturating_sub(4).max(1) as usize;
            let name = crate::text::provider_of(kinds, kind).name;
            let title = if user.is_empty() { format!(" Sign in to {name} ") } else { format!(" {name} ") };
            let mut said: Vec<(String, Style)> = Vec::new();
            if !user.is_empty() {
                said.push((format!("Signed in as {user}"), Style::default().fg(t.ink)));
                said.push((crate::text::SIGNED_IN_KEYS.into(), t.dim()));
            } else if *pasting {
                said.push((crate::text::PASTE_COOKIES.into(), t.dim()));
                // Cookies are a secret: their length shows, not them.
                let dots = "•".repeat(typed.chars().count().min(iw.saturating_sub(1)));
                said.push((format!("{dots}▏"), Style::default().fg(t.ink)));
            } else {
                let why = if *busy { crate::text::SIGNING_IN.into() } else if *ended { crate::text::signed_out_again(&name) } else { crate::text::SIGN_IN_WHY.into() };
                said.push((why, Style::default().fg(t.ink)));
                said.push(((if *ended { crate::text::SIGN_IN_KEYS_ENDED } else { crate::text::SIGN_IN_KEYS }).into(), t.dim()));
            }
            if let Some(e) = error {
                said.push((e.clone(), Style::default().fg(Color::LightRed)));
            }
            let rows: usize = said.iter().map(|(l, _)| l.chars().count().div_ceil(iw).max(1)).sum::<usize>() + said.len() - 1;
            let r = centred(area, w, rows as u16 + 3);
            let inner = popup(f, r, Span::styled(title, bold().fg(t.ink)), t);
            let mut lines = Vec::new();
            for (i, (l, style)) in said.into_iter().enumerate() {
                if i > 0 {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(Span::styled(l, style)));
            }
            put(f, Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(1) });
        }
        Overlay::Menu { title, lines, sel, .. } => {
            use crate::menu::MenuLine;
            let words: Vec<(String, Style)> = lines
                .iter()
                .map(|l| match l {
                    MenuLine::Heading(h) => (h.clone(), bold().fg(t.ink)),
                    MenuLine::More => (format!(" {}", crate::text::MORE), t.dim()),
                    MenuLine::Act(a) => (format!(" {}", crate::text::menu_act(a, missing)), Style::default().fg(t.ink)),
                })
                .collect();
            let w = words.iter().map(|(s, _)| s.width()).max().unwrap_or(10).max(title.width() + 2) as u16 + 6;
            let r = centred(area, w, (words.len() as u16 + 2).min(area.height.saturating_sub(2)));
            let inner = popup(f, r, Span::styled(format!(" {} ", fit(title, w.saturating_sub(4) as usize)), bold().fg(t.ink)), t);
            hits.push((r, Hit::List(ListRef::Menu)));
            let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
            list(f, inner, sel, words.len(), ListRef::Menu, hits, t, true, &|i, w| Line::from(Span::styled(fit(&words[i].0, w).into_owned(), words[i].1)));
        }
        Overlay::NewPlaylist { text: typed, .. } => {
            let r = centred(area, 60, 3);
            let inner = popup(f, r, Span::styled(format!(" {} · enter makes it, esc drops ", crate::text::NEW_PLAYLIST), bold().fg(t.ink)), t);
            put(f, Paragraph::new(format!(" {typed}▏")), inner);
        }
        Overlay::Input { title, text: typed, secret, .. } => {
            let r = centred(area, 60, 3);
            let inner = popup(f, r, Span::styled(format!(" {title} · enter keeps, esc drops "), bold().fg(t.ink)), t);
            let shown = if *secret { "•".repeat(typed.chars().count()) } else { typed.clone() };
            put(f, Paragraph::new(format!(" {shown}▏")), inner);
        }
        Overlay::Confirm { title, lines: said, yes, .. } => {
            let w = 64.min(area.width);
            let iw = w.saturating_sub(4).max(1) as usize;
            let mut lines = Vec::new();
            for (i, l) in said.iter().enumerate() {
                lines.push(Line::from(Span::styled(l.clone(), if i == 0 { Style::default().fg(t.ink) } else { t.dim() })));
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(crate::text::confirm_keys(yes), t.dim())));
            let rows: usize = said.iter().map(|l| l.chars().count().div_ceil(iw).max(1) + 1).sum::<usize>() + 1;
            let r = centred(area, w, rows as u16 + 2);
            let inner = popup(f, r, Span::styled(format!(" {title} "), bold().fg(t.ink)), t);
            put(f, Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner });
        }
    }
}
