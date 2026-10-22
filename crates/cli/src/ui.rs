//! Drawing: the page under two outlines, the tabs along the top and the player along the bottom, and the
//! full player with its panel. The page is painted in the season's colors; where it scrolls under an outline
//! its nearest two rows fade into the page instead of blurring. Every clickable area is recorded in
//! `App::hits` as it is drawn. Lists and grids draw only visible rows.

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

use crate::app::{App, Button, DeviceRow, Focus, Hit, Item, ListRef, Load, Overlay, Page, Panel, QueueRow, SearchRow, Sel, View, LOGIN_FIELDS, PANELS};
use crate::art::{Art, Theme};
use crate::keys::{Scope, BINDINGS};
use crate::settings_view::{self, EqRow, Line as SLine, SettingsView};
use crate::text::clock;

/// Album card minimum width and height without covers.
const CARD_W: u16 = 22;
const CARD_H: u16 = 4;

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

/// Card cover state for a frame: the covers, whether cards show them, and missing ones to request after.
pub struct Pics<'a> {
    art: Option<&'a mut Art>,
    on: bool,
    want: Vec<String>,
}

impl Pics<'_> {
    /// Card minimum width and height.
    fn card(&self) -> (u16, u16) {
        if self.on {
            (CARD_W + 8, CARD_H + 2)
        } else {
            (CARD_W, CARD_H)
        }
    }
}

/// A grid of cards (cover with text beside it, or a box without covers), selection kept in view;
/// `sel.top` is the first visible row. Returns the number of columns.
#[allow(clippy::too_many_arguments)]
fn cards(f: &mut Frame, area: Rect, sel: &mut Sel, len: usize, lref: ListRef, hits: &mut Vec<(Rect, Hit)>, t: &Theme, focused: bool, pics: &mut Pics, card: &dyn Fn(usize) -> (String, String, Option<String>)) -> usize {
    hits.push((area, Hit::List(lref)));
    let (min_w, card_h) = pics.card();
    let cols = ((area.width + 1) / (min_w + 1)).max(1) as usize;
    let w = ((area.width + 1) / cols as u16).saturating_sub(1).max(1);
    let rows = (area.height / card_h).max(1) as usize;
    sel.at = sel.at.min(len.saturating_sub(1));
    let row_at = sel.at / cols;
    if row_at < sel.top {
        sel.top = row_at;
    } else if row_at >= sel.top + rows {
        sel.top = row_at + 1 - rows;
    }
    sel.cut = (sel.top > 0, (sel.top + rows) * cols < len);
    for r in 0..rows {
        for c in 0..cols {
            let i = (sel.top + r) * cols + c;
            if i >= len {
                return cols;
            }
            let rect = Rect { x: area.x + c as u16 * (w + 1), y: area.y + r as u16 * card_h, width: w, height: card_h.min(area.height) };
            let (title, sub, art) = card(i);
            one_card(f, rect, &title, &sub, art.as_deref(), pics, t, i == sel.at, focused);
            hits.push((rect.intersection(area), Hit::Row(lref, i)));
        }
    }
    cols
}

#[allow(clippy::too_many_arguments)]
fn one_card(f: &mut Frame, r: Rect, title: &str, sub: &str, art: Option<&str>, pics: &mut Pics, t: &Theme, chosen: bool, focused: bool) {
    // The chosen card is raised: the surface under the cover and the words.
    let ground = if chosen { t.select(focused).remove_modifier(Modifier::BOLD) } else { Style::default() };
    put(f, Block::default().style(ground), r);
    if pics.on {
        let h = r.height.saturating_sub(1).min(CARD_H + 1);
        let cr = Rect { width: (h * 2).min(r.width), height: h, ..r };
        let key = art.map(|a| format!("{}{a}", crate::backend::THUMB));
        if let (Some(a), Some(k)) = (art, &key) {
            if !pics.art.as_ref().is_some_and(|x| x.has(k)) {
                pics.want.push(a.to_string());
            }
        }
        cover(f, cr, pics.art.as_deref_mut(), key.as_deref(), t);
        let words = Rect { x: cr.x + cr.width + 1, y: r.y + h.saturating_sub(3) / 2, width: r.width.saturating_sub(cr.width + 1), height: 3.min(h) };
        let w = words.width as usize;
        let (who, when) = sub.split_once(" · ").unwrap_or((sub, ""));
        let lines = vec![Line::from(Span::styled(fit(title, w).into_owned(), bold().fg(t.ink))), Line::from(Span::styled(fit(who, w).into_owned(), t.dim())), Line::from(Span::styled(fit(when, w).into_owned(), t.dim()))];
        put(f, Paragraph::new(lines), words);
        return;
    }
    let border = if chosen { Style::default().fg(t.ink) } else { Style::default().fg(t.hair) };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border);
    let inner = block.inner(r);
    put(f, block, r);
    let w = inner.width.saturating_sub(1) as usize;
    let lines = vec![Line::from(Span::styled(format!(" {}", fit(title, w)), bold().fg(t.ink))), Line::from(Span::styled(format!(" {}", fit(sub, w)), t.dim()))];
    put(f, Paragraph::new(lines), inner);
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

/// A bold page title with a right-aligned caption; returns the area below plus a blank line.
fn heading(f: &mut Frame, area: Rect, title: &str, caption: &str, t: &Theme) -> Rect {
    if area.height < 3 {
        return area;
    }
    let line = spread(vec![Span::styled(title.to_string(), bold().fg(t.ink))], Span::styled(caption.to_string(), t.dim()), area.width as usize);
    put(f, Paragraph::new(line), Rect { height: 1, ..area });
    Rect { y: area.y + 2, height: area.height - 2, ..area }
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
    if area.height < 4 {
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
    put(f, Paragraph::new(Span::styled(s, t.dim().add_modifier(Modifier::BOLD))), Rect { height: 1, ..area });
    put(f, Paragraph::new(Span::styled("─".repeat(area.width as usize), Style::default().fg(t.hair))), Rect { y: area.y + 1, height: 1, ..area });
    Rect { y: area.y + 2, height: area.height - 2, ..area }
}

/// What a song row marks after its title: that a jam guest asked for the song, else its seal where the
/// profile has favorites, as the session's star marks show it.
struct Marks<'a> {
    stars: &'a StarMarks,
    hearts: bool,
    asked: HashSet<&'a str>,
}

impl<'a> Marks<'a> {
    fn of(stars: &'a StarMarks, rules: &nori_core::browse::ProfileRules, devices: &'a crate::app::Devices) -> Marks<'a> {
        Marks { stars, hearts: rules.stars, asked: devices.asked() }
    }

    fn mark(&self, s: &Song) -> String {
        if self.asked.contains(s.id.as_str()) {
            format!(" {}", crate::text::ASKED)
        } else if self.hearts && self.stars.starred(Starrable::Song, &s.id, s.starred) {
            format!(" {SEAL}")
        } else {
            String::new()
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
    let mut title = s.title.clone();
    if s.is_provider() {
        title.push_str(" ☁");
    }
    let mark = marks.mark(s);
    let title_style = if playing { t.current() } else { Style::default().fg(t.ink) };
    let tw = c[1].saturating_sub(mark.width() + 2);
    let mut spans = vec![num, Span::styled(pad(&title, tw), title_style), Span::styled(pad(&mark, mark.width() + 2), if mark.trim() == SEAL { Style::default().fg(t.accent) } else { t.dim() })];
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

// ---- the frame ----

/// The two rows nearest an outline where scrolled content fades: a third, then two thirds of the way to the
/// page.
const FADE: [f32; 2] = [0.66, 0.33];

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
        main(f, rows.page, app, &t, art.as_deref_mut());
        fade_edges(f, rows.page, app, &t);
        toast(f, rows.page, app);
        top_outline(f, rows.top, app, &t);
        if rows.bar.height > 0 {
            player_bar(f, rows.bar, app, &t, art.as_deref_mut());
        }
    }
    footer(f, rows.foot, app, &t);
    overlay(f, area, app, &t);
    crate::art::finish(f.buffer_mut(), app.colors);
}

/// Fades the rows next to an outline where the page scrolls under it: toward the page color, or dimmed
/// where the page is the terminal's own.
fn fade_edges(f: &mut Frame, page: Rect, app: &App, t: &Theme) {
    let (above, below) = app.page_cut();
    let bottom = page.y + page.height;
    if above {
        fade_row(f.buffer_mut(), Rect { y: page.y, height: 1, ..page }, t, FADE[0]);
        fade_row(f.buffer_mut(), Rect { y: page.y + 1, height: 1, ..page }, t, FADE[1]);
    }
    if below && page.height >= 2 {
        fade_row(f.buffer_mut(), Rect { y: bottom - 1, height: 1, ..page }, t, FADE[0]);
        fade_row(f.buffer_mut(), Rect { y: bottom - 2, height: 1, ..page }, t, FADE[1]);
    }
}

/// Blends the colors of the cells in a one-row `r` toward the page by `strength`.
pub fn fade_row(buf: &mut ratatui::buffer::Buffer, r: Rect, t: &Theme, strength: f32) {
    let r = r.intersection(buf.area);
    if r.is_empty() {
        return;
    }
    for x in r.left()..r.right() {
        let cell = &mut buf[(x, r.y)];
        if t.painted {
            cell.fg = t.fade(cell.fg, strength);
            cell.bg = t.fade(cell.bg, strength);
        } else if strength > 0.5 {
            cell.modifier.insert(Modifier::DIM);
        }
    }
}

