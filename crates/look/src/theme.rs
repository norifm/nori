//! The app's four themes, one per season, each a small palette of traditional Japanese colors in light
//! and dark, and the rule that follows the calendar from one to the next.

use crate::color::{luminance, BLACK, WHITE};
use crate::compose::{blend, veil};
use crate::dress::ink_on;

/// A theme, named for the season it is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

/// The seasons in the calendar's order.
const SEASONS: [Season; 4] = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];

/// What a theme is made of: the page, a raised surface (sheets, menus), the ink, the accent for the
/// one thing that is "you, now" (the current tab, a favorite, what is playing) and the color of a
/// switch that is on. ARGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Palette {
    pub page: u32,
    pub surface: u32,
    pub ink: u32,
    pub accent: u32,
    pub on: u32,
}

const fn p(page: u32, surface: u32, ink: u32, accent: u32, on: u32) -> Palette {
    Palette { page, surface, ink, accent, on }
}

impl Season {
    /// Light and dark. Spring (春霞, haze): usu-ai paper, imayō cherry red, asagi water. Summer (紫陽花): shiro-fuji
    /// rain grey, ajisai hydrangea, koke moss. Autumn (柿): kinari, kaki persimmon, chitose-midori pine. Winter
    /// (雪): yuki-shiro snow, ai indigo, ginnezumi grey for what is on.
    pub fn palette(self, dark: bool) -> Palette {
        match (self, dark) {
            (Season::Spring, false) => p(0xFFF3_F2F6, 0xFFFD_FDFE, 0xFF26_262A, 0xFFB1_4C66, 0xFF2C_7F8F),
            (Season::Spring, true) => p(0xFF10_1015, 0xFF1B_1B20, 0xFFEC_EBEF, 0xFFE9_A0B1, 0xFF6F_C3D1),
            (Season::Summer, false) => p(0xFFEF_F1F7, 0xFFFD_FDFE, 0xFF20_2229, 0xFF4B_5BB0, 0xFF4F_7F3A),
            (Season::Summer, true) => p(0xFF0A_0C14, 0xFF15_171F, 0xFFE8_EAF0, 0xFF9A_A6EE, 0xFF8F_BF74),
            (Season::Autumn, false) => p(0xFFF6_F0E6, 0xFFFF_FDF8, 0xFF24_1D16, 0xFFBE_5526, 0xFF36_563C),
            (Season::Autumn, true) => p(0xFF11_0E0A, 0xFF1C_1813, 0xFFF3_EDE3, 0xFFE8_743B, 0xFF7F_A27F),
            (Season::Winter, false) => p(0xFFF2_F4F6, 0xFFFD_FDFE, 0xFF21_2326, 0xFF2E_5C8A, 0xFF6B_7077),
            (Season::Winter, true) => p(0xFF0B_0D11, 0xFF16_181C, 0xFFEB_EDEF, 0xFF7F_B0E0, 0xFF9A_9FA6),
        }
    }

    /// The day of a 365-day year (0 for 1 January) it begins on: the old calendar's risshun, rikka,
    /// risshū and rittō.
    fn starts(self) -> u32 {
        match self {
            Season::Spring => 34,
            Season::Summer => 124,
            Season::Autumn => 218,
            Season::Winter => 310,
        }
    }

    fn before(self) -> Season {
        SEASONS[(self as usize + 3) % 4]
    }

    fn after(self) -> Season {
        SEASONS[(self as usize + 1) % 4]
    }
}

/// How many days a season takes to come in, blending from the one before: the week before it starts and
/// the week after.
const BLEND_DAYS: u32 = 14;

/// AMOLED: a dark palette on black, its surface brought a third of the way down with it.
fn on_black(p: Palette) -> Palette {
    Palette { page: BLACK, surface: blend(p.surface, BLACK, 0.35), ..p }
}

fn mixed(a: Palette, b: Palette, t: f32) -> Palette {
    Palette {
        page: blend(a.page, b.page, t),
        surface: blend(a.surface, b.surface, t),
        ink: blend(a.ink, b.ink, t),
        accent: blend(a.accent, b.accent, t),
        on: blend(a.on, b.on, t),
    }
}

const MONTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `month` (1-12) and `day` as the day of a 365-day year, so a season starts on the same date every
/// year (29 February counts as the 28th).
fn day_of_year(month: u32, day: u32) -> u32 {
    let m = month.clamp(1, 12) as usize - 1;
    MONTHS[..m].iter().sum::<u32>() + day.clamp(1, MONTHS[m]) - 1
}

fn date_of(day: u32) -> (u32, u32) {
    let mut left = day % 365;
    for (i, n) in MONTHS.iter().enumerate() {
        if left < *n {
            return (i as u32 + 1, left + 1);
        }
        left -= n;
    }
    unreachable!("the months make 365 days")
}

/// The UTC month and day of a time in seconds since 1970, for a client with no time zone to hand.
pub fn utc_date(unix_s: i64) -> (u32, u32) {
    // Howard Hinnant's days-to-civil.
    let z = unix_s.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (month as u32, day as u32)
}

/// Where the calendar is on a date: the season, and the next one with the date it starts on.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SeasonNow {
    pub season: Season,
    pub next: Season,
    pub next_month: u32,
    pub next_day: u32,
}

/// The day of the 365-day year the calendar is on. South of the equator it is half a year round.
fn calendar_day(month: u32, day: u32, southern: bool) -> u32 {
    (day_of_year(month, day) + if southern { 182 } else { 0 }) % 365
}

/// The season on a date.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn season_now(month: u32, day: u32, southern: bool) -> SeasonNow {
    let shift = if southern { 182 } else { 0 };
    let today = calendar_day(month, day, southern);
    let season = SEASONS.iter().rev().copied().find(|s| today >= s.starts()).unwrap_or(Season::Winter);
    let next = season.after();
    let (next_month, next_day) = date_of(next.starts() + 365 - shift);
    SeasonNow { season, next, next_month, next_day }
}

/// The season blending in on `today` and how far it has come, from a week before it starts (1/14) to a
/// week after (all of it); none between blends.
fn coming_in(today: u32) -> Option<(Season, f32)> {
    SEASONS.iter().find_map(|s| {
        let since = (today + 365 + BLEND_DAYS / 2 - s.starts()) % 365;
        (since < BLEND_DAYS).then(|| (*s, (since + 1) as f32 / BLEND_DAYS as f32))
    })
}

/// The palette the app wears: `season`'s, or with none the calendar's on that date, each season blended
/// in around its start.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn palette(season: Option<Season>, month: u32, day: u32, southern: bool, dark: bool, amoled: bool) -> Palette {
    let of = |s: Season| {
        let p = s.palette(dark);
        if dark && amoled { on_black(p) } else { p }
    };
    match season {
        Some(s) => of(s),
        None => match coming_in(calendar_day(month, day, southern)) {
            Some((s, t)) if t < 1.0 => mixed(of(s.before()), of(s), t),
            _ => of(season_now(month, day, southern).season),
        },
    }
}

/// Whether a time zone (an IANA id, "Australia/Sydney") lies south of the equator.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn southern(zone: &str) -> bool {
    const SOUTH: &[&str] = &[
        "Australia/", "Antarctica/", "Pacific/Auckland", "Pacific/Chatham", "Pacific/Fiji", "Pacific/Tongatapu", "Pacific/Apia",
        "Pacific/Noumea", "Pacific/Efate", "Pacific/Port_Moresby", "Pacific/Tahiti", "Pacific/Rarotonga", "Pacific/Easter",
        "America/Argentina/", "America/Buenos_Aires", "America/Santiago", "America/Punta_Arenas", "America/Montevideo",
        "America/Asuncion", "America/Sao_Paulo", "America/Rio_Branco", "America/Porto_Velho", "America/Cuiaba",
        "America/Campo_Grande", "America/Maceio", "America/Recife", "America/Bahia", "America/Araguaina", "America/Lima",
        "America/La_Paz", "Africa/Johannesburg", "Africa/Maputo", "Africa/Harare", "Africa/Lusaka", "Africa/Windhoek",
        "Africa/Gaborone", "Africa/Maseru", "Africa/Mbabane", "Africa/Blantyre", "Africa/Lubumbashi", "Africa/Luanda",
        "Africa/Dar_es_Salaam", "Indian/Antananarivo", "Indian/Mauritius", "Indian/Reunion", "Asia/Jakarta", "Asia/Makassar",
        "Asia/Jayapura", "Asia/Dili",
    ];
    SOUTH.iter().any(|s| zone.starts_with(s))
}

