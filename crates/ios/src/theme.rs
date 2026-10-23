//! The season theme's colors for the UIKit app: the `theme` and `season` settings worn on the device's
//! date and zone (`nori_core::settings::theme_palette`), and the season's paper dyed with a mix's or a
//! cover's color (`nori_look::theme::Dye`).

use std::ffi::{c_char, CStr};

use nori_core::settings::{set_by_name, theme_palette, StoredPrefs, ThemeMode};
use nori_look::cover::derive;
use nori_look::dress;
use nori_look::sleeve::HERO_STOPS;
use nori_look::theme::{roles, season_now, southern, utc_date, Dye, Palette};
use serde_json::json;

use crate::session::c_text;

/// `nori_look::theme::Palette` (ARGB) and its secondary text color, as `NoriPalette` in the header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoriPalette {
    page: u32,
    surface: u32,
    ink: u32,
    accent: u32,
    on: u32,
    quiet: u32,
}

impl From<Palette> for NoriPalette {
    fn from(p: Palette) -> NoriPalette {
        NoriPalette { page: p.page, surface: p.surface, ink: p.ink, accent: p.accent, on: p.on, quiet: roles(p).on_surface_variant }
    }
}

impl From<&NoriPalette> for Palette {
    fn from(p: &NoriPalette) -> Palette {
        Palette { page: p.page, surface: p.surface, ink: p.ink, accent: p.accent, on: p.on }
    }
}

/// What the app wears, as `NoriLook` in the header: the theme's palette, the season's day and night
/// palettes a cover's dye takes, whether the theme is dark, and whether pages take their covers' colors
/// (not in Midnight).
#[repr(C)]
pub struct NoriLook {
    worn: NoriPalette,
    light: NoriPalette,
    dark: NoriPalette,
    dark_theme: i32,
    covers: i32,
}

/// `nori_look::theme::SeasonNow`, as `NoriSeasonNow` in the header: seasons are 0 spring to 3 winter.
#[repr(C)]
pub struct NoriSeasonNow {
    season: i32,
    next: i32,
    next_month: u32,
    next_day: u32,
}

/// The device's local month and day, and whether its zone is south of the equator.
fn local_date(unix_s: i64, offset_min: i32, zone: &str) -> (u32, u32, bool) {
    let (month, day) = utc_date(unix_s + i64::from(offset_min) * 60);
    (month, day, southern(zone))
}

/// Settings holding the `season` and `theme` values given; an unknown value keeps the default.
fn prefs(season: &str, theme: &str) -> StoredPrefs {
    let p = StoredPrefs::default();
    let p = set_by_name(&p, "season", season).map_or(p, |c| c.prefs);
    set_by_name(&p, "theme", theme).map_or(p, |c| c.prefs)
}

fn look(p: &StoredPrefs, system_dark: bool, (month, day, south): (u32, u32, bool)) -> NoriLook {
    let (worn, dark) = theme_palette(p, system_dark, month, day, south);
    let wearing = |theme| theme_palette(&StoredPrefs { theme, ..p.clone() }, system_dark, month, day, south).0;
    let night = if p.theme == ThemeMode::Midnight { ThemeMode::Midnight } else { ThemeMode::Dark };
    NoriLook {
        worn: worn.into(),
        light: wearing(ThemeMode::Light).into(),
        dark: wearing(night).into(),
        dark_theme: i32::from(dark),
        covers: i32::from(p.theme.covers_tint_pages()),
    }
}

/// What the `season` and `theme` settings' values (`FOLLOW`, `SPRING`, ...; `SYSTEM`, `LIGHT`, `DARK`,
/// `MIDNIGHT`) wear on a local date given as seconds since 1970 and the zone's offset from UTC in minutes
/// and its IANA id. `system_dark`: the system's scheme, which `SYSTEM` follows.
///
/// # Safety
/// `season`, `theme` and `zone` are NUL-terminated UTF-8; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_look(
    season: *const c_char,
    theme: *const c_char,
    system_dark: i32,
    unix_s: i64,
    offset_min: i32,
    zone: *const c_char,
    out: *mut NoriLook,
) {
    let p = prefs(&c_text(season), &c_text(theme));
    let date = local_date(unix_s, offset_min, &c_text(zone));
    // SAFETY: the caller's.
    unsafe { out.write(look(&p, system_dark != 0, date)) };
}

