//! The app's colors: the season's palette from nori_look, dressed into the roles the `Theme` global holds,
//! for a plain page and for a page that wears a cover. No color is chosen here.

use nori_core::settings::{season_of, StoredPrefs, ThemeMode};
use nori_look::compose::with_alpha;
use nori_look::cover::CoverColours;
use nori_look::dress::{self, Scheme, LEN};
use nori_look::theme::{self, Palette, Season};

use crate::Theme;

/// A calendar date, and whether the zone it is in lies south of the equator (the seasons turn half a year
/// later there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Today {
    pub month: u32,
    pub day: u32,
    pub southern: bool,
}

impl Today {
    /// The local date now.
    pub fn now() -> Today {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (month, day) = theme::utc_date(now + nori_core::library::local_offset_s(now));
        Today { month, day, southern: zone().is_some_and(|z| theme::southern(&z)) }
    }
}

/// The IANA id of the local time zone, from the link `/etc/localtime` is.
fn zone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    zone_of(&target.to_string_lossy()).map(str::to_string)
}

fn zone_of(link: &str) -> Option<&str> {
    link.split_once("zoneinfo/").map(|(_, zone)| zone)
}

/// What the app wears: the colors of a page, plus the few the theme keeps whatever the page.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    table: [u32; LEN],
    on: u32,
    on_on: u32,
    danger: u32,
    /// Each season's page and accent, for the season tiles.
    seasons: [(u32, u32); 4],
}

/// Material's baseline error, light and dark: the palette has none of its own.
const DANGER_LIGHT: u32 = 0xFFB3_261E;
const DANGER_DARK: u32 = 0xFFF2_B8B5;

/// Whether the app is dark: its setting, or the system's.
pub fn is_dark(prefs: &StoredPrefs, system_dark: bool) -> bool {
    match prefs.theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => system_dark,
    }
}

/// The palette the settings and the calendar ask for.
pub fn palette(prefs: &StoredPrefs, system_dark: bool, today: Today) -> Palette {
    theme::palette(season_of(prefs.season), today.month, today.day, today.southern, is_dark(prefs, system_dark), prefs.amoled)
}

/// A page with no cover: the palette's own.
pub fn plain(prefs: &StoredPrefs, system_dark: bool, today: Today) -> Look {
    let p = palette(prefs, system_dark, today);
    let r = theme::roles(p);
    let scheme = Scheme {
        background: r.background,
        on: r.on_background,
        on_variant: r.on_surface_variant,
        primary: r.primary,
        on_primary: r.on_primary,
        surface_variant: r.surface_variant,
        surface_container: r.container,
        surface_container_high: r.container_high,
        secondary_container: r.secondary_container,
        outline_variant: r.outline_variant,
    };
    let dark = is_dark(prefs, system_dark);
    // A tile wears its season's dark palette rather than black: on black every tile would be the same.
    let seasons = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter].map(|s| {
        let p = theme::palette(Some(s), 1, 1, false, dark, false);
        (p.page, p.accent)
    });
    Look { table: dress::plain(&scheme), on: p.on, on_on: r.on_secondary, danger: if dark { DANGER_DARK } else { DANGER_LIGHT }, seasons }
}

/// A page about a cover: the cover's page and ink, the theme's accent moved until it reads there.
pub fn cover(prefs: &StoredPrefs, system_dark: bool, today: Today, c: &CoverColours) -> Look {
    let base = plain(prefs, system_dark, today);
    let (light, dark) = theme::accents(season_of(prefs.season), today.month, today.day, today.southern);
    let page = dress::page(c.edge, c.background, c.on, c.accent, c.wash_edge);
    Look { table: dress::accented(&page, light, dark), ..base }
}

impl Look {
    /// `self` a fraction `t` of the way to `other`: the page's colors cross-fade as one.
    pub fn towards(&self, other: &Look, t: f32) -> Look {
        let mut table = [0; LEN];
        dress::mix(&self.table, &other.table, t, &mut table);
        Look { table, ..other.clone() }
    }

    pub fn page(&self) -> u32 {
        self.table[dress::BACKGROUND]
    }

    #[cfg(test)]
    pub fn ink(&self) -> u32 {
        self.table[dress::ON]
    }

    #[cfg(test)]
    pub fn accent(&self) -> u32 {
        self.table[dress::ACCENT]
    }