/// The accents of `season`'s palette (the calendar's on a date with none) for a pale page and a dark one,
/// which [`accent_on`] picks from.
pub fn accents(season: Option<Season>, month: u32, day: u32, southern: bool) -> (u32, u32) {
    let accent = |dark| palette(season, month, day, southern, dark, false).accent;
    (accent(false), accent(true))
}

/// The theme's accent on a page of any color (a cover's): `light` on a pale page, `dark` on a dark one,
/// moved in its own hue until it reads there as text, else the page's `ink`.
pub fn accent_on(page: u32, ink: u32, light: u32, dark: u32) -> u32 {
    let accent = if luminance(page) < crate::cover::DARK_PAGE { dark } else { light };
    crate::cover::text_either_way(accent, page, ink)
}

/// A palette as a Material scheme's roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Roles {
    pub background: u32,
    pub on_background: u32,
    pub on_surface_variant: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub primary_container: u32,
    pub on_primary_container: u32,
    pub secondary: u32,
    pub on_secondary: u32,
    pub secondary_container: u32,
    pub surface_variant: u32,
    pub outline: u32,
    pub outline_variant: u32,
    pub container_lowest: u32,
    pub container_low: u32,
    pub container: u32,
    pub container_high: u32,
    pub container_highest: u32,
}