/// The status note, bottom right of the page.
fn toast(f: &mut Frame, area: Rect, app: &App) {
    let Some((msg, error, _)) = &app.note else { return };
    let t = app.look;
    let w = (msg.width() as u16 + 4).min(area.width);
    if w < 8 || area.height < 4 {
        return;
    }
    let r = Rect { x: area.x + area.width.saturating_sub(w + 1), y: area.y + area.height - 3, width: w, height: 3 };
    let colour = if *error { Style::default().fg(Color::LightRed) } else { Style::default().fg(t.ink) };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(t.line)).style(t.raised());
    let inner = block.inner(r);
    put(f, Clear, r);
    put(f, block, r);
    text(f, Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner }, msg, colour);
}

/// A rounded outline in the chrome's line color with the page inside it.
fn outline(f: &mut Frame, r: Rect, t: &Theme) {
    put(f, Clear, r);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(t.line)).style(t.ground());
    put(f, block, r);
}

// ---- the top outline ----

/// The tabs, then search and settings, each in its own outline. The open tab is accent, bold and underlined.
fn top_outline(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let tabs = app.tabs();
    let top = crate::chrome::top(area, &tabs);
    let boxed = area.height >= 3;
    if boxed {
        outline(f, top.tabs, t);
    }
    for (view, r) in &top.items {
        let style = if *view == app.view {
            t.current().add_modifier(Modifier::UNDERLINED)
        } else {
            t.dim()
        };
        put(f, Paragraph::new(Span::styled(crate::chrome::tab_label(*view, top.names), style)), *r);
        app.hits.push((*r, Hit::Tab(*view)));
    }
    if let (Some(x), true) = (top.rule, boxed) {
        put(f, Paragraph::new(Span::styled("│", Style::default().fg(t.hair))), Rect { x, y: area.y + 1, width: 1, height: 1 });
    }
    if let Some(r) = top.search {
        let editing = app.view == View::Search && app.search.editing;
        if boxed {
            outline(f, r, t);
            if editing {
                put(f, Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(t.current()), r);
            }
        }
        let inner = Rect { x: r.x + 2, y: area.y + boxed as u16, width: r.width.saturating_sub(4), height: 1 };
        let shown = if top.search_small {
            Span::styled("⌕", t.dim())
        } else if app.search.text.is_empty() && !editing {
            Span::styled("⌕ Search", t.dim())
        } else {
            let cursor = if editing { "▏" } else { "" };
            let typed = format!("{}{cursor}", app.search.text);
            let room = inner.width.saturating_sub(2) as usize;
            // The end of what is typed, where the cursor is.
            let tail: String = typed.chars().rev().take(room).collect::<Vec<_>>().into_iter().rev().collect();
            Span::styled(format!("⌕ {tail}"), Style::default().fg(t.ink))
        };
        put(f, Paragraph::new(shown), inner);
        app.hits.push((r, Hit::SearchField));
    }
    if let Some(r) = top.settings {
        if boxed {
            outline(f, r, t);
        }
        let open = app.view == View::Settings;
        let style = if open { t.current() } else { t.dim() };
        put(f, Paragraph::new(Span::styled("✱", style)).alignment(Alignment::Center), Rect { x: r.x + 1, y: area.y + boxed as u16, width: r.width.saturating_sub(2), height: 1 });
        app.hits.push((r, Hit::Tab(View::Settings)));
    }
}

// ---- the hint line ----

/// The keys under the player, and on the right the destinations the tabs have no room for and the
/// server's state.
fn footer(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let m = crate::chrome::margin(area.width);
    let quiet = Style::default().fg(t.lit(0.32));
    let (dot, colour, word) = if app.offline {
        ("●", Color::Yellow, "offline · ")
    } else if app.unreachable.is_some() {
        ("●", Color::LightRed, "unreachable · ")
    } else {
        ("●", Color::Green, "")
    };
    let server = fit(&format!("{word}{}", app.server), 24).into_owned();
    let mut items = vec![(format!("{dot} {server}"), None), ("≋ Equalizer".to_string(), Some(View::Equalizer)), ("↓ Downloads".to_string(), Some(View::Downloads))];
    items.retain(|(_, link)| link.is_none_or(|v| app.reaches(v)));
    let mut x = area.x + area.width.saturating_sub(m);
    let mut right = Vec::new();
    for (label, link) in items {
        let w = label.width() as u16;
        // The hint keeps 20 columns.
        if x < area.x + m + 20 + w + 2 {
            break;
        }
        x -= w;
        right.push((Rect { x, y: area.y, width: w, height: 1 }, label, link));
        x -= 2;
    }
    let hint = if app.full { crate::text::HINT_FULL } else { crate::text::HINT };
    let room = x.saturating_sub(area.x + m + 1) as usize;
    let hint_r = Rect { x: area.x + m, y: area.y, width: room as u16, height: 1 };
    text(f, hint_r, hint, quiet);
    // A click on the hint shows every key.
    app.hits.push((hint_r, Hit::Button(Button::Help)));
    for (r, label, link) in right {
        match link {
            Some(view) => {
                let style = if app.view == view { t.current() } else { t.dim() };
                put(f, Paragraph::new(Span::styled(label, style)), r);
                app.hits.push((r, Hit::Tab(view)));
            }
            None => {
                let (d, rest) = label.split_at(dot.len());
                put(f, Paragraph::new(Line::from(vec![Span::styled(d.to_string(), Style::default().fg(if t.mono { Color::Reset } else { colour })), Span::styled(rest.to_string(), t.dim())])), r);
            }
        }
    }
}

// ---- the page ----

fn main(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, art: Option<&mut Art>) {
    let on = app.images && app.card_covers && art.is_some();
    let mut pics = Pics { art, on, want: Vec::new() };
    page_view(f, area, app, t, &mut pics);
    // Request missing card covers once each.
    for id in pics.want {
        if app.thumbs_asked.len() > 2000 {
            app.thumbs_asked.clear();
        }
        if app.thumbs_asked.insert(id.clone()) {
            app.cmds.push(crate::app::Cmd::Thumb(id));
        }
    }
}

fn page_view(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, pics: &mut Pics) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let mut area = area;
    if let Some(e) = &app.unreachable {
        text(f, Rect { x: area.x + 2, width: area.width.saturating_sub(4), ..area }, &format!("⚠ {e} · showing what is stored · R tries again"), Style::default().fg(Color::LightRed));
        area = Rect { y: area.y + 1, height: area.height - 1, ..area };
    }
    let inner = Rect { x: area.x + 2, y: area.y + 1, width: area.width.saturating_sub(4), height: area.height.saturating_sub(1) };
    let focused = app.focus == Focus::Main;
    if !app.pages.is_empty() {
        return page(f, inner, app, t, focused, pics);
    }
    match app.view {
        View::Home => home(f, inner, app, t, focused, pics),
        View::Search => search(f, inner, app, t, focused),
        View::Albums => albums(f, inner, app, t, focused, pics),
        View::Artists => artists(f, inner, app, t, focused),
        View::Songs => songs(f, inner, app, t, focused),
        View::Playlists => playlists(f, inner, app, t, focused),
        View::Favorites => favorites(f, inner, app, t, focused),
        View::Downloads => downloads(f, inner, app, t, focused),
        View::Equalizer => equalizer(f, inner, app, t, focused),
        View::Settings => settings(f, inner, app, t, focused),
        View::Login => {}
    }
}

