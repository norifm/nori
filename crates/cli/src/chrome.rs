//! Where the chrome goes: one line of tabs along the top, the page, two lines of the song playing and one
//! of keys along the bottom, at any window size. Pure sums over rectangles, so the drawing and the mouse
//! hits share them and tests can check every size.

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::View;

/// The rows of a window: the tabs, the page, the song playing and the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows {
    pub head: Rect,
    pub page: Rect,
    pub now: Rect,
    pub foot: Rect,
}

/// Splits `area`. The tabs and the keys take a row each; the song playing two, one on a short window. A
/// tiny window keeps the page and the song.
pub fn rows(area: Rect) -> Rows {
    let h = area.height;
    let head = u16::from(h >= 4);
    let foot = u16::from(h >= 8);
    let now = match h {
        12.. => 2,
        3.. => 1,
        _ => 0,
    };
    let page = h.saturating_sub(head + now + foot);
    let at = |y: u16, height: u16| Rect { y: area.y + y, height, ..area };
    Rows { head: at(0, head), page: at(head, page), now: at(head + page, now), foot: at(head + page + now, foot) }
}

/// The tabs, in the order the phone's tab bar has them.
pub const TABS: [View; 4] = [View::Home, View::Albums, View::Settings, View::Search];

/// Each tab's place on the tab line, from one column in, two columns apart; what does not fit is left off.
pub fn tabs(head: Rect) -> Vec<(View, Rect)> {
    let mut x = head.x + 1;
    let end = head.x + head.width;
    let mut out = Vec::with_capacity(TABS.len());
    for v in TABS {
        let w = tab_name(v).width() as u16;
        if x + w > end {
            break;
        }
        out.push((v, Rect { x, y: head.y, width: w, height: 1 }));
        x += w + 2;
    }
    out
}

/// A tab's name: the library's sections share one.
pub fn tab_name(v: View) -> &'static str {
    match v {
        View::Home | View::Login => "Home",
        View::Settings | View::Equalizer => "Settings",
        View::Search => "Search",
        _ => "Library",
    }
}

/// The two lines of the song playing, their pieces by how wide they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Now {
    /// Play or pause.
    pub toggle: Rect,
    /// The title, then the artist and album.
    pub song: Rect,
    /// Shuffle and repeat while on, and the seal, right-aligned on the song's line.
    pub marks: Rect,
    /// The elapsed time, the seek bar, the length.
    pub clock: Option<[Rect; 3]>,
    /// Where the music plays.
    pub output: Option<Rect>,
    /// The volume's bar.
    pub volume: Option<Rect>,
}

const MARKS_W: u16 = 44;
const OUTPUT_W: u16 = 22;
const VOLUME_W: u16 = 17;

/// Lays the song playing out in `area` (one or two rows): the song's line, then its clock with where it
/// plays and the volume. Narrower, the volume goes, then where it plays.
pub fn now(area: Rect) -> Now {
    let line = |r: u16, x: u16, width: u16| Rect { x, y: area.y + r.min(area.height.saturating_sub(1)), width, height: 1 };
    let w = area.width;
    let x0 = area.x + 1;
    let marks_w = MARKS_W.min(w / 2);
    let toggle = line(0, x0, 1);
    let song = line(0, x0 + 2, w.saturating_sub(marks_w + 4));
    let marks = line(0, area.x + w.saturating_sub(marks_w + 1), marks_w);
    let mut out = Now { toggle, song, marks, clock: None, output: None, volume: None };
    if area.height < 2 || w < 24 {
        return out;
    }
    let volume_on = w >= 56;
    let output_on = w >= 96;
    let right = if volume_on { VOLUME_W + 5 } else { 0 } + if output_on { OUTPUT_W + 2 } else { 0 };
    let time = 5;
    let bar_w = w.saturating_sub(2 + right + 2 * (time + 1) + 1).min(80);
    let x = x0 + 2;
    out.clock = Some([line(1, x, time), line(1, x + time + 1, bar_w), line(1, x + time + 2 + bar_w, time)]);
    let mut rx = area.x + w - 1;
    if volume_on {
        rx -= VOLUME_W;
        out.volume = Some(line(1, rx, VOLUME_W));
        rx -= 5;
    }
    if output_on {
        rx -= OUTPUT_W;
        out.output = Some(line(1, rx, OUTPUT_W));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: u16, h: u16) -> Rect {
        Rect { x: 0, y: 0, width: w, height: h }
    }

    #[test]
    fn rows_fill_the_window_at_any_height() {
        for h in 1..60 {
            let r = rows(area(120, h));
            assert_eq!(r.head.height + r.page.height + r.now.height + r.foot.height, h, "h {h}");
            assert_eq!((r.head.y + r.head.height, r.page.y + r.page.height, r.now.y + r.now.height), (r.page.y, r.now.y, r.foot.y), "h {h}");
        }
        let r = rows(area(80, 24));
        assert_eq!((r.head.height, r.page.height, r.now.height, r.foot.height), (1, 20, 2, 1), "four rows of chrome on an 80 by 24");
        assert_eq!(rows(area(80, 10)).now.height, 1);
    }

    #[test]
    fn a_tab_is_found_by_the_cell_under_the_pointer() {
        let t = tabs(rows(area(80, 24)).head);
        assert_eq!(t.iter().map(|(v, _)| tab_name(*v)).collect::<Vec<_>>(), ["Home", "Library", "Settings", "Search"]);
        for pair in t.windows(2) {
            assert_eq!(pair[0].1.x + pair[0].1.width + 2, pair[1].1.x, "two columns between tabs");
        }
        assert_eq!(tabs(rows(area(20, 24)).head).len(), 2, "what does not fit is left off");
    }

    #[test]
    fn the_song_playing_drops_pieces_as_it_narrows() {
        let at = |w| now(Rect { x: 0, y: 30, width: w, height: 2 });
        let wide = at(120);
        assert!(wide.clock.is_some() && wide.output.is_some() && wide.volume.is_some());
        let mid = at(80);
        assert!(mid.output.is_none() && mid.volume.is_some());
        let narrow = at(40);
        assert!(narrow.output.is_none() && narrow.volume.is_none() && narrow.clock.is_some());
        assert!(now(Rect { x: 0, y: 0, width: 120, height: 1 }).clock.is_none(), "one row: the song alone");
        for w in 10..200 {
            let n = at(w);
            let mut all: Vec<Rect> = [n.toggle, n.song, n.marks].into_iter().chain(n.clock.into_iter().flatten()).chain(n.output).chain(n.volume).filter(|r| r.width > 0).collect();
            for r in &all {
                assert!(r.x + r.width <= w, "w {w}: {r:?}");
            }
            all.sort_by_key(|r| (r.y, r.x));
            for pair in all.windows(2) {
                if pair[0].y == pair[1].y {
                    assert!(pair[0].x + pair[0].width <= pair[1].x, "w {w}: {pair:?}");
                }
            }
        }
    }
}