/// The palette as a Material scheme's roles: the page under everything, sheets and menus on the surface
/// and lifted from it by the ink, the accent as the primary and the switch color as the secondary.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn roles(p: Palette) -> Roles {
    Roles {
        background: p.page,
        on_background: p.ink,
        on_surface_variant: veil(p.ink, 0.62, p.page),
        primary: p.accent,
        on_primary: ink_on(p.accent),
        primary_container: veil(p.accent, 0.18, p.page),
        on_primary_container: p.ink,
        secondary: p.on,
        on_secondary: ink_on(p.on),
        secondary_container: veil(p.ink, 0.12, p.surface),
        surface_variant: veil(p.ink, 0.08, p.page),
        outline: veil(p.ink, 0.36, p.page),
        outline_variant: veil(p.ink, 0.14, p.page),
        container_lowest: p.page,
        container_low: p.surface,
        container: p.surface,
        container_high: veil(p.ink, 0.05, p.surface),
        container_highest: veil(p.ink, 0.09, p.surface),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::calculate_contrast;

    #[test]
    fn every_palette_reads() {
        for s in SEASONS {
            for (dark, amoled) in [(false, false), (true, false), (true, true)] {
                let p = palette(Some(s), 1, 1, false, dark, amoled);
                let accent = calculate_contrast(p.accent, p.page);
                assert!(accent >= 4.0, "{s:?} dark {dark} amoled {amoled}: accent reads at {accent:.2}");
                assert!(calculate_contrast(p.ink, p.page) >= 12.0, "{s:?} dark {dark}: ink");
                assert!(calculate_contrast(p.on, p.page) >= 3.0, "{s:?} dark {dark}: on");
                let r = roles(p);
                assert!(calculate_contrast(r.on_primary, r.primary) >= 3.0, "{s:?} dark {dark}: on primary");
                assert!(calculate_contrast(r.on_secondary, r.secondary) >= 3.0, "{s:?} dark {dark}: on secondary");
                assert_eq!(luminance(p.page) < 0.5, dark);
            }
        }
    }

    #[test]
    fn amoled_is_black_with_the_dark_colours() {
        let (dark, black) = (Season::Autumn.palette(true), palette(Some(Season::Autumn), 1, 1, false, true, true));
        assert_eq!((black.page, black.accent, black.ink), (BLACK, dark.accent, dark.ink));
        assert!(luminance(black.surface) < luminance(dark.surface) && black.surface != BLACK);
        assert_eq!(palette(Some(Season::Autumn), 1, 1, false, false, true), Season::Autumn.palette(false), "AMOLED is a dark theme's");
    }

    #[test]
    fn the_calendar_turns_on_the_first_days_of_the_seasons() {
        for (m, d, season) in [
            (2, 3, Season::Winter), (2, 4, Season::Spring), (5, 4, Season::Spring), (5, 5, Season::Summer), (8, 6, Season::Summer),
            (8, 7, Season::Autumn), (11, 6, Season::Autumn), (11, 7, Season::Winter), (1, 1, Season::Winter), (12, 31, Season::Winter),
        ] {
            assert_eq!(season_now(m, d, false).season, season, "{d}/{m}");
        }
        let autumn = season_now(10, 18, false);
        assert_eq!((autumn.next, autumn.next_month, autumn.next_day), (Season::Winter, 11, 7));
        let winter = season_now(12, 1, false);
        assert_eq!((winter.next, winter.next_month, winter.next_day), (Season::Spring, 2, 4));
        // South of the equator October is spring, and summer comes in at the start of November.
        let south = season_now(10, 18, true);
        assert_eq!((south.season, south.next, south.next_month, south.next_day), (Season::Spring, Season::Summer, 11, 4));
    }

    #[test]
    fn a_season_blends_from_a_week_before_it_starts_to_a_week_after() {
        let follow = |m, d| palette(None, m, d, false, true, false);
        let (autumn, winter) = (Season::Autumn.palette(true), Season::Winter.palette(true));
        assert_eq!(follow(10, 30), autumn);
        assert_eq!(follow(10, 31), mixed(autumn, winter, 1.0 / 14.0));
        assert_eq!(follow(11, 7), mixed(autumn, winter, 8.0 / 14.0));
        assert_eq!(follow(11, 13), winter);
        assert_eq!(follow(11, 20), winter);
        // The blend is the same across New Year (winter to spring is early February) and south of the equator.
        let spring = Season::Spring.palette(true);
        assert_eq!(follow(1, 28), mixed(winter, spring, 1.0 / 14.0));
        assert_eq!(palette(None, 5, 9, true, true, false), mixed(autumn, winter, 8.0 / 14.0));
    }

    #[test]
    fn the_accent_reads_as_text_on_any_page() {
        let (light, dark) = (0xFFBE_5526, 0xFFE8_743B);
        for page in [0xFF10_1010u32, 0xFF62_8040, 0xFF20_3A60, 0xFF80_8080, 0xFFE8_7040, WHITE] {
            let ink = if luminance(page) < 0.5 { WHITE } else { 0xFF11_1111 };
            let a = accent_on(page, ink, light, dark);
            // Where no shade of it reads, the page's own ink, which reads as well as anything can.
            assert!(calculate_contrast(a, page) >= 4.5f64.min(calculate_contrast(ink, page)), "{a:08x} on {page:08x}");
        }
        assert_eq!(accent_on(0xFF10_1010, WHITE, light, dark), dark);
        assert_eq!(accent_on(WHITE, 0xFF11_1111, light, dark), light);
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc_date(0), (1, 1));
        assert_eq!(utc_date(1_792_281_600), (10, 18)); // 2026-10-18
        assert_eq!(utc_date(1_709_164_800), (2, 29)); // 2024-02-29
        assert_eq!(utc_date(-1), (12, 31));
    }

    #[test]
    fn southern_zones() {
        assert!(southern("Australia/Sydney") && southern("America/Argentina/Cordoba") && southern("Pacific/Auckland"));
        assert!(!southern("Europe/Warsaw") && !southern("Asia/Tokyo") && !southern("America/New_York") && !southern(""));
    }
}