fn failed(f: &mut Frame, area: Rect, t: &Theme, e: &str) {
    let text = vec![
        Line::from(Span::styled("Could not load this page", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(e.to_string(), Style::default().fg(t.ink))),
        Line::from(""),
        Line::from(Span::styled("R tries again", t.dim())),
    ];
    put(f, Paragraph::new(text).alignment(Alignment::Center).wrap(ratatui::widgets::Wrap { trim: true }), centred(area, area.width.min(70), 6));
}

fn loading(f: &mut Frame, area: Rect, t: &Theme) {
    put(f, Paragraph::new(Span::styled("Loading…", t.dim())).alignment(Alignment::Center), centred(area, 20, 1));
}

fn empty(f: &mut Frame, area: Rect, t: &Theme, text: &str) {
    put(f, Paragraph::new(Span::styled(text.to_string(), t.dim())).alignment(Alignment::Center).wrap(ratatui::widgets::Wrap { trim: true }), centred(area, area.width, 2));
}

fn home(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool, pics: &mut Pics) {
    let area = heading(f, area, "Listen now", "", t);
    let shelves: Vec<(usize, String, usize)> = app.home.shelves().iter().map(|(i, title, v)| (*i, title.to_string(), v.len())).collect();
    if shelves.is_empty() {
        match &app.home.error {
            Some(e) => failed(f, area, t, e),
            None => loading(f, area, t),
        }
        return;
    }
    // Each shelf: title line, a row of cards, a blank line.
    let (min_w, card_h) = pics.card();
    let shelf_h = card_h + 2;
    let fits = (area.height / shelf_h).max(1) as usize;
    let at = shelves.iter().position(|s| s.0 == app.home.shelf).unwrap_or(0);
    let h = &mut app.home;
    if at < h.top {
        h.top = at;
    } else if at >= h.top + fits {
        h.top = at + 1 - fits;
    }
    h.top = h.top.min(shelves.len().saturating_sub(1));
    h.below = h.top + fits < shelves.len();
    let cols = ((area.width + 1) / (min_w + 1)).max(1) as usize;
    let w = ((area.width + 1) / cols as u16).saturating_sub(1).max(1);
    for (k, (i, title, len)) in shelves.iter().enumerate().skip(app.home.top).take(fits) {
        let y = area.y + (k - app.home.top) as u16 * shelf_h;
        if y + 1 >= area.y + area.height {
            break;
        }
        let chosen_shelf = *i == app.home.shelf;
        let pos = app.home.pos.get(*i).copied().unwrap_or(0).min(len.saturating_sub(1));
        let left = &mut app.home.left[*i];
        if pos < *left {
            *left = pos;
        } else if pos >= *left + cols {
            *left = pos + 1 - cols;
        }
        let left = *left;
        let more = if *len > cols { format!("{} of {len}  ‹ ›", pos + 1) } else { String::new() };
        put(f, Paragraph::new(spread(vec![Span::styled(title.to_string(), bold().fg(t.ink))], Span::styled(more, t.dim()), area.width as usize)), Rect { y, height: 1, ..area });
        let row = Rect { y: y + 1, height: card_h.min((area.y + area.height).saturating_sub(y + 1)), ..area };
        app.hits.push((row, Hit::List(ListRef::Shelf(*i))));
        let Some(Some((_, items))) = app.home.rows.get(*i) else { continue };
        for c in 0..cols {
            let n = left + c;
            let Some(item) = items.get(n) else { break };
            let r = Rect { x: area.x + c as u16 * (w + 1), width: w, ..row };
            let (name, sub, art) = match item {
                Item::Album(a) | Item::DownloadedAlbum(a) => (&a.name, if a.year > 0 { format!("{} · {}", a.artist, a.year) } else { a.artist.clone() }, &a.cover_art),
                Item::Song(songs, at) => (&songs[*at].title, songs[*at].artist.clone(), &songs[*at].cover_art),
                Item::Playlist(p) => (&p.name, crate::text::playlist_line(p), &p.cover_art),
                Item::Artist(a) => (&a.name, a.album_count.map(crate::text::albums).unwrap_or_default(), &a.cover_art),
            };
            one_card(f, r, name, &sub, art.as_deref(), pics, t, chosen_shelf && n == pos, focused);
            app.hits.push((r, Hit::Row(ListRef::Shelf(*i), n)));
        }
    }
}

fn albums(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool, pics: &mut Pics) {
    let l = &mut app.library;
    let caption = l.albums.ready().map_or(String::new(), |v| crate::text::count(v.len() as u64, "album", "albums") + if l.albums_more { "+" } else { "" });
    let body = heading(f, area, "Albums", &caption, t);
    match &l.albums {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "Nothing here yet"),
        Load::Ready(v) => {
            let len = v.len();
            let cols = cards(f, body, &mut l.albums_sel, len, ListRef::Albums, &mut app.hits, t, focused, pics, &|i| {
                let a = &v[i];
                (a.name.clone(), if a.year > 0 { format!("{} · {}", a.artist, a.year) } else { a.artist.clone() }, a.cover_art.clone())
            });
            app.shown.cols = cols;
        }
        Load::Failed(e) => failed(f, body, t, e),
        _ => loading(f, body, t),
    }
}

fn artists(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let hearts = app.rules.stars;
    let App { library: l, marks, hits, .. } = app;
    let caption = l.artists.ready().map_or(String::new(), |v| crate::text::count(v.len() as u64, "artist", "artists"));
    let body = heading(f, area, "Artists", &caption, t);
    match &l.artists {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "Nothing here yet"),
        Load::Ready(v) => {
            let row = |i: usize, w: usize| {
                let a = &v[i];
                let seal = if hearts && marks.starred(Starrable::Artist, &a.id, a.starred) { format!(" {SEAL}") } else { String::new() };
                spread(vec![Span::styled("  ◉  ", t.dim()), Span::styled(a.name.as_str(), bold()), Span::styled(seal, Style::default().fg(t.accent))], Span::styled(a.album_count.map(crate::text::albums).unwrap_or_default() + " ", t.dim()), w)
            };
            list(f, body, &mut l.artists_sel, v.len(), ListRef::Artists, hits, t, focused, &row);
        }
        Load::Failed(e) => failed(f, body, t, e),
        _ => loading(f, body, t),
    }
}

fn songs(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { library: l, hits, marks, rules, devices, .. } = app;
    let marks = Marks::of(marks, rules, devices);
    let caption = l.songs.ready().map_or(String::new(), |v| crate::text::count(v.len() as u64, "song", "songs") + if l.songs_more { "+" } else { "" });
    let body = heading(f, area, "Songs", &caption, t);
    match &l.songs {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "Nothing here yet"),
        Load::Ready(v) => {
            let body = table_head(f, body, t, true);
            let row = |i: usize, w: usize| song_line(&v[i], &marks, i + 1, w, t, playing.as_deref() == Some(v[i].id.as_str()), true);
            list(f, body, &mut l.songs_sel, v.len(), ListRef::Songs, hits, t, focused, &row);
        }
        Load::Failed(e) => failed(f, body, t, e),
        _ => loading(f, body, t),
    }
}

