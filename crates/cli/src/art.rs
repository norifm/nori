//! Covers drawn through ratatui-image (kitty, sixel, iTerm2 or half blocks) from images decoded by
//! nori-covers, and the interface colours derived from them by nori-look.

use std::sync::Arc;

use image::{DynamicImage, RgbaImage};
use nori_covers::memory::Image;
use ratatui::style::{Color, Modifier, Style};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;

/// Display name of a graphics protocol.
pub fn protocol_name(p: ProtocolType) -> &'static str {
    match p {
        ProtocolType::Kitty => "kitty graphics",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Iterm2 => "iTerm2 inline images",
        ProtocolType::Halfblocks => "half blocks",
    }
}

/// Decode size of large covers: sharp at a quarter of a big terminal, cheap to derive colours from.
pub const COVER_PX: u32 = 600;

/// Covers kept: the playing one and the last few, for a skip back.
const COVERS: usize = 3;

/// Encoded covers by key, each with its decoded image for [`Art::resend`]. Oldest first.
type Kept = Vec<(String, StatefulProtocol, Arc<Image>)>;

/// The covers on screen, each encoded once for the area it is drawn in.
pub struct Art {
    picker: Picker,
    covers: Kept,
}

impl Art {
    pub fn new(picker: Picker) -> Art {
        Art { picker, covers: Vec::new() }
    }

    /// Adds a decoded cover under `key`, evicting the oldest past the cap.
    pub fn put(&mut self, key: String, image: &Arc<Image>) {
        let Some(protocol) = encode(&self.picker, image) else { return };
        self.covers.retain(|(k, _, _)| *k != key);
        if self.covers.len() >= COVERS {
            self.covers.remove(0);
        }
        self.covers.push((key, protocol, image.clone()));
    }

    /// Re-encodes every cover so the next draw transmits it in full. A protocol state sends its picture
    /// once, so a terminal that dropped it (tmux, while the pane was hidden) would never get it back.
    pub fn resend(&mut self) {
        if self.picker.protocol_type() == ProtocolType::Halfblocks {
            return;
        }
        for (_, protocol, image) in &mut self.covers {
            if let Some(p) = encode(&self.picker, image) {
                *protocol = p;
            }
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&mut StatefulProtocol> {
        self.covers.iter_mut().find(|(k, _, _)| k == key).map(|(_, p, _)| p)
    }
}

fn encode(picker: &Picker, image: &Image) -> Option<StatefulProtocol> {
    let rgba = RgbaImage::from_raw(image.width, image.height, image.pixels.to_vec())?;
    Some(picker.new_resize_protocol(DynamicImage::ImageRgba8(rgba)))
}

/// How many colors the terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colors {
    #[default]
    True,
    /// The xterm 256-color palette.
    Indexed,
    /// `NO_COLOR`: no color at all, weights and underlines only.
    None,
}

impl Colors {
    /// From the environment: `NO_COLOR` (set and not empty) wins, then `COLORTERM`.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Colors {
        if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            Colors::None
        } else if env("COLORTERM").is_some_and(|v| matches!(v.as_str(), "truecolor" | "24bit")) {
            Colors::True
        } else {
            Colors::Indexed
        }
    }
}

/// Interface colors: the roles of a palette (page, surface, ink, accent, on) and the steps of ink over the
/// page that the contract fixes (quiet 65 %, chrome outline 22 %, hairline 14 %).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub page: Color,
    /// Raised: popups, cards, the selected row.
    pub surface: Color,
    pub ink: Color,
    /// Secondary text.
    pub quiet: Color,
    /// The chrome's outline.
    pub line: Color,
    pub hair: Color,
    pub accent: Color,
    /// Only for a switch that is on.
    pub on: Color,
    /// The focused selection's background: the ink over the surface at 12 %.
    pub lift: Color,
    /// The page is painted; else the terminal's own background shows and the roles are its own.
    pub painted: bool,
    /// No colors at all.
    pub mono: bool,
}

pub fn argb(c: u32) -> Color {
    Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Linear blend from `a` (t = 0) to `b` (t = 1); non-RGB colors snap at 0.5.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
            Color::Rgb(m(ar, br), m(ag, bg), m(ab, bb))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// A date and hemisphere, which the followed season depends on.
pub use nori_host::desktop::Today as Day;

/// Whether the terminal's own background is dark, as `COLORFGBG` ("fg;bg") says; dark when unknown.
pub fn terminal_dark(colorfgbg: Option<&str>) -> bool {
    let bg = colorfgbg.and_then(|v| v.rsplit(';').next()).and_then(|b| b.parse::<u8>().ok());
    !matches!(bg, Some(7 | 15))
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::of(nori_look::theme::Season::Autumn.palette(true), true, Colors::True)
    }
}