    /// Sets the roles on a window's `Theme`.
    pub fn paint(&self, theme: &Theme) {
        let t = &self.table;
        let c = |argb| slint::Color::from_argb_encoded(argb);
        theme.set_ground(c(t[dress::BACKGROUND]));
        theme.set_panel(c(t[dress::SURFACE_CONTAINER]));
        theme.set_text(c(t[dress::ON]));
        theme.set_dim(c(t[dress::ON_VARIANT]));
        theme.set_faint(c(t[dress::ON_35]));
        theme.set_hairline(c(t[dress::OUTLINE_VARIANT]));
        theme.set_hover(c(with_alpha(t[dress::ON], 0.06)));
        theme.set_chosen(c(with_alpha(t[dress::ON], 0.10)));
        theme.set_fill(c(t[dress::FIELD]));
        theme.set_raised(c(t[dress::SURFACE_CONTAINER_HIGH]));
        theme.set_accent(c(t[dress::ACCENT]));
        theme.set_on_accent(c(t[dress::ON_PRIMARY]));
        theme.set_pill(c(t[dress::PILL]));
        theme.set_pill_ink(c(t[dress::PILL_INK]));
        theme.set_plate(c(t[dress::PILL_PLATE]));
        theme.set_plate_ink(c(t[dress::TINT_INK]));
        theme.set_outline(c(t[dress::CHROME_OUTLINE]));
        theme.set_on(c(self.on));
        theme.set_on_on(c(self.on_on));
        theme.set_danger(c(self.danger));
        theme.set_edge(c(t[dress::EDGE]));
        theme.set_dark(nori_look::color::luminance(t[dress::BACKGROUND]) < 0.5);
        let colors = |pick: fn(&(u32, u32)) -> u32| slint::ModelRc::new(slint::VecModel::from(self.seasons.iter().map(|s| c(pick(s))).collect::<Vec<_>>()));
        theme.set_season_pages(colors(|s| s.0));
        theme.set_season_accents(colors(|s| s.1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::settings::ThemeSeason;
    use nori_look::theme::Season;

    const OCTOBER: Today = Today { month: 10, day: 19, southern: false };

    fn prefs(season: ThemeSeason, theme: ThemeMode, amoled: bool) -> StoredPrefs {
        StoredPrefs { season, theme, amoled, ..Default::default() }
    }

    #[test]
    fn the_setting_or_the_system_decides_dark() {
        let of = |theme, system| is_dark(&prefs(ThemeSeason::Follow, theme, false), system);
        assert!(of(ThemeMode::Dark, false) && !of(ThemeMode::Light, true));
        assert!(of(ThemeMode::System, true) && !of(ThemeMode::System, false));
    }

    #[test]
    fn following_wears_the_calendars_season_and_a_fixed_one_its_own() {
        let follow = palette(&prefs(ThemeSeason::Follow, ThemeMode::Dark, false), false, OCTOBER);
        assert_eq!(follow, Season::Autumn.palette(true));
        let winter = palette(&prefs(ThemeSeason::Winter, ThemeMode::Light, false), true, OCTOBER);
        assert_eq!(winter, Season::Winter.palette(false));
        // South of the equator October is spring.
        let south = palette(&prefs(ThemeSeason::Follow, ThemeMode::Dark, false), false, Today { southern: true, ..OCTOBER });
        assert_eq!(south, Season::Spring.palette(true));
    }

    #[test]
    fn amoled_is_a_black_page_in_the_dark_theme_only() {
        let black = plain(&prefs(ThemeSeason::Autumn, ThemeMode::Dark, true), false, OCTOBER);
        assert_eq!(black.page(), nori_look::color::BLACK);
        let light = plain(&prefs(ThemeSeason::Autumn, ThemeMode::Light, true), false, OCTOBER);
        assert_eq!(light.page(), Season::Autumn.palette(false).page);
    }

    #[test]
    fn a_plain_page_is_the_palettes() {
        let p = Season::Spring.palette(true);
        let look = plain(&prefs(ThemeSeason::Spring, ThemeMode::Dark, false), false, OCTOBER);
        assert_eq!((look.page(), look.ink(), look.accent()), (p.page, p.ink, p.accent));
        assert_eq!(look.on, p.on);
    }

    #[test]
    fn a_cover_page_keeps_the_themes_accent_readable_on_the_cover() {
        let calculate_contrast = |a: u32, b: u32| {
            let (a, b) = (nori_look::color::luminance(a) as f64 + 0.05, nori_look::color::luminance(b) as f64 + 0.05);
            a.max(b) / a.min(b)
        };
        let prefs = prefs(ThemeSeason::Autumn, ThemeMode::Dark, false);
        let cover_colours = nori_look::cover::derive(&[0xFF20_5030], 1, 1, false);
        let look = cover(&prefs, false, OCTOBER, &cover_colours);
        assert_eq!((look.page(), look.ink()), (cover_colours.background, cover_colours.on));
        assert_ne!(look.accent(), cover_colours.accent, "the cover's own accent is not worn");
        let reads = calculate_contrast(look.accent(), look.page());
        assert!(reads >= 4.5f64.min(calculate_contrast(look.ink(), look.page())), "accent reads at {reads:.2}");
    }

    #[test]
    fn a_fade_runs_from_one_page_to_the_other() {
        let light = plain(&prefs(ThemeSeason::Winter, ThemeMode::Light, false), false, OCTOBER);
        let dark = plain(&prefs(ThemeSeason::Winter, ThemeMode::Dark, false), false, OCTOBER);
        assert_eq!(light.towards(&dark, 0.0).page(), light.page());
        assert_eq!(light.towards(&dark, 1.0), dark);
        let half = light.towards(&dark, 0.5).page();
        assert!(nori_look::color::luminance(half) < nori_look::color::luminance(light.page()));
        assert!(nori_look::color::luminance(half) > nori_look::color::luminance(dark.page()));
    }

    #[test]
    fn the_zone_is_what_follows_zoneinfo_in_the_link() {
        assert_eq!(zone_of("/var/db/timezone/zoneinfo/Australia/Sydney"), Some("Australia/Sydney"));
        assert_eq!(zone_of("../usr/share/zoneinfo/Europe/Warsaw"), Some("Europe/Warsaw"));
        assert_eq!(zone_of("/etc/localtime"), None);
    }
}