fn playlists(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let App { library: l, hits, .. } = app;
    let caption = l.playlists.ready().map_or(String::new(), |v| crate::text::count(v.len() as u64, "playlist", "playlists"));
    let body = heading(f, area, "Playlists", &caption, t);
    match &l.playlists {
        Load::Ready(v) if v.is_empty() => empty(f, body, t, "No playlists yet"),
        Load::Ready(v) => {
            let row = |i: usize, w: usize| {
                let p = &v[i];
                let owner = p.owner.clone().filter(|o| !o.is_empty()).map(|o| format!("{o} · ")).unwrap_or_default();
                spread(vec![Span::styled("  ≡  ", t.dim()), Span::styled(p.name.as_str(), bold())], Span::styled(format!("{owner}{} ", crate::text::count(p.song_count as u64, "song", "songs")), t.dim()), w)
            };
            list(f, body, &mut l.playlists_sel, v.len(), ListRef::Playlists, hits, t, focused, &row);
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
    let body = heading(f, area, "Favorites", &caption, t);
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

fn search(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    if area.height < 3 {
        return;
    }
    let s = &app.search;
    text(f, Rect { height: 1, ..area }, "Search", bold().fg(t.ink));
    let note = Rect { y: area.y + 1, height: 1, ..area };
    let view = s.view.as_ref();
    let (words, style) = match view {
        Some(v) if v.error.is_some() => (crate::text::search_fallback(v.error.as_ref().and_then(|e| e.reason.as_deref())), Style::default().fg(Color::LightRed)),
        Some(v) if v.searching => ("Asking the server…".to_string(), t.dim()),
        Some(v) if v.nothing_found => ("Nothing found".to_string(), t.dim()),
        Some(v) if !v.query.is_empty() => ((if v.from_server { "From the server" } else { "From the offline index · the server is asked when you pause" }).to_string(), t.dim()),
        _ => ("Songs, albums and artists, from the library".to_string(), t.dim()),
    };
    text(f, note, &words, style);
    let body = Rect { y: area.y + 3, height: area.height.saturating_sub(3), ..area };
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let App { search: s, hits, marks, rules, devices, .. } = app;
    let marks = Marks::of(marks, rules, devices);
    let lit = focused && !s.editing;
    let mut sel = s.sel;
    let rows = s.rows();
    if rows.is_empty() {
        return;
    }
    let len = rows.len();
    let row = |i: usize, w: usize| match &rows[i] {
        SearchRow::Title(title, n) => spread(vec![Span::styled(title.to_string(), bold().fg(t.ink))], Span::styled(format!("{n} "), t.dim()), w),
        SearchRow::Song(song) => song_line(song, &marks, 0, w, t, playing.as_deref() == Some(song.id.as_str()), true),
        SearchRow::Album(a) => {
            let year = if a.year > 0 { format!("{} ", a.year) } else { String::new() };
            spread(vec![Span::styled("  ◫  ", t.dim()), Span::styled(a.name.clone(), Style::default().fg(t.ink)), Span::styled(format!("  {}", a.artist), t.dim())], Span::styled(year, t.dim()), w)
        }
        SearchRow::Artist(a) => spread(vec![Span::styled("  ◉  ", t.dim()), Span::styled(a.name.clone(), Style::default().fg(t.ink))], Span::styled(a.album_count.map(crate::text::albums).unwrap_or_default() + " ", t.dim()), w),
        SearchRow::Playlist(p) => spread(vec![Span::styled("  ≡  ", t.dim()), Span::styled(p.name.clone(), Style::default().fg(t.ink))], Span::styled(crate::text::playlist_line(p) + " ", t.dim()), w),
    };
    list(f, body, &mut sel, len, ListRef::Search, hits, t, lit, &row);
    drop(rows);
    s.sel = sel;
}

/// A cover in `area`, or a placeholder box if it is not loaded.
fn cover(f: &mut Frame, area: Rect, art: Option<&mut Art>, key: Option<&str>, t: &Theme) {
    let area = area.intersection(f.area());
    if area.width < 2 || area.height < 1 {
        return;
    }
    let protocol: Option<&mut StatefulProtocol> = match (art, key) {
        (Some(a), Some(k)) => a.get(k),
        _ => None,
    };
    match protocol {
        Some(p) => {
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
        None => {
            let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(t.dim());
            let inner = block.inner(area);
            put(f, block, area);
            put(f, Paragraph::new(Span::styled("♫", t.dim())).alignment(Alignment::Center), centred(inner, 1, 1));
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

/// A square cover `rows` tall (twice as wide in cells) at the top left of `area`.
fn cover_box(area: Rect, rows: u16) -> Rect {
    let h = rows.min(area.height).min(area.width / 2);
    Rect { x: area.x, y: area.y, width: h * 2, height: h }
}

/// A page button, bold when strong; advances `x`.
#[allow(clippy::too_many_arguments)]
fn pill(f: &mut Frame, hits: &mut Vec<(Rect, Hit)>, x: &mut u16, y: u16, end: u16, label: &str, strong: bool, b: Button, t: &Theme) {
    let s = format!(" {label} ");
    let w = s.width() as u16;
    if *x + w > end {
        return;
    }
    let style = if strong { bold().fg(t.ink) } else { Style::default().fg(t.ink) };
    let r = Rect { x: *x, y, width: w, height: 1 };
    put(f, Paragraph::new(Span::styled(s, style)), r);
    hits.push((r, Hit::Button(b)));
    *x += w + 2;
}

fn page(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool, pics: &mut Pics) {
    let images = app.images;
    let playing = app.song.as_ref().map(|s| s.id.clone());
    let account = app.rules.account;
    let downloads = app.rules.downloads;
    let App { pages, hits, shown, marks, rules, devices, .. } = app;
    let marks = Marks::of(marks, rules, devices);
    let Some(p) = pages.last_mut() else { return };
    // star: None when the page cannot be starred.
    let (kind, title, sub, caption, art_key, star): (&str, String, String, String, Option<String>, Option<bool>) = match p {
        Page::Album { detail: Load::Ready(d), .. } | Page::DownloadedAlbum { detail: Load::Ready(d), .. } => ("ALBUM", d.album.name.clone(), d.album.artist.clone(), crate::text::album_caption(d), d.album.cover_art.clone(), Some(marks.stars.starred(Starrable::Album, &d.album.id, d.album.starred))),
        Page::Artist { detail: Load::Ready(d), .. } => ("ARTIST", d.artist.name.clone(), if d.albums.is_empty() && !d.songs.is_empty() { crate::text::count(d.songs.len() as u64, "song", "songs") } else { d.artist.album_count.map(crate::text::albums).unwrap_or_default() }, String::new(), None, Some(marks.stars.starred(Starrable::Artist, &d.artist.id, d.artist.starred))),
        Page::Playlist { detail: Load::Ready(d), .. } => ("PLAYLIST", d.playlist.name.clone(), d.playlist.owner.clone().unwrap_or_default(), crate::text::playlist_caption(d), None, None),
        Page::Album { detail: Load::Failed(e), .. } | Page::DownloadedAlbum { detail: Load::Failed(e), .. } | Page::Artist { detail: Load::Failed(e), .. } | Page::Playlist { detail: Load::Failed(e), .. } => {
            return failed(f, area, t, e);
        }
        _ => return loading(f, area, t),
    };
    let with_cover = images && matches!(p, Page::Album { .. } | Page::DownloadedAlbum { .. }) && area.height >= 16;
    let head_h = if with_cover { 8 } else { 6 }.min(area.height);
    let head = Rect { height: head_h, ..area };
    let mut words = head;
    if with_cover {
        let c = cover_box(head, head_h);
        cover(f, c, pics.art.as_deref_mut(), art_key.as_deref(), t);
        words = Rect { x: c.x + c.width + 3, width: head.width.saturating_sub(c.width + 3), ..head };
    }
    let w = words.width as usize;
    let mut lines = vec![Line::from(Span::styled(kind, t.dim().add_modifier(Modifier::BOLD))), Line::from(Span::styled(fit(&title, w).into_owned(), bold().fg(t.ink)))];
    if !sub.is_empty() {
        lines.push(Line::from(Span::styled(fit(&sub, w).into_owned(), bold().fg(t.ink))));
    }
    lines.push(Line::from(Span::styled(fit(&caption, w).into_owned(), t.dim())));
    let top = if with_cover { words.y + words.height.saturating_sub(6) } else { words.y };
    put(f, Paragraph::new(lines), Rect { y: top, height: 4.min(words.height), ..words });
    let y = top + 5;
    if y < area.y + area.height {
        let mut x = words.x;
        let end = words.x + words.width;
        pill(f, hits, &mut x, y, end, "▶ Play", true, Button::PlayAll, t);
        pill(f, hits, &mut x, y, end, "⤮ Shuffle", false, Button::ShuffleAll, t);
        // Hearts and downloads are the account's.
        if let Some(on) = star.filter(|_| account) {
            let label = " Favorite";
            let w = 2 + label.width() as u16;
            if x + w <= end {
                let r = Rect { x, y, width: w, height: 1 };
                put(f, Paragraph::new(Line::from(vec![seal(on, false, t), Span::styled(label, Style::default().fg(t.ink))])), r);
                hits.push((r, Hit::Button(Button::Star)));
                x += w + 2;
            }
        }
        if downloads && !matches!(p, Page::DownloadedAlbum { .. }) {
            pill(f, hits, &mut x, y, end, "↓ Download", false, Button::Download, t);
        }
        let back = "‹ Back";
        if x + back.width() as u16 <= end {
            let r = Rect { x, y, width: back.width() as u16, height: 1 };
            put(f, Paragraph::new(Span::styled(back, t.dim())), r);
            hits.push((r, Hit::Button(Button::Back)));
        }
    }
    let body = Rect { y: area.y + head_h + 1, height: area.height.saturating_sub(head_h + 1), ..area };
    match p {
        Page::Album { detail: Load::Ready(d), sel, .. } | Page::DownloadedAlbum { detail: Load::Ready(d), sel, .. } => {
            let multi = d.discs.len() > 1;
            let songs = &d.songs;
            let body = table_head(f, body, t, false);
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| {
                let n = if multi { songs[i].disc_number as usize * 100 + songs[i].track as usize } else { songs[i].track as usize };
                song_line(&songs[i], &marks, if n > 0 { n } else { i + 1 }, w, t, playing.as_deref() == Some(songs[i].id.as_str()), false)
            });
        }
        Page::Playlist { detail: Load::Ready(d), sel, .. } => {
            let songs = &d.songs;
            let body = table_head(f, body, t, true);
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| song_line(&songs[i], &marks, i + 1, w, t, playing.as_deref() == Some(songs[i].id.as_str()), true));
        }
        Page::Artist { detail: Load::Ready(d), sel, .. } if d.albums.is_empty() && !d.songs.is_empty() => {
            let songs = &d.songs;
            let body = heading(f, body, "Songs", "", t);
            let body = table_head(f, body, t, true);
            list(f, body, sel, songs.len(), ListRef::Page, hits, t, focused, &|i, w| song_line(&songs[i], &marks, i + 1, w, t, playing.as_deref() == Some(songs[i].id.as_str()), true));
        }
        Page::Artist { detail: Load::Ready(d), sel, .. } => {
            let albums = &d.albums;
            let body = heading(f, body, "Albums", "", t);
            shown.cols = cards(f, body, sel, albums.len(), ListRef::Page, hits, t, focused, pics, &|i| {
                let a = &albums[i];
                (a.name.clone(), if a.year > 0 { format!("{} · {}", a.artist, a.year) } else { a.artist.clone() }, a.cover_art.clone())
            });
        }
        _ => {}
    }
}

fn downloads(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let body = heading(f, area, "Downloads", "D on a song, an album or a playlist downloads it", t);
    match &app.downloads {
        Load::Failed(e) => return failed(f, body, t, &e.clone()),
        Load::Ready(_) => {}
        _ => return loading(f, body, t),
    }
    let rows = app.download_rows();
    if rows.is_empty() {
        return empty(f, body, t, "Nothing downloaded yet");
    }
    let lines: Vec<Line> = rows
        .iter()
        .map(|row| match row {
            crate::app::DownloadRow::Header(title) => Line::from(Span::styled(title.clone(), bold().fg(t.ink))),
            crate::app::DownloadRow::Album(a) => Line::from(vec![Span::styled("  ↓  ", t.dim()), Span::raw(a.name.clone()), Span::styled(format!("  {} · {} songs", a.artist, a.song_count), t.dim())]),
            crate::app::DownloadRow::Song(list, i) => {
                let s = &list[*i];
                Line::from(vec![Span::styled("  ↓  ", t.dim()), Span::raw(s.title.clone()), Span::styled(format!("  {}", s.artist), t.dim())])
            }
        })
        .collect();
    let len = lines.len();
    let App { downloads_sel, hits, .. } = app;
    list(f, body, downloads_sel, len, ListRef::Downloads, hits, t, focused, &|i, _| lines[i].clone());
}

/// The equalizer: toolbar chips, band faders, then the rest of the chain as chips. ← → walk them in
/// that order; ↑ ↓ change the selected one.
fn equalizer(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let p = app.prefs.clone();
    let bypass = nori_core::settings::eq_bypass(p.bit_perfect, p.sound_bypass);
    let head: &str = match bypass {
        Some(why) => crate::text::eq_bypass(why),
        None if p.eq_enabled => "On · heard at once while this is open",
        None => "Off · switch it on to hear the bands",
    };
    let area = heading(f, area, "Equalizer", head, t);
    if area.height < 6 || area.width < 20 {
        return;
    }
    let rows = crate::settings_view::eq_rows(&p);
    app.eq_sel.at = app.eq_sel.at.min(rows.len().saturating_sub(1));
    let chosen = app.eq_sel.at;
    let top: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].above_bands()).collect();
    let bands: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].band(&p).is_some()).collect();
    let under: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].above_bands() && rows[i].band(&p).is_none()).collect();
    let boxed = area.height >= 26;
    let chip = |i: usize| rows[i].chip(&p);
    let top_h = chips(f, area, &top, &chip, chosen, focused, boxed, &mut app.hits, t, &|i| lit(&rows[i], &p));
    let hint = "← → choose · ↑ ↓ change · enter switch · d remove a band";
    let under_h = chips_height(area.width, &under, &chip, boxed);
    let bottom = area.y + area.height;
    let under_area = Rect { y: bottom.saturating_sub(under_h + 1), height: under_h, ..area };
    text(f, Rect { y: bottom - 1, height: 1, ..area }, hint, t.dim());
    chips(f, under_area, &under, &chip, chosen, focused, boxed, &mut app.hits, t, &|i| lit(&rows[i], &p));
    let mid = Rect { y: area.y + top_h + 1, height: under_area.y.saturating_sub(area.y + top_h + 2), ..area };
    let on_ = p.eq_enabled && bypass.is_none();
    sliders(f, mid, &rows, &bands, &p, chosen, on_, app, t);
}