impl Theme {
    /// The season's palette for the appearance settings on `day`.
    pub fn season(p: &nori_core::settings::StoredPrefs, day: Day, terminal_dark: bool, painted: bool, colors: Colors) -> Theme {
        use nori_core::settings::ThemeMode;
        let pal = nori_look::theme::palette(nori_core::settings::season_of(p.season), day.month, day.day, day.southern, p.theme.is_dark(terminal_dark), p.theme == ThemeMode::Midnight);
        Theme::of(pal, painted, colors)
    }

    /// The roles of `pal`; with an unpainted page the terminal's own ink and background, with no colors
    /// nothing but weights.
    pub fn of(pal: nori_look::theme::Palette, painted: bool, colors: Colors) -> Theme {
        let r = nori_look::theme::roles(pal);
        // The ink over the page at an alpha, as every client veils it.
        let at = |a: f32| argb(nori_look::compose::over(nori_look::compose::with_alpha(pal.ink, a), pal.page));
        let rgb = Theme {
            page: argb(pal.page),
            surface: argb(pal.surface),
            ink: argb(pal.ink),
            quiet: argb(r.on_surface_variant),
            line: at(0.22),
            hair: argb(r.outline_variant),
            accent: argb(pal.accent),
            on: argb(pal.on),
            lift: argb(r.secondary_container),
            painted: true,
            mono: false,
        };
        match (colors, painted) {
            (Colors::None, _) => Theme::terminal(Color::Reset, true),
            (_, true) => rgb,
            (_, false) => Theme { accent: rgb.accent, on: rgb.on, ..Theme::terminal(Color::DarkGray, false) },
        }
    }

    /// The terminal's own foreground and background.
    fn terminal(grey: Color, mono: bool) -> Theme {
        Theme { page: Color::Reset, surface: Color::Reset, ink: Color::Reset, quiet: grey, line: grey, hair: grey, accent: Color::Reset, on: Color::Reset, lift: Color::Reset, painted: false, mono }
    }

    /// The season's paper dyed with a cover whose sleeve ends in `edge` (`theme::Dye`), as the phone's
    /// player and pages wear it: between the season's light and dark palettes for these settings on `day`,
    /// the dark one for a dark cover or a `dark` interface.
    pub fn dyed(edge: u32, p: &nori_core::settings::StoredPrefs, day: Day, dark: bool, painted: bool, colors: Colors) -> Theme {
        let paper = |dark| nori_core::settings::season_palette(p.season, p.theme, dark, day.month, day.day, day.southern);
        Theme::of(nori_look::theme::Dye::new(edge, paper(false), paper(true), dark).palette(), painted, colors)
    }

    /// Text color at `strength` (1 = full) between the page and the ink; unpainted, only full or quiet.
    pub fn lit(&self, strength: f32) -> Color {
        if self.painted {
            blend(self.page, self.ink, strength)
        } else if strength >= 0.9 {
            self.ink
        } else {
            self.quiet
        }
    }

    /// A liked seal: the page's color on the accent (reversed where there are no colors).
    pub fn liked(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if self.painted {
            Style::default().fg(self.page).bg(self.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Black).bg(self.accent).add_modifier(Modifier::BOLD)
        }
    }

    /// The page background, nothing when unpainted.
    pub fn ground(&self) -> Style {
        if self.painted { Style::default().bg(self.page).fg(self.ink) } else { Style::default() }
    }

    /// A raised background (popups, cards), nothing when unpainted.
    pub fn raised(&self) -> Style {
        if self.painted { Style::default().bg(self.surface).fg(self.ink) } else { Style::default() }
    }

    /// Secondary text.
    pub fn dim(&self) -> Style {
        let s = Style::default().fg(self.quiet);
        if self.mono { s.add_modifier(Modifier::DIM) } else { s }
    }

    /// A current thing: the open tab, the song playing, a lit toggle, a focused field. Accent ink, never a
    /// background.
    pub fn current(&self) -> Style {
        let s = Style::default().fg(self.accent).add_modifier(Modifier::BOLD);
        if self.mono { s.add_modifier(Modifier::UNDERLINED) } else { s }
    }

    /// The selected row: the raised surface (lifted when the list has the keys), bold; reversed where the
    /// page is not painted.
    pub fn select(&self, focused: bool) -> Style {
        match (self.painted, focused) {
            (true, true) => Style::default().bg(self.lift).add_modifier(Modifier::BOLD),
            (true, false) => Style::default().bg(self.surface).add_modifier(Modifier::BOLD),
            (false, true) => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            (false, false) => Style::default().add_modifier(Modifier::BOLD),
        }
    }
}