/// The season the calendar is in and the next one with its start date, for the same date.
///
/// # Safety
/// `zone` is NUL-terminated UTF-8; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_season_now(unix_s: i64, offset_min: i32, zone: *const c_char, out: *mut NoriSeasonNow) {
    // SAFETY: the caller's.
    let zone = unsafe { CStr::from_ptr(zone).to_string_lossy() };
    let (month, day, south) = local_date(unix_s, offset_min, &zone);
    let now = season_now(month, day, south);
    // SAFETY: the caller's.
    unsafe { out.write(NoriSeasonNow { season: now.season as i32, next: now.next as i32, next_month: now.next_month, next_day: now.next_day }) };
}

/// A mix card's colors on Home (`mixes::mix_card_colours`): the season's paper, `light` and `dark` as
/// [`nori_ios_look`] gave them, dyed with mix `id`'s color.
///
/// # Safety
/// `id` is NUL-terminated UTF-8; `light` and `dark` are readable and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_mix_card(id: *const c_char, light: *const NoriPalette, dark: *const NoriPalette, dark_theme: i32, out: *mut NoriPalette) {
    // SAFETY: the caller's.
    let (light, dark) = unsafe { (&*light, &*dark) };
    let p = nori_core::mixes::board::mix_card_colours(c_text(id), light.into(), dark.into(), dark_theme != 0);
    // SAFETY: the caller's.
    unsafe { out.write(p.into()) };
}

/// The colors of a cover's page from its RGBA pixels: the season's paper dyed with the cover (as Android's
/// `CoverLook.dyed` wears it), as a dress table.
fn cover_page(rgba: &[u8], w: usize, h: usize, light: Palette, dark: Palette, dark_theme: bool) -> [u32; dress::LEN] {
    let argb: Vec<u32> = rgba.as_chunks::<4>().0.iter().map(|p| 0xFF00_0000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2])).collect();
    let c = derive(&argb, w, h, false);
    let page = dress::page(c.edge, c.background, c.on, c.accent, c.wash_edge);
    dress::dyed(&page, &Dye::new(page[dress::EDGE], light, dark, dark_theme))
}

/// The cover is measured at this size: enough for its colors, cheap to decode.
const MEASURED_PX: u32 = 128;

