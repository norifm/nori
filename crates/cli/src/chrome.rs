//! Where the chrome goes: the rounded outline along the top (tabs, search, settings) and the one along
//! the bottom (the player), at any window size. Pure sums over rectangles, so the drawing and the mouse
//! hits share them and tests can check every size.

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::{Button, Panel, View};

/// The rows of a window: the top outline, the page, the player's outline and the hint line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows {
    pub top: Rect,
    pub page: Rect,
    pub bar: Rect,
    pub foot: Rect,
}

/// Splits `area`. The top outline is three rows (one on a tiny window, without its box), the player's six
/// (three when short, none when tiny), the hint line one row.
pub fn rows(area: Rect) -> Rows {
    let h = area.height;
    let foot = if h >= 12 { 1 } else { 0 };
    let bar = match h {
        18.. => 6,
        10..=17 => 3,
        _ => 0,
    };
    let top = if h >= 10 { 3 } else { 1.min(h) };
    let page = h.saturating_sub(top + bar + foot);
    let at = |y: u16, height: u16| Rect { y: area.y + y, height, ..area };
    Rows { top: at(0, top), page: at(top, page), bar: at(top + page, bar), foot: at(top + page + bar, foot) }
}

/// The side margin of the chrome: wide windows leave more.
pub fn margin(width: u16) -> u16 {
    match width {
        110.. => 4,
        90..=109 => 2,
        _ => 1,
    }
}

/// A tab's text: its icon, and with room its name.
pub fn tab_label(view: View, name: bool) -> String {
    if name {
        format!(" {} {} ", view.icon(), view.name())
    } else {
        format!(" {} ", view.icon())
    }
}

/// The top outline: the tabs' box, the search box and the settings box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top {
    pub tabs: Rect,
    /// Each tab's text and where it is; the separator after Home is at `rule`.
    pub items: Vec<(View, Rect)>,
    pub rule: Option<u16>,
    /// Tabs with their names.
    pub names: bool,
    pub search: Option<Rect>,
    /// The search box is only its icon.
    pub search_small: bool,
    pub settings: Option<Rect>,
}

const SEARCH_MIN: u16 = 14;
const SEARCH_MAX: u16 = 24;
const SETTINGS_W: u16 = 5;

/// The width of the tabs' text: labels, a gap between, and the separator and its gap after Home.
fn tabs_width(tabs: &[View], names: bool) -> u16 {
    let labels: u16 = tabs.iter().map(|v| tab_label(*v, names).width() as u16).sum();
    let gaps = tabs.len().saturating_sub(1) as u16;
    labels + gaps + if tabs.len() > 1 { 2 } else { 0 }
}

/// Lays the top outline out in `top` (its rows) for `tabs`. Names go first when the box, search and
/// settings still fit; below that the tabs are icons and the search shrinks to its icon, then goes.
pub fn top(top: Rect, tabs: &[View]) -> Top {
    let m = margin(top.width);
    let avail = top.width.saturating_sub(2 * m);
    let boxed = top.height >= 3;
    // A box adds its border and a pad inside each side.
    let frame = if boxed { 6 } else { 0 };
    let right = |w: u16| Rect { x: top.x + m + avail - w, y: top.y, width: w, height: top.height };
    let names = avail >= tabs_width(tabs, true) + frame + 1 + SEARCH_MIN + 1 + SETTINGS_W;
    let tabs_w = tabs_width(tabs, names) + frame;
    let rest = avail.saturating_sub(tabs_w + 1);
    let (search_w, search_small, settings_w) = if rest >= SEARCH_MIN + 1 + SETTINGS_W {
        ((rest - SETTINGS_W - 1).min(SEARCH_MAX), false, SETTINGS_W)
    } else if rest >= 5 + 1 + SETTINGS_W {
        (5, true, SETTINGS_W)
    } else if rest >= SETTINGS_W {
        (0, false, SETTINGS_W)
    } else {
        (0, false, 0)
    };
    let settings = (settings_w > 0).then(|| right(settings_w));
    let search = (search_w > 0).then(|| Rect { x: settings.map_or(right(search_w).x, |s| s.x - 1 - search_w), ..right(search_w) });
    let tabs_box = Rect { x: top.x + m, y: top.y, width: tabs_w.min(avail), height: top.height };
    let mut x = tabs_box.x + if boxed { 3 } else { 0 };
    let mut items = Vec::new();
    let mut rule = None;
    // What does not fit in the box is left off.
    let end = tabs_box.x + tabs_box.width - if boxed { 3 } else { 0 };
    for (i, v) in tabs.iter().enumerate() {
        let w = tab_label(*v, names).width() as u16;
        let sep = if i == 1 { 2 } else { 0 };
        if x + sep + w > end {
            break;
        }
        if i == 1 {
            rule = Some(x);
            x += 2;
        }
        items.push((*v, Rect { x, y: top.y + boxed as u16, width: w, height: 1 }));
        x += w + 1;
    }
    Top { tabs: tabs_box, items, rule, names, search, search_small, settings }
}