/// The nearest xterm-256 color of an RGB one: the 6x6x6 cube or the 24 greys, whichever is closer.
pub fn indexed(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| (0..6).min_by_key(|&i| (LEVELS[i] - v as i32).abs()).unwrap_or(0);
    let (ci, cj, ck) = (near(r), near(g), near(b));
    let cube = (16 + 36 * ci + 6 * cj + ck) as u8;
    let dist = |(x, y, z): (i32, i32, i32)| (x - r as i32).pow(2) + (y - g as i32).pow(2) + (z - b as i32).pow(2);
    let cube_d = dist((LEVELS[ci], LEVELS[cj], LEVELS[ck]));
    let mean = (r as i32 + g as i32 + b as i32) / 3;
    let step = ((mean - 8 + 5) / 10).clamp(0, 23);
    let grey = 8 + 10 * step;
    if dist((grey, grey, grey)) < cube_d { 232 + step as u8 } else { cube }
}

/// Maps the drawn frame to what the terminal shows: 256 colors, or none.
pub fn finish(buf: &mut ratatui::buffer::Buffer, colors: Colors) {
    if colors == Colors::True {
        return;
    }
    let map = |c: Color| match (colors, c) {
        (Colors::Indexed, Color::Rgb(r, g, b)) => Color::Indexed(indexed(r, g, b)),
        (Colors::None, _) => Color::Reset,
        _ => c,
    };
    for cell in &mut buf.content {
        cell.fg = map(cell.fg);
        cell.bg = map(cell.bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers() {
        let mut px = vec![0u8; 32 * 32 * 4];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[180, 20, 30, 255]);
        }
        let image = Image { width: 32, height: 32, pixels: px.into_boxed_slice() };
        let c = nori_host::derive(&image);
        // The season's paper dyed the record's red, its dark one for a dark sleeve in the day.
        let prefs = nori_core::settings::StoredPrefs { season: nori_core::settings::ThemeSeason::Autumn, theme: nori_core::settings::ThemeMode::Light, ..Default::default() };
        let day = Day { month: 10, day: 19, southern: false };
        let t = Theme::dyed(c.edge, &prefs, day, false, true, Colors::True);
        let Color::Rgb(r, g, b) = t.page else { panic!("a page color") };
        assert!(r > g && r > b && r < 128, "the page is the dark paper dyed red: {r} {g} {b}");
        // The palette every other client's cover page wears.
        let paper = |dark| nori_core::settings::season_palette(prefs.season, prefs.theme, dark, 10, 19, false);
        let dyed = nori_look::theme::Dye::new(c.edge, paper(false), paper(true), false).palette();
        assert_eq!(t, Theme::of(dyed, true, Colors::True));

        // Cover renders in every protocol.
        let image = Arc::new(Image { width: 8, height: 8, pixels: vec![200u8; 8 * 8 * 4].into_boxed_slice() });
        for p in [ProtocolType::Halfblocks, ProtocolType::Sixel, ProtocolType::Kitty, ProtocolType::Iterm2] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(p);
            let mut art = Art::new(picker);
            art.put("u".into(), &image);
            assert!(art.get("u").is_some());
            art.resend();
            assert!(art.get("u").is_some());
            let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 5));
            let widget = ratatui_image::StatefulImage::<StatefulProtocol>::default();
            ratatui::widgets::StatefulWidget::render(widget, buf.area, &mut buf, art.get("u").unwrap());
            assert!(buf.content.iter().any(|c| !c.symbol().trim().is_empty() || c.diff_option != ratatui::buffer::CellDiffOption::None), "{p:?} drew nothing");
        }
    }

    #[test]
    fn xterm_256_nearest() {
        // Pure cube corners, greys on the ramp, and a mid grey that is closer to the ramp than the cube.
        for ((r, g, b), want) in [((0, 0, 0), 16), ((255, 255, 255), 231), ((255, 0, 0), 196), ((0, 255, 0), 46), ((0, 0, 255), 21), ((95, 135, 175), 67), ((128, 128, 128), 244), ((8, 8, 8), 232), ((238, 238, 238), 255)] {
            assert_eq!(indexed(r, g, b), want, "{r} {g} {b}");
        }
    }

    #[test]
    fn colors_from_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(Colors::detect(env(&[("COLORTERM", "truecolor")])), Colors::True);
        assert_eq!(Colors::detect(env(&[("COLORTERM", "24bit")])), Colors::True);
        assert_eq!(Colors::detect(env(&[("TERM", "xterm-256color")])), Colors::Indexed);
        assert_eq!(Colors::detect(env(&[("COLORTERM", "truecolor"), ("NO_COLOR", "1")])), Colors::None);
        assert_eq!(Colors::detect(env(&[("COLORTERM", "truecolor"), ("NO_COLOR", "")])), Colors::True, "an empty NO_COLOR is unset");
    }

    #[test]
    fn terminal_background_from_colorfgbg() {
        for (v, dark) in [(None, true), (Some("15;0"), true), (Some("0;15"), false), (Some("0;7"), false), (Some("default;default"), true), (Some("12;8"), true)] {
            assert_eq!(terminal_dark(v), dark, "{v:?}");
        }
    }

    #[test]
    fn roles_follow_the_palette() {
        use nori_look::theme::Season;
        let pal = Season::Autumn.palette(true);
        let t = Theme::of(pal, true, Colors::True);
        assert_eq!((t.page, t.surface, t.ink, t.accent, t.on), (argb(pal.page), argb(pal.surface), argb(pal.ink), argb(pal.accent), argb(pal.on)));
        // Quiet 65 %, the chrome outline 22 % and the hairline 14 % of the ink over the page.
        let r = nori_look::theme::roles(pal);
        assert_eq!((t.quiet, t.hair), (argb(r.on_surface_variant), argb(r.outline_variant)));
        let veil = |a: f32| argb(nori_look::compose::over(nori_look::compose::with_alpha(pal.ink, a), pal.page));
        assert_eq!((t.quiet, t.line, t.hair), (veil(nori_look::theme::QUIET), veil(0.22), veil(0.14)));
        let Color::Rgb(a, b, c) = t.line else { panic!() };
        assert!(a > 0x11 && b > 0x0E && c > 0x0A, "the outline lies above the page");
        // The selected row is raised, never the accent.
        assert_ne!(t.select(true).bg, Some(t.accent));
        assert_eq!(t.select(true).bg, Some(t.lift));
        assert_eq!(t.select(false).bg, Some(t.surface));
        assert_eq!(t.current().fg, Some(t.accent));
    }

    #[test]
    fn appearance_picks_the_palette() {
        use nori_core::settings::{StoredPrefs, ThemeMode, ThemeSeason};
        let day = Day { month: 1, day: 15, southern: false };
        let prefs = |season, theme| StoredPrefs { season, theme, ..StoredPrefs::default() };
        let pick = |p: &StoredPrefs, term_dark| Theme::season(p, day, term_dark, true, Colors::True);
        let light = pick(&prefs(ThemeSeason::Spring, ThemeMode::Light), true);
        let dark = pick(&prefs(ThemeSeason::Spring, ThemeMode::Dark), false);
        let black = pick(&prefs(ThemeSeason::Spring, ThemeMode::Midnight), false);
        assert_eq!(light.page, argb(nori_look::theme::Season::Spring.palette(false).page));
        assert_eq!(dark.page, argb(nori_look::theme::Season::Spring.palette(true).page));
        assert_eq!(black.page, Color::Rgb(0, 0, 0));
        assert_eq!(pick(&prefs(ThemeSeason::Spring, ThemeMode::System), false).page, light.page, "system follows the terminal");
        assert_eq!(pick(&prefs(ThemeSeason::Spring, ThemeMode::System), true).page, dark.page);
        // Following the calendar in mid January is winter, in the south summer.
        let follow = prefs(ThemeSeason::Follow, ThemeMode::Dark);
        assert_eq!(pick(&follow, true).page, argb(nori_look::theme::Season::Winter.palette(true).page));
        let south = Theme::season(&follow, Day { southern: true, ..day }, true, true, Colors::True);
        assert_eq!(south.page, argb(nori_look::theme::Season::Summer.palette(true).page));
    }

    #[test]
    fn unpainted_and_mono_keep_to_the_terminals_own() {
        let pal = nori_look::theme::Season::Winter.palette(false);
        let plain = Theme::of(pal, false, Colors::True);
        assert_eq!((plain.page, plain.ink, plain.painted), (Color::Reset, Color::Reset, false));
        assert_eq!(plain.accent, argb(pal.accent));
        assert!(plain.select(true).add_modifier.contains(Modifier::REVERSED));
        let mono = Theme::of(pal, true, Colors::None);
        assert_eq!((mono.accent, mono.page, mono.mono), (Color::Reset, Color::Reset, true));
        assert!(mono.current().add_modifier.contains(Modifier::UNDERLINED | Modifier::BOLD));
        assert!(mono.dim().add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn frames_are_mapped_to_the_terminal() {
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_fg(Color::Rgb(255, 0, 0)).set_bg(Color::Rgb(0, 0, 0));
        buf[(1, 0)].set_fg(Color::Blue);
        let mut kept = buf.clone();
        finish(&mut kept, Colors::True);
        assert_eq!(kept, buf);
        let mut cells = buf.clone();
        finish(&mut cells, Colors::Indexed);
        assert_eq!((cells[(0, 0)].fg, cells[(0, 0)].bg, cells[(1, 0)].fg), (Color::Indexed(196), Color::Indexed(16), Color::Blue));
        finish(&mut buf, Colors::None);
        assert_eq!((buf[(0, 0)].fg, buf[(0, 0)].bg, buf[(1, 0)].fg), (Color::Reset, Color::Reset, Color::Reset));
    }
}