/// Whether a chip's switch is on.
fn lit(row: &EqRow, p: &nori_core::settings::StoredPrefs) -> bool {
    match row {
        EqRow::Enabled => p.eq_enabled,
        EqRow::AutoPreamp => p.eq_preamp_db.is_none(),
        EqRow::Mono => p.mono,
        EqRow::Limiter => p.limiter,
        _ => false,
    }
}

/// Rows the wrapped chips take in `w` columns.
fn chips_height(w: u16, which: &[usize], label: &dyn Fn(usize) -> String, boxed: bool) -> u16 {
    let (mut x, mut lines) = (0u16, 1u16);
    for &i in which {
        let cw = label(i).width() as u16 + if boxed { 4 } else { 2 };
        if x > 0 && x + cw > w {
            lines += 1;
            x = 0;
        }
        x += cw + 1;
    }
    if which.is_empty() {
        return 0;
    }
    lines * if boxed { 3 } else { 1 }
}

/// Wrapped chips; the selected one raised, switches that are on in the `on` color. Returns the rows used.
#[allow(clippy::too_many_arguments)]
fn chips(f: &mut Frame, area: Rect, which: &[usize], label: &dyn Fn(usize) -> String, chosen: usize, focused: bool, boxed: bool, hits: &mut Vec<(Rect, Hit)>, t: &Theme, on_: &dyn Fn(usize) -> bool) -> u16 {
    let line_h = if boxed { 3 } else { 1 };
    let (mut x, mut y) = (area.x, area.y);
    for &i in which {
        let text = label(i);
        let cw = text.width() as u16 + if boxed { 4 } else { 2 };
        if x > area.x && x + cw > area.x + area.width {
            x = area.x;
            y += line_h;
        }
        if y + line_h > area.y + area.height.max(line_h) {
            break;
        }
        let r = Rect { x, y, width: cw.min(area.width), height: line_h };
        let is = i == chosen;
        // A switch that is on takes the `on` color; the chosen chip is raised.
        let mut style = Style::default().fg(if on_(i) { t.on } else { t.ink });
        if is {
            style = style.patch(t.select(focused));
        }
        if boxed {
            let border = if is { Style::default().fg(t.ink) } else { Style::default().fg(t.hair) };
            let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border);
            let inner = block.inner(r);
            put(f, block, r);
            put(f, Paragraph::new(Span::styled(format!(" {text} "), style)), inner);
        } else {
            put(f, Paragraph::new(Span::styled(format!(" {text} "), style)), r);
        }
        hits.push((r, Hit::Row(ListRef::Eq, i)));
        x += cw + 1;
    }
    (y + line_h).saturating_sub(area.y)
}

/// Band faders: a track filled from 0 dB to the knob, frequency and gain below.
#[allow(clippy::too_many_arguments)]
fn sliders(f: &mut Frame, area: Rect, rows: &[EqRow], bands: &[usize], p: &nori_core::settings::StoredPrefs, chosen: usize, on_: bool, app: &mut App, t: &Theme) {
    if bands.is_empty() || area.height < 6 || area.width < 16 {
        return;
    }
    let range = nori_core::settings::EQ_RANGES.gain.max;
    // An odd height so 0 dB gets its own middle row; two rows below for labels.
    let h = (area.height - 2).min(25);
    let h = if h % 2 == 0 { h - 1 } else { h };
    let mid = h / 2;
    let top = area.y;
    app.eq_track = (top, h);
    let n = bands.len() as u16;
    let col = ((area.width.saturating_sub(6)) / n).clamp(4, 10);
    let x0 = area.x + 6 + (area.width.saturating_sub(6 + col * n)) / 2;
    for (y, s) in [(0, format!("+{range:.0}")), (mid, "0 dB".into()), (h - 1, format!("-{range:.0}"))] {
        put(f, Paragraph::new(Span::styled(format!("{s:>5}"), t.dim())), Rect { x: area.x, y: top + y, width: 5, height: 1 });
    }
    let line_w = col * n;
    put(f, Paragraph::new(Span::styled("┈".repeat(line_w as usize), t.dim())), Rect { x: x0, y: top + mid, width: line_w, height: 1 });
    for (k, &i) in bands.iter().enumerate() {
        let Some((_, gain)) = rows[i].band(p) else { continue };
        let x = x0 + k as u16 * col;
        let c = x + (col - 1) / 2;
        let is = i == chosen;
        let fill_colour = if !on_ { t.quiet } else if is { t.accent } else { t.ink };
        let knob_y = (mid as f32 - (gain / range).clamp(-1.0, 1.0) * mid as f32).round() as u16;
        for y in 0..h {
            let (sym, style) = if y == knob_y {
                (if col >= 5 { "━●━" } else { "●" }, Style::default().fg(if is { t.accent } else { fill_colour }).add_modifier(Modifier::BOLD))
            } else if (y > knob_y && y < mid) || (y < knob_y && y > mid) || (y == mid && knob_y != mid) {
                ("┃", Style::default().fg(fill_colour))
            } else {
                ("│", t.dim())
            };
            let w = sym.width() as u16;
            put(f, Paragraph::new(Span::styled(sym, style)), Rect { x: c + 1 - w.div_ceil(2), y: top + y, width: w, height: 1 });
        }
        let (name, _) = rows[i].words(p);
        let label_style = if is { t.current() } else { t.dim() };
        let cw = (col - 1) as usize;
        let centre = |s: &str| format!("{:^cw$}", fit(s, cw));
        put(f, Paragraph::new(Span::styled(centre(&name), label_style)), Rect { x, y: top + h, width: col - 1, height: 1 });
        let value = nori_core::numbers::signed_db(gain, nori_core::numbers::POINT);
        let value_style = if is { Style::default().fg(t.accent) } else { t.dim() };
        put(f, Paragraph::new(Span::styled(centre(&value), value_style)), Rect { x, y: top + h + 1, width: col - 1, height: 1 });
        app.hits.push((Rect { x, y: top, width: col, height: h + 2 }.intersection(area), Hit::Row(ListRef::Eq, i)));
    }
}

fn settings(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, focused: bool) {
    let prefs = app.prefs.clone();
    let own = &app.settings.own;
    app.settings.own = settings_view::Own { mouse: app.mouse, images: app.images, card_covers: app.card_covers, paint: app.paint, protocol: app.protocol.to_string(), data: own.data.clone(), device: own.device.clone() };
    let body = heading(f, area, "Settings", "enter switches or opens · ← → change · [ ] groups", t);
    let App { settings: v, hits, .. } = app;
    let pages = v.pages(&prefs).to_vec();
    let lines = SettingsView::lines(&pages);
    let len = lines.len();
    if v.row.at == 0 && len > 0 {
        v.skip_titles(true);
    }
    // Group index on the left when wide enough.
    let body = if body.width >= 96 {
        let at = v.group_at();
        for (g, group) in settings_view::GROUPS.iter().enumerate() {
            let r = Rect { x: body.x, y: body.y + g as u16, width: 18, height: 1 };
            if r.y >= body.y + body.height {
                break;
            }
            let style = if g == at { t.current() } else { t.dim() };
            let mark = if g == at { "▌ " } else { "  " };
            put(f, Paragraph::new(Line::from(vec![Span::styled(mark, style), Span::styled(group.title, style)])), r);
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
        SLine::Group(title) => return Line::from(vec![Span::styled(format!("{title}  "), t.current()), Span::styled("─".repeat(w.saturating_sub(title.width() + 3)), t.dim())]),
        SLine::Title(title) => return Line::from(Span::styled(format!("  {title}"), bold().fg(t.ink))),
        SLine::Row(r) => *r,
    };
    let (title, detail) = settings_view::row_words(row);
    let live = settings_view::row_enabled(row);
    let text_style = if live { Style::default().fg(t.ink) } else { t.dim() };
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
        settings_view::Row::Toggle { on, .. } | settings_view::Row::Ranked { on, .. } => (if *on { "━━●" } else { "○──" }).to_string(),
        _ => settings_view::row_value(row),
    };
    let mut left = vec![Span::styled(format!("    {title}"), text_style)];
    if !detail.is_empty() {
        left.push(Span::styled(format!("  {detail}"), t.dim()));
    }
    let switched = matches!(row, settings_view::Row::Toggle { on: true, .. } | settings_view::Row::Ranked { on: true, .. });
    let value_style = if !live { t.dim() } else if switched { Style::default().fg(t.on) } else { Style::default().fg(t.ink) };
    spread(left, Span::styled(value + " ", value_style), w)
}