/// The player's outline, its pieces by how wide it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub outline: Rect,
    pub cover: Option<Rect>,
    /// Title row and artist row.
    pub song: [Rect; 2],
    pub buttons: Vec<(Button, Rect)>,
    /// The elapsed time, the seek bar, the time left.
    pub clock: Option<[Rect; 3]>,
    pub seal: Option<Rect>,
    /// After the seal: the full player on the song, the queue, the lyrics and the devices, then full screen.
    pub views: Vec<(Button, Rect)>,
    /// The volume's icon and bar.
    pub volume: Option<[Rect; 2]>,
}

/// Lays the player out in `outline` (its border included) for the `buttons` the player offers, in order,
/// with `devices` when other devices can play. Tall, its lines are the middle two of four, beside a cover
/// four rows high. Narrower, the volume goes, then the buttons after the seal; below 60 columns only the
/// song and play stay.
pub fn bar(outline: Rect, buttons: &[Button], covers: bool, devices: bool) -> Bar {
    let inner = Rect { x: outline.x + 1, y: outline.y + 1, width: outline.width.saturating_sub(2), height: outline.height.saturating_sub(2) };
    let w = inner.width;
    let tall = inner.height >= 4;
    let top = if tall { 1 } else { 0 };
    let row = |r: u16, x: u16, width: u16| Rect { x, y: inner.y + (top + r).min(inner.height.saturating_sub(1)), width, height: 1 };
    let narrow = w < 58;
    let views_on = w >= 78;
    // Tall, the volume is under the seal and its buttons.
    let volume_on = views_on && (tall || w >= 98);
    let right_w = if narrow {
        0
    } else if !views_on {
        6
    } else if volume_on && !tall {
        37
    } else {
        21
    };
    let cover = (covers && tall && w >= 58).then(|| Rect { x: inner.x + 1, y: inner.y, width: 8, height: 4 });
    let song_x = inner.x + 1 + cover.map_or(0, |_| 10);
    let left_w = if narrow { w.saturating_sub(10) } else { (w / 4).clamp(16, 30).max(if cover.is_some() { 22 } else { 0 }) };
    let song_w = (inner.x + left_w).saturating_sub(song_x + 1);
    let artist = if tall { song_w } else { 0 };
    let song = [row(0, song_x, song_w), row(1, song_x, artist)];
    let mid = Rect { x: inner.x + left_w, width: w.saturating_sub(left_w + right_w + 1), ..inner };
    let mut out = Bar { outline, cover, song, buttons: Vec::new(), clock: None, seal: None, views: Vec::new(), volume: None };
    let shown: Vec<Button> = if narrow { buttons.iter().copied().filter(|b| *b == Button::Toggle).collect() } else { buttons.to_vec() };
    if narrow {
        // Only play, at the right.
        out.buttons = shown.iter().map(|b| (*b, row(0, inner.x + w - 4, 3))).collect();
        return out;
    }
    let step = ((mid.width.saturating_sub(4)) / shown.len().max(1) as u16).clamp(5, 7);
    let span = step * shown.len().saturating_sub(1) as u16 + 1;
    let x0 = mid.x + mid.width.saturating_sub(span) / 2;
    out.buttons = shown.iter().enumerate().map(|(i, b)| (*b, row(0, x0 + i as u16 * step, 1))).collect();
    if tall {
        let time = 5;
        let bar_w = mid.width.saturating_sub(2 * (time + 1)).min(40);
        if bar_w >= 6 {
            let total = bar_w + 2 * (time + 1);
            let x = mid.x + mid.width.saturating_sub(total) / 2;
            out.clock = Some([row(1, x, time), row(1, x + time + 1, bar_w), row(1, x + time + 1 + bar_w + 1, time)]);
        }
    }
    if right_w > 0 {
        let rx = inner.x + w - right_w;
        out.seal = Some(row(0, rx, 4));
        if views_on {
            let mut x = rx + 5;
            let panels = [Panel::Playing, Panel::Queue, Panel::Lyrics, Panel::Devices];
            for b in panels.into_iter().filter(|p| *p != Panel::Devices || devices).map(Button::Panel).chain([Button::Full]) {
                out.views.push((b, row(0, x, 1)));
                x += 3;
            }
            if volume_on {
                let (line, x) = if tall { (1, rx + 1) } else { (0, x + 1) };
                out.volume = Some([row(line, x, 2), row(line, x + 3, 12)]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [View; 6] = [View::Home, View::Albums, View::Artists, View::Songs, View::Playlists, View::Favorites];

    fn area(w: u16, h: u16) -> Rect {
        Rect { x: 0, y: 0, width: w, height: h }
    }

    #[test]
    fn rows_fill_the_window_at_any_height() {
        for h in 1..60 {
            let r = rows(area(120, h));
            assert_eq!(r.top.height + r.page.height + r.bar.height + r.foot.height, h, "h {h}");
            assert_eq!(r.top.y + r.top.height, r.page.y);
            assert_eq!(r.page.y + r.page.height, r.bar.y);
            assert_eq!(r.bar.y + r.bar.height, r.foot.y);
        }
        let r = rows(area(120, 36));
        assert_eq!((r.top.height, r.bar.height, r.foot.height, r.page.height), (3, 6, 1, 26));
        assert_eq!(rows(area(80, 24)).page.height, 14);
        assert_eq!((rows(area(80, 12)).bar.height, rows(area(80, 9)).bar.height), (3, 0));
    }

    #[test]
    fn tabs_lose_their_names_below_the_width_they_need() {
        let wide = top(rows(area(120, 36)).top, &ALL);
        assert!(wide.names && wide.search.is_some() && !wide.search_small && wide.settings.is_some());
        assert_eq!(wide.items.len(), 6);
        // Nothing overlaps and everything is inside the margins.
        let mut boxes = vec![wide.tabs, wide.search.unwrap(), wide.settings.unwrap()];
        boxes.sort_by_key(|r| r.x);
        for pair in boxes.windows(2) {
            assert!(pair[0].x + pair[0].width < pair[1].x, "{pair:?}");
        }
        let m = margin(120);
        assert!(boxes[0].x >= m && boxes[2].x + boxes[2].width <= 120 - m);
        for (_, r) in &wide.items {
            assert!(r.x >= wide.tabs.x && r.x + r.width <= wide.tabs.x + wide.tabs.width, "{r:?} in {:?}", wide.tabs);
        }
        for w in [100, 80, 60, 40, 30] {
            let t = top(rows(area(w, 24)).top, &ALL);
            assert!(t.items.iter().all(|(_, r)| r.x + r.width <= w), "width {w}: {t:?}");
            assert!(t.settings.is_some() || w < 40, "settings stay at {w}");
            if w < 100 {
                assert!(!t.names, "icons only at {w}");
            }
        }
        assert!(top(rows(area(80, 24)).top, &ALL).search.is_some());
        assert!(top(rows(area(40, 24)).top, &ALL).search.is_none() || top(rows(area(40, 24)).top, &ALL).search_small);
    }

    #[test]
    fn a_tab_is_found_by_the_cell_under_the_pointer() {
        let t = top(rows(area(120, 36)).top, &ALL);
        for (v, r) in &t.items {
            for x in [r.x, r.x + r.width - 1] {
                assert_eq!(t.items.iter().find(|(_, c)| c.x <= x && x < c.x + c.width).map(|(w, _)| w), Some(v));
            }
            assert_eq!(r.y, 1, "the tabs sit inside the box");
        }
        // A profile without playlists and favorites has four.
        assert_eq!(top(rows(area(120, 36)).top, &ALL[..4]).items.len(), 4);
    }

    #[test]
    fn the_player_drops_pieces_as_it_narrows() {
        use Button::*;
        let buttons = [Shuffle, Previous, Toggle, Next, Repeat];
        let at = |w: u16| bar(Rect { x: 0, y: 0, width: w, height: 6 }, &buttons, true, true);
        let wide = at(120);
        assert!(wide.cover.is_some() && wide.clock.is_some() && wide.seal.is_some() && wide.volume.is_some());
        let views = [Panel(crate::app::Panel::Playing), Panel(crate::app::Panel::Queue), Panel(crate::app::Panel::Lyrics), Panel(crate::app::Panel::Devices), Full];
        assert_eq!(wide.views.iter().map(|v| v.0).collect::<Vec<_>>(), views);
        assert_eq!(wide.buttons.iter().map(|b| b.0).collect::<Vec<_>>(), buttons);
        // The transport is centered in the middle: play is the third of five.
        let play = wide.buttons[2].1;
        let (a, b) = (wide.buttons[0].1, wide.buttons[4].1);
        assert_eq!(play.x - a.x, b.x - play.x);
        let mid = at(90);
        assert!(mid.volume.is_some() && mid.views.len() == 5 && mid.seal.is_some(), "the volume goes under the seal");
        let short = bar(Rect { x: 0, y: 0, width: 90, height: 3 }, &buttons, true, true);
        assert!(short.volume.is_none() && short.views.len() == 5);
        let small = at(70);
        assert!(small.views.is_empty() && small.seal.is_some() && small.clock.is_some());
        let tiny = at(50);
        assert_eq!(tiny.buttons.iter().map(|b| b.0).collect::<Vec<_>>(), [Toggle]);
        assert!(tiny.seal.is_none() && tiny.clock.is_none() && tiny.volume.is_none() && tiny.cover.is_none());
        // Without devices there is no devices button.
        let no = bar(Rect { x: 0, y: 0, width: 120, height: 6 }, &buttons, true, false);
        assert_eq!(no.views.iter().map(|v| v.0).collect::<Vec<_>>(), [views[0], views[1], views[2], Full]);
    }

    #[test]
    fn the_players_lines_sit_in_the_middle_of_its_cover() {
        use Button::*;
        let outline = Rect { x: 0, y: 10, width: 140, height: 6 };
        let b = bar(outline, &[Shuffle, Previous, Toggle, Next, Repeat], true, true);
        let cover = b.cover.expect("a cover");
        let lines: Vec<Rect> = b.buttons.iter().map(|x| x.1).chain(b.views.iter().map(|x| x.1)).chain(b.seal).chain(b.clock.into_iter().flatten()).chain(b.volume.into_iter().flatten()).chain(b.song).collect();
        for r in lines {
            let (above, below) = (r.y - cover.y, cover.y + cover.height - 1 - r.y);
            assert!(above >= 1 && below >= 1, "{r:?} beside {cover:?}");
        }
        assert_eq!(b.song[0].y - cover.y, cover.y + cover.height - 1 - b.song[1].y);
    }

    #[test]
    fn nothing_leaves_the_outline_or_overlaps() {
        use Button::*;
        let buttons = [Shuffle, Previous, Toggle, Next, Repeat];
        for w in 20..200u16 {
            for h in [3u16, 6] {
                let outline = Rect { x: 3, y: 2, width: w, height: h };
                let b = bar(outline, &buttons, true, true);
                let mut all: Vec<Rect> = b.buttons.iter().map(|x| x.1).chain(b.views.iter().map(|x| x.1)).chain(b.seal).chain(b.cover).chain(b.clock.into_iter().flatten()).chain(b.volume.into_iter().flatten()).collect();
                all.extend(b.song);
                for r in &all {
                    assert!(r.x > outline.x && r.x + r.width < outline.x + outline.width && r.y > outline.y && r.y < outline.y + outline.height - 1, "w {w} h {h}: {r:?} outside {outline:?}");
                }
                all.retain(|r| r.width > 0);
                all.sort_by_key(|r| (r.y, r.x));
                for pair in all.windows(2) {
                    if pair[0].y == pair[1].y && pair[0].height == 1 && pair[1].height == 1 {
                        assert!(pair[0].x + pair[0].width <= pair[1].x, "w {w} h {h}: {pair:?}");
                    }
                }
            }
        }
    }
}
