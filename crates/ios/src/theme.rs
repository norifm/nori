//! The season theme's colors for the UIKit app: `nori_look::theme` on the device's date and zone.

use std::ffi::{c_char, CStr};

use nori_look::theme::{palette, roles, season_now, southern, utc_date, Season};

/// `nori_look::theme::Palette` (ARGB) and its secondary text color, as `NoriPalette` in the header.
#[repr(C)]
pub struct NoriPalette {
    page: u32,
    surface: u32,
    ink: u32,
    accent: u32,
    on: u32,
    quiet: u32,
}

/// `nori_look::theme::SeasonNow`, as `NoriSeasonNow` in the header: seasons are 0 spring to 3 winter.
#[repr(C)]
pub struct NoriSeasonNow {
    season: i32,
    next: i32,
    next_month: u32,
    next_day: u32,
}

/// The season a `season` setting value names; none for `FOLLOW`, which is the calendar's.
fn pick(setting: &str) -> Option<Season> {
    match setting {
        "SPRING" => Some(Season::Spring),
        "SUMMER" => Some(Season::Summer),
        "AUTUMN" => Some(Season::Autumn),
        "WINTER" => Some(Season::Winter),
        _ => None,
    }
}

/// The device's local month and day, and whether its zone is south of the equator.
fn local_date(unix_s: i64, offset_min: i32, zone: &str) -> (u32, u32, bool) {
    let (month, day) = utc_date(unix_s + i64::from(offset_min) * 60);
    (month, day, southern(zone))
}

/// The palette for the `season` setting's value (`FOLLOW`, `SPRING`, ...), on a local date given as
/// seconds since 1970 and the zone's offset from UTC in minutes and its IANA id.
///
/// # Safety
/// `season` and `zone` are NUL-terminated UTF-8; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_palette(
    season: *const c_char,
    dark: i32,
    unix_s: i64,
    offset_min: i32,
    zone: *const c_char,
    out: *mut NoriPalette,
) {
    // SAFETY: the caller's.
    let (season, zone) = unsafe { (CStr::from_ptr(season).to_string_lossy(), CStr::from_ptr(zone).to_string_lossy()) };
    let (month, day, south) = local_date(unix_s, offset_min, &zone);
    let p = palette(pick(&season), month, day, south, dark != 0, false);
    let quiet = roles(p).on_surface_variant;
    // SAFETY: the caller's.
    unsafe { out.write(NoriPalette { page: p.page, surface: p.surface, ink: p.ink, accent: p.accent, on: p.on, quiet }) };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    // 2026-10-19 12:00 UTC and 2026-02-04 12:00 UTC.
    const OCTOBER: i64 = 1_792_411_200;
    const FEBRUARY: i64 = 1_770_206_400;

    fn colors(season: &str, dark: i32, unix_s: i64, offset: i32, zone: &str) -> [u32; 5] {
        let (s, z) = (CString::new(season).unwrap(), CString::new(zone).unwrap());
        let mut out = NoriPalette { page: 0, surface: 0, ink: 0, accent: 0, on: 0, quiet: 0 };
        unsafe { nori_ios_palette(s.as_ptr(), dark, unix_s, offset, z.as_ptr(), &mut out) };
        [out.page, out.surface, out.ink, out.accent, out.on]
    }

    fn of(season: Season, dark: bool) -> [u32; 5] {
        let p = season.palette(dark);
        [p.page, p.surface, p.ink, p.accent, p.on]
    }

    #[test]
    fn a_fixed_season_ignores_the_date() {
        assert_eq!(colors("WINTER", 0, OCTOBER, 0, "Europe/Warsaw"), of(Season::Winter, false));
        assert_eq!(colors("SPRING", 1, OCTOBER, 0, "Europe/Warsaw"), of(Season::Spring, true));
    }

    #[test]
    fn following_wears_the_calendars_season_and_flips_in_the_south() {
        assert_eq!(colors("FOLLOW", 1, OCTOBER, 0, "Europe/Warsaw"), of(Season::Autumn, true));
        assert_eq!(colors("FOLLOW", 1, OCTOBER, 0, "Australia/Sydney"), of(Season::Spring, true));
    }

    #[test]
    fn following_blends_around_a_season_start() {
        let blended = colors("FOLLOW", 0, FEBRUARY, 0, "Europe/Warsaw");
        assert_ne!(blended, of(Season::Winter, false));
        assert_ne!(blended, of(Season::Spring, false));
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
}