/// Where the music plays: this computer first, then the account's other devices with what each plays,
/// the one playing ticked; then the jam.
fn devices(f: &mut Frame, area: Rect, app: &mut App, focused: bool, t: &Theme) {
    text(f, area, crate::text::PLAY_ON, bold().fg(t.ink));
    let rows = app.devices.rows();
    let body = Rect { y: area.y + 2, height: area.height.saturating_sub(2).min(rows.len() as u16), ..area };
    let App { devices, hits, .. } = app;
    let d = &*devices;
    let active = d.active.as_ref().map(|(id, _)| id.as_str());
    let tick = |on: bool| Span::styled(if on { "✓ " } else { "  " }, t.current());
    let name = |s: &str, on: bool| Span::styled(s.to_string(), if on { t.current() } else { bold().fg(t.ink) });
    let listeners = d.listeners().len();
    let mut sel = d.sel;
    list(f, body, &mut sel, rows.len(), ListRef::Devices, hits, t, focused, &|i, w| match rows[i] {
        DeviceRow::Here => Line::from(vec![tick(active.is_none()), name(crate::text::THIS_COMPUTER, active.is_none())]),
        DeviceRow::Device(k) => {
            let x = &d.list[k];
            let on = active == Some(x.id.as_str());
            spread(vec![tick(on), name(&x.name, on)], Span::styled(crate::text::device_kind(x.kind), t.dim()), w)
        }
        DeviceRow::Playing(k) => {
            let x = &d.list[k];
            Line::from(Span::styled(fit(&format!("  {}", crate::text::device_line(x)), w).into_owned(), if x.refused.is_some() { Style::default().fg(Color::LightRed) } else { t.dim() }))
        }
        DeviceRow::Jam if d.jam.is_some() => spread(vec![Span::styled("◉ ", t.current()), name(&crate::text::jam_strip(listeners), true)], Span::styled(crate::text::JAM_INVITE, t.dim()), w),
        DeviceRow::Jam => Line::from(vec![Span::styled("◉ ", t.dim()), Span::styled(crate::text::JAM_START, Style::default().fg(t.ink))]),
        DeviceRow::Join => Line::from(vec![Span::styled("+ ", t.dim()), Span::styled(crate::text::JAM_JOIN, Style::default().fg(t.ink))]),
        DeviceRow::Pair => Line::from(crate::text::PAIR_DEVICES),
        DeviceRow::ResetPairing => Line::from(crate::text::FORGET_DEVICES),
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
        lines.push(Line::from(vec![Span::styled("◇ ", t.dim()), Span::styled(fit(&heard, w.saturating_sub(2)).into_owned(), Style::default().fg(t.ink))]));
        if let Some(n) = &app.mixed_in {
            lines.push(Line::from(Span::styled(fit(&format!("  {}", how_mixed(n)), w).into_owned(), t.dim())));
        }
    }
    if let Some(n) = &app.transition {
        let next = crate::backend::app().song(&n.incoming_id).map(|s| s.title).unwrap_or_default();
        lines.push(Line::from(vec![Span::styled("⇢ ", t.dim()), Span::styled(fit(&format!("Next: “{next}”"), w.saturating_sub(2)).into_owned(), Style::default().fg(t.ink))]));
        lines.push(Line::from(Span::styled(fit(&format!("  {}", how_mixed(n)), w).into_owned(), t.dim())));
        if !n.reason.is_empty() {
            lines.push(Line::from(Span::styled(fit(&format!("  {}", n.reason), w).into_owned(), t.dim())));
        }
    } else if app.prefs.auto_mix || app.prefs.crossfade_sec > 0 {
        let what = if app.prefs.auto_mix { "AutoMix" } else { "Crossfade" };
        lines.push(Line::from(Span::styled(fit(&format!("⇢ {what} on · planned as this plays"), w).into_owned(), t.dim())));
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

fn queue(f: &mut Frame, area: Rect, app: &mut App, focused: bool, t: &Theme) {
    let rows = app.queue_rows();
    if rows.is_empty() {
        let lines = vec![Line::from(Span::styled("The queue is empty", bold())), Line::from(Span::styled("enter plays, a adds to the queue", t.dim()))];
        put(f, Paragraph::new(lines).alignment(Alignment::Center), centred(area, area.width, 2));
        return;
    }
    if let Some(q) = &app.queue {
        let secs: u64 = q.songs.iter().map(|s| s.duration as u64).sum();
        let mut modes = String::new();
        if q.shuffle {
            modes.push_str(" · shuffle");
        }
        match q.repeat {
            nori_core::rules::REPEAT_ONE => modes.push_str(" · repeat one"),
            nori_core::rules::REPEAT_ALL => modes.push_str(" · repeat all"),
            _ => {}
        }
        text(f, Rect { height: 1, ..area }, &format!("{}{modes}", crate::text::songs_caption(q.len, secs)), t.dim());
    }
    let body = Rect { y: area.y + 2, height: area.height.saturating_sub(2), ..area };
    let listeners: Vec<String> = app.devices.listeners().iter().map(|m| crate::text::jam_member(&m.name, m.role)).collect();
    let note = app.downloads_note();
    let along = app.along_note();
    let chosen = app.queue_sel.at;
    let App { queue, queue_sel, hits, devices, .. } = app;
    let current = queue.as_ref().map_or(-1, |q| q.index);
    let by_hand: HashSet<u32> = queue.as_ref().map_or_else(HashSet::new, |q| q.queued.iter().copied().collect());
    let asks = devices.asks();
    let hosting = devices.jam.as_ref().is_some_and(|j| j.hosting);
    let strip = devices.jam_strip().unwrap_or_default();
    let here = devices.listening() != nori_core::remote::Listening::Watching;
    let playing_here = devices.listening() == nori_core::remote::Listening::Playing;
    let accent = t.dim();
    let cur = t.current();
    list(f, body, queue_sel, rows.len(), ListRef::Queue, hits, t, focused, &|row, w| match rows[row] {
        QueueRow::Jam => spread(vec![Span::styled("◉ ", cur), Span::styled(strip.clone(), cur)], Span::styled(if hosting { crate::text::JAM_INVITE } else { "" }, t.dim()), w),
        QueueRow::Listeners => Line::from(Span::styled(fit(&format!("  {}", listeners.join(", ")), w).into_owned(), Style::default().fg(t.ink))),
        QueueRow::Listen => {
            // Asked for, it is lit whether or not the host lets it play (the note under it says why not).
            let words = if playing_here { crate::text::PLAYING_HERE } else { crate::text::LISTEN_HERE };
            let (keys, style) = if here { (crate::text::STOP_KEYS, cur) } else { (crate::text::LISTEN_KEYS, Style::default().fg(t.ink)) };
            spread(vec![Span::styled("  ♪ ", accent), Span::styled(words, style)], Span::styled(if row == chosen { keys } else { "" }, t.dim()), w)
        }
        QueueRow::AlongNote(n) => Line::from(Span::styled(format!("{:>w$}{}", "", along[n], w = crate::app::QUEUE_NOTE_INDENT), t.dim())),
        QueueRow::Asking => Line::from(Span::styled(format!("  {}", crate::text::YOU_ASKED), t.dim().add_modifier(Modifier::BOLD))),
        // The keys show on the request chosen.
        QueueRow::Ask(k) => {
            let p = asks[k];
            let keys = if hosting && rows.get(chosen).and_then(|r| r.ask()) == Some(k) { crate::text::JAM_DECIDE } else { "" };
            let who = if hosting { crate::text::jam_asked(&p.from_name) } else { crate::text::jam_waiting(devices.host()) };
            let asked = vec![Span::styled("  ? ", accent), Span::styled(p.song.title.clone(), Style::default().fg(t.ink)), Span::styled(who, t.dim())];
            spread(asked, Span::styled(keys, t.dim()), w)
        }
        QueueRow::Downloads(_, n) => {
            let mark = if n == 0 { "☁ " } else { "  " };
            Line::from(Span::styled(format!("{:>w$}{}", mark, note[n], w = crate::app::QUEUE_NOTE_INDENT), t.dim()))
        }
        QueueRow::End => Line::from(vec![Span::styled("  ✕ ", t.dim()), Span::styled(if hosting { crate::text::JAM_END } else { crate::text::JAM_LEAVE }, t.dim())]),
        QueueRow::Song(i) => {
            let Some(s) = queue.as_ref().and_then(|q| q.songs.get(i)) else { return Line::from("") };
            let playing = i as i32 == current;
            let mark = if playing { "▶ " } else if by_hand.contains(&(i as u32)) { "+ " } else { "  " };
            let title_style = if playing { t.current() } else { Style::default().fg(t.ink) };
            // Who asked for it before the artist, which is cut first.
            let by = devices.added.get(&s.id).map(|by| Span::styled(format!(" · {by}"), t.dim()));
            let spans = [Span::styled(mark, if playing { cur } else { accent }), Span::styled(s.title.clone(), title_style)].into_iter().chain(by).chain([Span::styled(format!("  {}", s.artist), t.dim())]);
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
            let mut row = Line::from(spans).alignment(Alignment::Center);
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

/// The full-window player: the cover, the title with its seal, the seek bar, the transport and the volume on
/// the left, the lyrics, queue or devices in an outline beside it. A narrow window gives the panel the room
/// under the song.
fn full_player(f: &mut Frame, area: Rect, app: &mut App, art: Option<&mut Art>) {
    let t = app.theme;
    put(f, Block::default().style(t.ground()), area);
    if area.width < 10 || area.height < 4 {
        return;
    }
    // Closing is the top edge's.
    put(f, Paragraph::new(Span::styled("⌄", t.dim())).alignment(Alignment::Center), Rect { height: 1, ..area });
    app.hits.push((Rect { height: 2.min(area.height), ..area }, Hit::Button(Button::Full)));
    let m = crate::chrome::margin(area.width) + 2;
    let inner = Rect { x: area.x + m, y: area.y + 1, width: area.width.saturating_sub(2 * m), height: area.height.saturating_sub(2) };
    if app.panel == Panel::Playing {
        // The song alone, as wide as its cover, under the panels' names.
        let tabs = panel_tabs(app);
        let tabs_w = tabs.iter().map(|t| t.1.width() as u16).sum::<u16>() + 4 * tabs.len().saturating_sub(1) as u16;
        draw_panel_tabs(f, app, &t, tabs, inner.x + inner.width.saturating_sub(tabs_w) / 2, inner.y, inner.x + inner.width);
        let w = (inner.height.saturating_sub(2 + SONG_BELOW).min(22) * 2).max(40).min(inner.width);
        let column = Rect { x: inner.x + (inner.width - w) / 2, y: inner.y + 2, width: w, height: inner.height.saturating_sub(2) };
        song_column(f, column, app, &t, art);
        return;
    }
    let wide = inner.width >= 84;
    let left_w = if wide { (inner.width * 2 / 5).clamp(32, 46) } else { inner.width };
    let left = Rect { width: left_w, ..inner };
    let used = song_column(f, left, app, &t, art);
    let panel = if wide {
        Rect { x: left.x + left.width + 4, width: inner.width.saturating_sub(left.width + 4), ..inner }
    } else {
        Rect { y: inner.y + used + 1, height: inner.height.saturating_sub(used + 1), ..inner }
    };
    if panel.height >= 5 && panel.width >= 20 {
        side_panel(f, panel, app, &t);
    }
}

/// Rows under the full player's cover: title, artist, a blank, seek, a blank, transport, a blank, volume.
const SONG_BELOW: u16 = 8;

/// The song in the full player's left column: returns the rows it took.
fn song_column(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, art: Option<&mut Art>) -> u16 {
    let Some(song) = app.song.clone() else {
        text(f, Rect { y: area.y + 1, height: 1, ..area }, "Nothing playing · esc goes back", t.dim());
        return 2;
    };
    let cover_rows = if app.images && art.is_some() { area.height.saturating_sub(SONG_BELOW).min(area.width / 2).min(22) } else { 0 };
    let mut y = area.y;
    if cover_rows >= 6 {
        let c = Rect { x: area.x, y, width: cover_rows * 2, height: cover_rows };
        cover(f, c, art, song.cover_art.as_deref(), t);
        y += cover_rows + 1;
    }
    let w = area.width;
    let row = |y: u16| Rect { y, height: 1, ..area };
    // The title with the seal at the right end.
    let sealed = app.rules.stars;
    let title_w = w.saturating_sub(if sealed { 4 } else { 0 });
    text(f, Rect { width: title_w, ..row(y) }, &song.title, bold().fg(t.ink));
    if sealed {
        let liked = app.starred(Starrable::Song, &song.id, song.starred);
        let r = Rect { x: area.x + w - 2, width: 2, ..row(y) };
        let flash = liked && app.flash.is_some_and(|(b, _)| b == Button::StarSong);
        put(f, Paragraph::new(seal(liked, flash, t)), r);
        app.hits.push((Rect { x: r.x - 1, width: 3, ..r }, Hit::Button(Button::StarSong)));
    }
    y += 1;
    let by = [song.artist.as_str(), song.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    text(f, row(y), &by, Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
    y += 1;
    if let Some(q) = crate::text::quality(&song) {
        text(f, row(y), &q, t.dim());
        y += 1;
    }
    y += 1;
    let time = 5;
    let track_w = w.saturating_sub(2 * (time + 1)).min(60);
    let x = area.x;
    seek_line(f, app, t, [Rect { x, width: time, ..row(y) }, Rect { x: x + time + 1, width: track_w, ..row(y) }, Rect { x: x + time + 2 + track_w, width: time, ..row(y) }]);
    y += 2;
    let buttons = offered_buttons(app);
    let step = (w.saturating_sub(2) / buttons.len().max(1) as u16).clamp(4, 8);
    let span = step * buttons.len().saturating_sub(1) as u16 + 1;
    let x0 = area.x + w.saturating_sub(span) / 2;
    for (i, b) in buttons.iter().enumerate() {
        transport_button(f, app, t, *b, Rect { x: x0 + i as u16 * step, width: 1, ..row(y) });
    }
    y += 2;
    text(f, Rect { width: 2, ..row(y) }, speaker(app.volume), t.dim());
    let vol = Rect { x: area.x + 3, width: 16.min(w.saturating_sub(3)), ..row(y) };
    track_bar(f, vol, app.volume, t);
    app.volume_rect = vol;
    app.hits.push((vol, Hit::Volume));
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

/// The panel beside the song: its tabs on the raised surface, a hairline, then the lyrics, the queue or
/// the devices.
fn side_panel(f: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(t.line)).style(t.raised());
    put(f, block, area);
    // Inside, the raised surface is the page, and a selected row one step above it.
    let mut p = *t;
    p.page = t.surface;
    if t.painted {
        p.surface = crate::art::blend(t.surface, t.ink, 0.06);
    }
    let inner = Rect { x: area.x + 3, y: area.y + 1, width: area.width.saturating_sub(6), height: area.height.saturating_sub(2) };
    app.shown.panel_w = inner.width;
    let tabs = panel_tabs(app);
    draw_panel_tabs(f, app, t, tabs, inner.x, inner.y, inner.x + inner.width);
    put(f, Paragraph::new(Span::styled("─".repeat(area.width.saturating_sub(2) as usize), Style::default().fg(t.hair))), Rect { x: area.x + 1, y: inner.y + 1, width: area.width.saturating_sub(2), height: 1 });
    let body = Rect { y: inner.y + 3, height: inner.height.saturating_sub(3), ..inner };
    let focused = app.focus == Focus::Panel;
    match app.panel {
        Panel::Lyrics => lyrics(f, body, app, &p),
        Panel::Queue => queue(f, body, app, focused, &p),
        Panel::Devices => devices(f, body, app, focused, &p),
        Panel::Playing => {}
    }
}

/// The full player's panels with their names: the devices only while other devices can play.
fn panel_tabs(app: &App) -> Vec<(Panel, String)> {
    PANELS.iter().filter(|(p, _)| *p != Panel::Devices || app.devices.on).map(|(p, name)| (*p, format!("{} {name}", panel_icon(*p)))).collect()
}

/// `tabs` from `x` on row `y`, up to `end`; the open one underlined in the accent.
fn draw_panel_tabs(f: &mut Frame, app: &mut App, t: &Theme, tabs: Vec<(Panel, String)>, mut x: u16, y: u16, end: u16) {
    for (which, label) in tabs {
        let w = label.width() as u16;
        if x + w > end {
            break;
        }
        let style = if which == app.panel { t.current().add_modifier(Modifier::UNDERLINED) } else { t.dim() };
        let r = Rect { x, y, width: w, height: 1 };
        put(f, Paragraph::new(Span::styled(label, style)), r);
        app.hits.push((Rect { x: x.saturating_sub(1), width: w + 2, ..r }, Hit::Button(Button::Panel(which))));
        x += w + 4;
    }
}

fn panel_icon(p: Panel) -> &'static str {
    match p {
        Panel::Playing => "♫",
        Panel::Queue => "≡",
        Panel::Lyrics => "❝",
        Panel::Devices => "⇄",
    }
}

/// The volume's speaker: struck through when muted, its waves by how loud.
fn speaker(volume: f32) -> &'static str {
    match volume {
        v if v <= 0.0 => "🔇",
        v if v < 0.34 => "🔈",
        v if v < 0.67 => "🔉",
        _ => "🔊",
    }
}

// ---- the player bar ----

/// The seal: not liked the symbol in quiet ink, liked the symbol on the accent (`flash`: the one frame of
/// the stamp, inverted). Two cells wide.
pub fn seal(liked: bool, flash: bool, t: &Theme) -> Span<'static> {
    let style = if liked { t.liked() } else { t.dim() };
    let style = if flash { style.add_modifier(Modifier::REVERSED) } else { style };
    Span::styled("好", style)
}

/// The player: the song on the left, shuffle, previous, play, next and repeat with the seek bar under them in
/// the middle, and on the right the seal, the playing, queue, lyrics, devices and full screen buttons, and
/// the volume.
fn player_bar(f: &mut Frame, area: Rect, app: &mut App, t: &Theme, art: Option<&mut Art>) {
    let m = crate::chrome::margin(area.width);
    let frame = Rect { x: area.x + m, width: area.width.saturating_sub(2 * m), ..area };
    if frame.width < 12 {
        return;
    }
    outline(f, frame, t);
    // On the top edge: where the music plays while it is another device, and the jam hosted.
    let mut said: Vec<String> = Vec::new();
    if let Some((_, name)) = &app.devices.active {
        said.push(format!("⇄ {}", crate::text::playing_on(name)));
    }
    if let Some(strip) = app.devices.jam_strip() {
        said.push(format!("◉ {strip}"));
    }
    if !said.is_empty() {
        let line = format!(" {} ", said.join(" · "));
        let r = Rect { x: frame.x + 3, y: frame.y, width: (line.width() as u16).min(frame.width.saturating_sub(6)), height: 1 };
        put(f, Clear, r);
        text(f, r, &line, t.current());
        // A guest's jam is in the queue panel.
        app.hits.push((r, Hit::Button(Button::Panel(if app.guest() { Panel::Queue } else { Panel::Devices }))));
    }
    let buttons = offered_buttons(app);
    let covers = app.images && art.is_some();
    let bar = crate::chrome::bar(frame, &buttons, covers, app.devices.on);
    let flash = app.flash.map(|(b, _)| b);
    // The song, and a click on it opens the full player.
    if let Some(c) = bar.cover {
        let key = app.song.as_ref().and_then(|s| s.cover_art.clone());
        cover(f, c, art, key.as_deref(), t);
    }
    let open = Rect { x: bar.cover.map_or(bar.song[0].x, |c| c.x), width: bar.song[0].x + bar.song[0].width - bar.cover.map_or(bar.song[0].x, |c| c.x), ..bar.song[0] };
    match &app.song {
        Some(s) => {
            let artist = if s.album.is_empty() { s.artist.clone() } else { format!("{} · {}", s.artist, s.album) };
            if bar.song[1].width > 0 {
                text(f, bar.song[0], &s.title, bold().fg(t.ink));
                text(f, bar.song[1], &artist, t.dim());
            } else {
                let line = Line::from(vec![Span::styled(s.title.clone(), bold().fg(t.ink)), Span::styled(format!("  {}", s.artist), t.dim())]);
                put(f, Paragraph::new(line), bar.song[0]);
            }
            app.hits.push((Rect { height: bar.song[1].y.max(bar.song[0].y) + 1 - bar.song[0].y, ..open }, Hit::Button(Button::Full)));
        }
        None => text(f, bar.song[0], "Nothing playing", t.dim()),
    }
    for (b, r) in &bar.buttons {
        transport_button(f, app, t, *b, *r);
    }
    if let Some(clock) = bar.clock {
        seek_line(f, app, t, clock);
    }
    if let (Some(r), true) = (bar.seal, app.rules.stars) {
        if let Some(s) = &app.song {
            let liked = app.starred(Starrable::Song, &s.id, s.starred);
            put(f, Paragraph::new(seal(liked, liked && flash == Some(Button::StarSong), t)), Rect { x: r.x + 1, width: 2, ..r });
            app.hits.push((r, Hit::Button(Button::StarSong)));
        }
    }
    for (b, r) in &bar.views {
        let (glyph, on) = match b {
            Button::Panel(p) => (panel_icon(*p), *p == Panel::Devices && app.devices.active.is_some()),
            _ => ("⤢", false),
        };
        put(f, Paragraph::new(Span::styled(glyph, if on { t.current() } else { t.dim() })), *r);
        app.hits.push((Rect { x: r.x.saturating_sub(1), width: 3, ..*r }, Hit::Button(*b)));
    }
    if let Some([icon, track]) = bar.volume {
        text(f, icon, speaker(app.volume), t.dim());
        track_bar(f, track, app.volume, t);
        app.volume_rect = track;
        app.hits.push((track, Hit::Volume));
    }
}

/// The transport buttons the player offers, in order. A jam guest's are those its role offers: shuffle and
/// repeat are the host's.
fn offered_buttons(app: &App) -> Vec<Button> {
    let offered = |b: Button| match b {
        Button::Toggle => app.offers(|c| c.play_pause),
        Button::Previous | Button::Next => app.offers(|c| c.skip),
        _ => !app.guest(),
    };
    [Button::Shuffle, Button::Previous, Button::Toggle, Button::Next, Button::Repeat].into_iter().filter(|b| offered(*b)).collect()
}

/// A transport glyph at `r`: bare, shuffle and repeat lit in the accent while on, reversed for a moment after
/// a press.
fn transport_button(f: &mut Frame, app: &mut App, t: &Theme, b: Button, r: Rect) {
    let lit = |on: bool| if on { t.current() } else { t.dim() };
    let playing = app.devices.controls.map_or(app.now.state == State::Playing, |c| c.playing);
    let (glyph, style) = match b {
        Button::Shuffle => ("⤮", lit(app.queue_shuffled())),
        Button::Previous => ("⏮", Style::default().fg(t.ink)),
        Button::Toggle => (if app.now.buffering { "⋯" } else if playing { "⏸" } else { "▶" }, Style::default().fg(t.ink)),
        Button::Next => ("⏭", Style::default().fg(t.ink)),
        _ => (if app.repeat() == nori_core::rules::REPEAT_ONE { "↻¹" } else { "↻" }, lit(app.repeat() != nori_core::rules::REPEAT_OFF)),
    };
    let style = if app.flash.is_some_and(|(p, _)| p == b) { style.add_modifier(Modifier::REVERSED) } else { style };
    let r = Rect { width: glyph.width() as u16, height: 1, ..r };
    put(f, Paragraph::new(Span::styled(glyph, style)), r);
    app.hits.push((Rect { x: r.x.saturating_sub(1), width: r.width + 2, ..r }, Hit::Button(b)));
}

/// The elapsed time, the seek bar and the time left in `[a, track, b]`; the bar takes the seek clicks.
fn seek_line(f: &mut Frame, app: &mut App, t: &Theme, [a, track, b]: [Rect; 3]) {
    let len = app.song.as_ref().map_or(0, |s| s.duration as i64 * 1000);
    let pos = match app.drag {
        Some(crate::app::Drag::Seek(share)) => (share * len as f32) as i64,
        _ => app.now.position(Instant::now()).clamp(0, len.max(0)),
    };
    app.shown.clock = true;
    text(f, a, &clock(pos), t.dim());
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
    let h = 17 + if servers > 0 { servers + 2 } else { 0 };
    let r = centred(area, 64, h);
    let inner = popup(f, r, Span::styled(" ♫ nori · connect to a server ", bold().fg(t.ink)), t);
    let l = &app.login;
    let bottom = inner.y + inner.height;
    let mut y = inner.y + 1;
    text(f, Rect { x: inner.x + 2, y, width: inner.width.saturating_sub(4), height: 1 }, "Navidrome, octo-fiesta or any Subsonic server", t.dim());
    y += 2;
    for (i, label) in LOGIN_FIELDS.iter().enumerate() {
        if y >= bottom {
            break;
        }
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
    y += 2;
    let msg = match &l.error {
        Some(e) => Span::styled(format!("  {e}"), Style::default().fg(Color::LightRed)),
        None => Span::styled("  tab next field · enter connect · esc back", t.dim()),
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
    let App { overlay, hits, devices, .. } = app;
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
            lines.push(Line::from(Span::styled("The mouse: click the tabs, cards, rows and buttons; click or drag the seek bar and the volume; the wheel scrolls.", t.dim())));
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
            let name = crate::text::provider(*kind).name;
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
                let why = if *busy { crate::text::SIGNING_IN } else if *ended { crate::text::SIGNED_OUT_AGAIN } else { crate::text::SIGN_IN_WHY };
                said.push((why.into(), Style::default().fg(t.ink)));
                said.push(((if *ended { crate::text::SIGN_IN_KEYS_ENDED } else { crate::text::SIGN_IN_KEYS }).into(), t.dim()));
            }
            if let Some(e) = error {
                said.push((e.clone(), Style::default().fg(Color::LightRed)));
            }
            said.push((crate::text::RISK.into(), t.dim()));
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
        Overlay::Input { title, text: typed, secret, .. } => {
            let r = centred(area, 60, 3);
            let inner = popup(f, r, Span::styled(format!(" {title} · enter keeps, esc drops "), bold().fg(t.ink)), t);
            let shown = if *secret { "•".repeat(typed.chars().count()) } else { typed.clone() };
            put(f, Paragraph::new(format!(" {shown}▏")), inner);
        }
    }
}