/// Measures cover `id`'s page colors; the answer comes to the page callback with `token` as JSON:
/// `page`, `surface`, `ink`, `quiet`, `accent`, `hairline` (ARGB), and the dissolve from the picture
/// into the page, `edge` and `mid` at `stops`. Nothing comes for a cover that does not load.
///
/// # Safety
/// `id` is NUL-terminated UTF-8; `light` and `dark` are readable.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_cover_look(token: u64, id: *const c_char, light: *const NoriPalette, dark: *const NoriPalette, dark_theme: i32) {
    // SAFETY: the caller's.
    let (light, dark) = unsafe { (Palette::from(&*light), Palette::from(&*dark)) };
    crate::pages::request(token, c_text(id), MEASURED_PX, move |image| {
        let Some(image) = image else { return };
        let t = cover_page(&image.pixels, image.width as usize, image.height as usize, light, dark, dark_theme != 0);
        crate::pages::send(
            token,
            &json!({
                "page": t[dress::BACKGROUND], "surface": t[dress::SURFACE_CONTAINER], "ink": t[dress::ON],
                "quiet": t[dress::ON_VARIANT], "accent": t[dress::ACCENT], "hairline": t[dress::OUTLINE_VARIANT],
                "edge": t[dress::HERO_EDGE], "mid": t[dress::HERO_MID], "stops": HERO_STOPS,
            }),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_look::theme::{palette, Season};
    use std::ffi::CString;

    // 2026-10-19 12:00 UTC and 2026-02-04 12:00 UTC.
    const OCTOBER: i64 = 1_792_411_200;
    const FEBRUARY: i64 = 1_770_206_400;

    fn wear(season: &str, theme: &str, system_dark: bool, unix_s: i64, zone: &str) -> NoriLook {
        let (s, t, z) = (CString::new(season).unwrap(), CString::new(theme).unwrap(), CString::new(zone).unwrap());
        let mut out = std::mem::MaybeUninit::<NoriLook>::uninit();
        unsafe {
            nori_ios_look(s.as_ptr(), t.as_ptr(), i32::from(system_dark), unix_s, 0, z.as_ptr(), out.as_mut_ptr());
            out.assume_init()
        }
    }

    fn of(season: Season, dark: bool) -> NoriPalette {
        season.palette(dark).into()
    }

    #[test]
    fn the_modes_wear_day_night_and_midnight() {
        // (theme, system's scheme) -> worn, dark, covers' colors.
        let cases = [
            ("LIGHT", true, of(Season::Winter, false), 0, 1),
            ("DARK", false, of(Season::Winter, true), 1, 1),
            ("SYSTEM", true, of(Season::Winter, true), 1, 1),
            ("SYSTEM", false, of(Season::Winter, false), 0, 1),
            ("MIDNIGHT", false, palette(Some(Season::Winter), 10, 19, false, true, true).into(), 1, 0),
        ];
        for (theme, system_dark, worn, dark, covers) in cases {
            let l = wear("WINTER", theme, system_dark, OCTOBER, "Europe/Warsaw");
            assert_eq!((l.worn, l.dark_theme, l.covers), (worn, dark, covers), "{theme}, system dark {system_dark}");
            assert_eq!(l.light, of(Season::Winter, false), "{theme}: the day paper a cover dyes");
        }
        assert_eq!(wear("WINTER", "MIDNIGHT", false, OCTOBER, "Europe/Warsaw").dark, wear("WINTER", "MIDNIGHT", false, OCTOBER, "Europe/Warsaw").worn);
        assert_eq!(wear("WINTER", "LIGHT", false, OCTOBER, "Europe/Warsaw").dark, of(Season::Winter, true));
    }

    #[test]
    fn following_wears_the_calendars_season_and_flips_in_the_south() {
        assert_eq!(wear("FOLLOW", "DARK", false, OCTOBER, "Europe/Warsaw").worn, of(Season::Autumn, true));
        assert_eq!(wear("FOLLOW", "DARK", false, OCTOBER, "Australia/Sydney").worn, of(Season::Spring, true));
        let blended = wear("FOLLOW", "LIGHT", false, FEBRUARY, "Europe/Warsaw").worn;
        assert!(blended != of(Season::Winter, false) && blended != of(Season::Spring, false));
    }

    #[test]
    fn the_local_offset_moves_the_date() {
        // 22:00 UTC on 3 February is already the 4th, spring's first day, in Auckland (+13).
        let evening = FEBRUARY - 14 * 3600;
        assert_eq!(local_date(evening, 0, "Pacific/Auckland").1, 3);
        assert_eq!(local_date(evening, 13 * 60, "Pacific/Auckland").1, 4);
    }

    #[test]
    fn season_now_names_the_next_start() {
        let z = CString::new("Europe/Warsaw").unwrap();
        let mut out = NoriSeasonNow { season: 0, next: 0, next_month: 0, next_day: 0 };
        unsafe { nori_ios_season_now(OCTOBER, 0, z.as_ptr(), &mut out) };
        assert_eq!((out.season, out.next, out.next_month, out.next_day), (2, 3, 11, 7));
    }

    #[test]
    fn a_dark_cover_dyes_the_day_paper_dark() {
        let (light, dark) = (Season::Spring.palette(false), Season::Spring.palette(true));
        let navy = [0x10, 0x18, 0x40, 0xFF].repeat(16 * 16);
        let pale = [0xF0, 0xE8, 0xD0, 0xFF].repeat(16 * 16);
        let lum = |c: u32| nori_look::color::luminance(c);
        let on_navy = cover_page(&navy, 16, 16, light, dark, false);
        let on_pale = cover_page(&pale, 16, 16, light, dark, false);
        assert!(lum(on_navy[dress::BACKGROUND]) < 0.2 && lum(on_navy[dress::ON]) > 0.5, "{on_navy:x?}");
        assert!(lum(on_pale[dress::BACKGROUND]) > 0.5 && lum(on_pale[dress::ON]) < 0.2, "{on_pale:x?}");
    }

    #[test]
    fn a_mix_card_wears_its_mixs_dye() {
        let (light, dark) = (of(Season::Summer, false), of(Season::Summer, true));
        let id = CString::new("favourites").unwrap();
        let mut out = NoriPalette { page: 0, surface: 0, ink: 0, accent: 0, on: 0, quiet: 0 };
        unsafe { nori_ios_mix_card(id.as_ptr(), &light, &dark, 1, &mut out) };
        let want = nori_core::mixes::board::mix_card_colours("favourites".into(), (&light).into(), (&dark).into(), true);
        assert_eq!(out, want.into());
    }
}
