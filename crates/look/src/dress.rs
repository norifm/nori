//! A page's full colour table, computed once per cover or theme; platforms only look entries up (and
//! [`mix`] two tables during a cross-fade).
//!
//! Indexed by the constants below; entries in [`NOT_COLOURS`] hold a flag or `f32` bits. Android mirrors
//! the indices in `CoverLook.kt`. Rules are exact ports of the former Compose code (see `compose`).

use crate::color::{calculate_contrast, color_to_hsl, luminance, WHITE};
use crate::compose::{blend, lerp, veil, with_alpha};
use crate::theme::Dye;

// Page colours from `cover::derive`.
pub const EDGE: usize = 0;
pub const BACKGROUND: usize = 1;
pub const ON: usize = 2;
/// Text's softer variant: `ON` at 66 %.
pub const ON_VARIANT: usize = 3;
/// The accent, which is the theme's primary.
pub const ACCENT: usize = 4;
/// Colour of the sleeve's faded bottom.
pub const MELT: usize = 5;

// Theme roles.
pub const ON_PRIMARY: usize = 6;
pub const SURFACE_VARIANT: usize = 7;
pub const SURFACE_CONTAINER: usize = 8;
pub const SURFACE_CONTAINER_HIGH: usize = 9;
pub const SECONDARY_CONTAINER: usize = 10;
pub const OUTLINE_VARIANT: usize = 11;

// Button plates, darker as the page gets paler.
/// `f32` bits: 0 on a dark page, 1 on a pale ("paper") page.
pub const PAPER: usize = 12;
/// The prominent pill (Play): the page's text colour, and the page as its ink.
pub const PILL: usize = 13;
pub const PILL_INK: usize = 14;
/// Secondary pill plate; its ink is [`TINT_INK`].
pub const PILL_PLATE: usize = 15;
/// Ink on a secondary plate: accent on dark, text colour on paper.
pub const TINT_INK: usize = 16;
/// Search field / chip (8 %), form field / card (7 %), switch off (16 %).
pub const FIELD: usize = 17;
pub const FORM: usize = 18;
pub const SWITCH_OFF: usize = 19;
/// Text at 13 % and 6 % over the page (cover placeholder tones, tile plates).
pub const VEIL_13: usize = 20;
pub const VEIL_6: usize = 21;
/// Text at 10 % over the page (lyrics timing pill).
pub const VEIL_10: usize = 22;

// Text alphas used by the player.
pub const ON_60: usize = 23;
pub const ON_45: usize = 24;
pub const ON_55: usize = 25;
pub const ON_22: usize = 26;
pub const ON_85: usize = 27;
pub const ON_35: usize = 28;
pub const ON_80: usize = 29;
pub const ON_VARIANT_70: usize = 30;

// Floating chrome: the now-playing bar and tabs are outlines over the page; a message floats on the slab.
pub const CHROME_SLAB: usize = 31;
pub const CHROME_CONTENT: usize = 32;
pub const CHROME_PAGE: usize = 33;
/// The slab's outline.
pub const CHROME_EDGE: usize = 34;
/// Chrome secondary text (65 %) and icon (75 %).
pub const CHROME_CONTENT_65: usize = 35;
pub const CHROME_CONTENT_75: usize = 36;
/// Inside the chrome's outline: page at 88 %, the page under it showing faintly through.
pub const CHROME_FILL: usize = 37;
/// The see-through chrome's outline: text at 20 %.
pub const CHROME_OUTLINE: usize = 48;

/// 1 for dark status bar icons (dark text), else 0.
pub const STATUS_LIGHT: usize = 38;
/// `f32` bits: blurred band tint strength (1 on white, cream or black pages); a float so cross-fades
/// mix it. The next three are per-channel luminance scales for [`crate::sleeve::band_matrix`].
pub const BAND_TINT: usize = 39;
pub const BAND_KR: usize = 40;
pub const BAND_KG: usize = 41;
pub const BAND_KB: usize = 42;

/// Album artwork dissolve: edge at 40 %, then edge/page blend at 86 %.
pub const HERO_EDGE: usize = 43;
pub const HERO_MID: usize = 44;
/// Player floor gradient: page at 0, 22 and 75 %.
pub const FLOOR_0: usize = 45;
pub const FLOOR_22: usize = 46;
pub const FLOOR_75: usize = 47;

pub const LEN: usize = 49;

/// Entries that are not colours (mixed as numbers).
pub(crate) const NOT_COLOURS: [usize; 6] = [PAPER, STATUS_LIGHT, BAND_TINT, BAND_KR, BAND_KG, BAND_KB];

/// Ink on a light accent.
const INK_ON_LIGHT: u32 = 0xFF0D_0D0D;

/// White ink on `fill` when it reads at 3:1 (WCAG's floor for large bold text), else near-black.
pub(crate) fn ink_on(fill: u32) -> u32 {
    if calculate_contrast(WHITE, fill) >= 3.0 { WHITE } else { INK_ON_LIGHT }
}

/// Theme roles for a page without a cover.
#[derive(Debug, Clone, Copy)]
pub struct Scheme {
    pub background: u32,
    pub on: u32,
    pub on_variant: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub surface_variant: u32,
    pub surface_container: u32,
    pub surface_container_high: u32,
    pub secondary_container: u32,
    pub outline_variant: u32,
}

/// Table for a cover-tinted page, from [`crate::cover::CoverColours`].
pub fn page(edge: u32, background: u32, on: u32, accent: u32, melt: u32) -> [u32; LEN] {
    let mut out = [0u32; LEN];
    out[EDGE] = edge;
    out[MELT] = melt;
    let s = Scheme {
        background,
        on,
        on_variant: with_alpha(on, 0.66),
        primary: accent,
        on_primary: ink_on(accent),
        surface_variant: veil(on, 0.10, background),
        surface_container: veil(on, 0.07, background),
        surface_container_high: veil(on, 0.11, background),
        secondary_container: veil(on, 0.14, background),
        outline_variant: veil(on, 0.14, background),
    };
    dress(&s, Some(edge), &mut out);
    out
}

/// Table for a theme-coloured page; `EDGE` and `MELT` are the background.
pub fn plain(s: &Scheme) -> [u32; LEN] {
    let mut out = [0u32; LEN];
    out[EDGE] = s.background;
    out[MELT] = s.background;
    dress(s, None, &mut out);
    out
}

fn dress(s: &Scheme, edge: Option<u32>, out: &mut [u32; LEN]) {
    let (bg, on, primary) = (s.background, s.on, s.primary);
    out[BACKGROUND] = bg;
    out[ON] = on;
    out[ON_VARIANT] = s.on_variant;
    out[ACCENT] = primary;
    out[ON_PRIMARY] = s.on_primary;
    out[SURFACE_VARIANT] = s.surface_variant;
    out[SURFACE_CONTAINER] = s.surface_container;
    out[SURFACE_CONTAINER_HIGH] = s.surface_container_high;
    out[SECONDARY_CONTAINER] = s.secondary_container;
    out[OUTLINE_VARIANT] = s.outline_variant;

    // Continuous in lightness so a cross-fade never flips a plate in one frame.
    let paper = ((luminance(bg) - 0.40) / 0.40).clamp(0.0, 1.0);
    out[PAPER] = paper.to_bits();
    out[PILL] = on;
    out[PILL_INK] = bg;
    out[PILL_PLATE] = veil(on, 0.12 + 0.05 * paper, bg);
    out[TINT_INK] = lerp(primary, on, paper);
    out[FIELD] = veil(on, 0.08, bg);
    out[FORM] = veil(on, 0.07, bg);
    out[SWITCH_OFF] = veil(on, 0.16, bg);
    out[VEIL_13] = veil(on, 0.13, bg);
    out[VEIL_6] = veil(on, 0.06, bg);
    out[VEIL_10] = veil(on, 0.10, bg);

    for (i, a) in [(ON_60, 0.6), (ON_45, 0.45), (ON_55, 0.55), (ON_22, 0.22), (ON_85, 0.85), (ON_35, 0.35), (ON_80, 0.8)] {
        out[i] = with_alpha(on, a);
    }
    out[ON_VARIANT_70] = with_alpha(s.on_variant, 0.7);

    // The slab: page lifted by the text colour (more on dark pages to stand out from AMOLED black),
    // with some of the cover's edge colour on tinted pages.
    let dark = luminance(bg) < 0.5;
    let lift = if dark { 0.20 } else { 0.11 };
    out[CHROME_SLAB] = veil(on, lift, edge.map_or(bg, |e| blend(bg, e, 0.30)));
    out[CHROME_CONTENT] = on;
    out[CHROME_PAGE] = bg;
    out[CHROME_EDGE] = with_alpha(on, if dark { 0.14 } else { 0.07 });
    out[CHROME_CONTENT_65] = with_alpha(on, 0.65);
    out[CHROME_CONTENT_75] = with_alpha(on, 0.75);
    out[CHROME_FILL] = with_alpha(bg, 0.88);
    out[CHROME_OUTLINE] = with_alpha(on, 0.20);

    out[STATUS_LIGHT] = (luminance(on) < 0.5) as u32;
    // White, cream and black pages have a single-tint wash, so the blurred band takes that tint too;
    // pure black has no hue to take, so its band goes grey rather than black.
    let l = color_to_hsl(bg)[2];
    let tinted = l > 0.85 || l < 0.08;
    out[BAND_TINT] = (if tinted { 1.0f32 } else { 0.0 }).to_bits();
    let (r, g, b) = (crate::color::red(bg) as f32 / 255.0, crate::color::green(bg) as f32 / 255.0, crate::color::blue(bg) as f32 / 255.0);
    let top = r.max(g).max(b).max(0.01);
    let k = if tinted && top > 0.01 { [r / top, g / top, b / top] } else { [1.0, 1.0, 1.0] };
    out[BAND_KR] = k[0].to_bits();
    out[BAND_KG] = k[1].to_bits();
    out[BAND_KB] = k[2].to_bits();

    let e = out[EDGE];
    out[HERO_EDGE] = with_alpha(e, crate::sleeve::HERO_ALPHAS[1]);
    out[HERO_MID] = with_alpha(blend(e, bg, 0.55), crate::sleeve::HERO_ALPHAS[2]);
    out[FLOOR_0] = with_alpha(bg, 0.0);
    out[FLOOR_22] = with_alpha(bg, 0.22);
    out[FLOOR_75] = with_alpha(bg, 0.75);
}

/// `t` with the theme's accent in place of its cover's ([`crate::theme::accent_on`]), and everything
/// dressed in the accent dressed again.
pub fn accented(t: &[u32; LEN], light: u32, dark: u32) -> [u32; LEN] {
    let (bg, on) = (t[BACKGROUND], t[ON]);
    let primary = crate::theme::accent_on(bg, on, light, dark);
    let s = Scheme {
        background: bg,
        on,
        on_variant: t[ON_VARIANT],
        primary,
        on_primary: ink_on(primary),
        surface_variant: t[SURFACE_VARIANT],
        surface_container: t[SURFACE_CONTAINER],
        surface_container_high: t[SURFACE_CONTAINER_HIGH],
        secondary_container: t[SECONDARY_CONTAINER],
        outline_variant: t[OUTLINE_VARIANT],
    };
    let mut out = [0u32; LEN];
    out[EDGE] = t[EDGE];
    out[MELT] = t[MELT];
    dress(&s, Some(t[EDGE]), &mut out);
    out
}

/// `t`, a cover's page ([`page`]), on the season's paper dyed with the cover's edge: `dye` is
/// [`Dye::new`] of `t[EDGE]`. The roles are laid out over that paper as the theme's are
/// ([`crate::theme::roles`]); the edge stays, for the artwork's dissolve.
pub fn dyed(t: &[u32; LEN], dye: &Dye) -> [u32; LEN] {
    let r = crate::theme::roles(dye.palette());
    let s = Scheme {
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
    let mut out = [0u32; LEN];
    out[EDGE] = t[EDGE];
    out[MELT] = dye.wash(t[MELT]);
    dress(&s, None, &mut out);
    out
}

/// Cross-fade from `a` to `b` into `out` (no allocation): colours via Compose `lerp`, numbers linearly,
/// status bar icons switching at half way.
pub fn mix(a: &[u32; LEN], b: &[u32; LEN], t: f32, out: &mut [u32; LEN]) {
    let t = t.clamp(0.0, 1.0);
    for i in 0..LEN {
        out[i] = if i == STATUS_LIGHT {
            if t < 0.5 { a[i] } else { b[i] }
        } else if NOT_COLOURS.contains(&i) {
            let x = f32::from_bits(a[i]);
            (x + (f32::from_bits(b[i]) - x) * t).to_bits()
        } else {
            lerp(a[i], b[i], t)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_flips_icons_half_way() {
        let dark = page(0xFF20_1010, 0xFF20_1010, WHITE, 0xFFE0_4040, 0xFF20_1010);
        let white = page(WHITE, WHITE, 0xFF11_1111, 0xFFE0_4040, WHITE);
        let mut out = [0u32; LEN];
        mix(&dark, &white, 0.0, &mut out);
        assert_eq!(out, dark);
        mix(&dark, &white, 1.0, &mut out);
        assert_eq!(out, white);
        mix(&dark, &white, 0.25, &mut out);
        assert_eq!(f32::from_bits(out[PAPER]), 0.25);
        assert_eq!(out[STATUS_LIGHT], 0);
        assert_eq!(out[BACKGROUND], lerp(dark[BACKGROUND], white[BACKGROUND], 0.25));
        mix(&dark, &white, 0.5, &mut out);
        assert_eq!(out[STATUS_LIGHT], 1);
    }

    /// Output of the former Compose code (ui-graphics 1.12, JVM): edge, background, on, accent, entries.
    const PAGES: &[(u32, u32, u32, u32, [u32; 12])] = include!("dress_pages.in");

    #[test]
    fn matches_compose() {
        for &(edge, bg, on, accent, want) in PAGES {
            let t = page(edge, bg, on, accent, edge);
            let got = [
                t[ON_VARIANT], t[ON_PRIMARY], t[SURFACE_VARIANT], t[SURFACE_CONTAINER], t[SURFACE_CONTAINER_HIGH], t[SECONDARY_CONTAINER],
                t[PILL_PLATE], t[TINT_INK],
                t[CHROME_SLAB], t[CHROME_EDGE], t[HERO_MID], t[FIELD],
            ];
            assert_eq!(got, want, "page {bg:08x} on {on:08x}, accent {accent:08x}");
        }

        // Paper follows page lightness.
        let dark = page(0xFF20_1010, 0xFF20_1010, WHITE, 0xFFE0_4040, 0xFF20_1010);
        let white = page(WHITE, WHITE, 0xFF11_1111, 0xFFE0_4040, WHITE);
        assert_eq!(f32::from_bits(dark[PAPER]), 0.0);
        assert_eq!(f32::from_bits(white[PAPER]), 1.0);
        assert_eq!((dark[STATUS_LIGHT], white[STATUS_LIGHT]), (0, 1));
        // Status icons follow the text, also on a mid-tone page.
        let mid = page(0xFFB0_A080, 0xFFB0_A080, 0xFF0D_0D0D, 0xFF0A_B4E3, 0xFFB0_A080);
        assert_eq!(mid[STATUS_LIGHT], 1);
        assert_eq!((f32::from_bits(dark[BAND_TINT]), f32::from_bits(white[BAND_TINT])), (0.0, 1.0));
        assert_eq!(f32::from_bits(white[BAND_KG]), 1.0);
        // Midnight's black page keeps the band's light and dark, grey.
        let black = page(0xFF00_0000, 0xFF00_0000, WHITE, 0xFFE0_4040, 0xFF00_0000);
        assert_eq!([black[BAND_KR], black[BAND_KG], black[BAND_KB]].map(f32::from_bits), [1.0; 3]);
    }

    #[test]
    fn the_theme_accent_reads_on_any_cover() {
        let (light, dark) = (0xFFB6_4D1D, 0xFFE8_743B);
        for bg in [0xFF10_1010u32, 0xFF20_3A60, 0xFF55_5555, 0xFFE8_7040, 0xFFB0_A080, WHITE] {
            let on = if luminance(bg) < 0.5 { WHITE } else { 0xFF11_1111 };
            let cover = page(bg, bg, on, 0xFF30_C0F0, bg);
            let t = accented(&cover, light, dark);
            assert!(calculate_contrast(t[ACCENT], bg) >= 4.5, "accent {:08x} on {bg:08x}", t[ACCENT]);
            assert_eq!((t[BACKGROUND], t[ON], t[EDGE], t[MELT], t[CHROME_SLAB]), (cover[BACKGROUND], cover[ON], cover[EDGE], cover[MELT], cover[CHROME_SLAB]));
            assert_eq!(t[TINT_INK], lerp(t[ACCENT], on, f32::from_bits(t[PAPER])));
        }
        // The dark theme's accent on a dark cover, the light one's on a pale one, as they are where they read.
        assert_eq!(accented(&page(0xFF10_1010, 0xFF10_1010, WHITE, 0xFF30_C0F0, 0xFF10_1010), light, dark)[ACCENT], dark);
        assert_eq!(accented(&page(WHITE, WHITE, 0xFF11_1111, 0xFF30_C0F0, WHITE), light, dark)[ACCENT], light);
    }

    use crate::theme::{palette, Palette, Season};

    const SEASONS: [Season; 4] = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];

    /// `season`'s light and dark palettes as the app wears them with or without `amoled`.
    fn worn(season: Season, amoled: bool) -> (Palette, Palette) {
        (palette(Some(season), 1, 1, false, false, amoled), palette(Some(season), 1, 1, false, true, amoled))
    }

    /// A cover's page ending in `edge`, dyed.
    fn dyed_page(edge: u32, season: Season, dark: bool, amoled: bool) -> ([u32; LEN], Dye) {
        let (light, dark_palette) = worn(season, amoled);
        let dye = Dye::new(edge, light, dark_palette, dark);
        let text = if luminance(edge) < 0.5 { WHITE } else { 0xFF11_1111 };
        (dyed(&page(edge, edge, text, 0xFF30_C0F0, edge), &dye), dye)
    }

    /// Dark, deep red, near-black, cream, white, saturated, a pale and a mid grey.
    const EDGES: [u32; 11] =
        [0xFF10_1418, 0xFF3A_1210, 0xFF1C_1C1E, 0xFFF0_E8D8, WHITE, 0xFFE0_2040, 0xFF20_C040, 0xFF20_40E0, 0xFFF0_D020, 0xFFC8_C8C8, 0xFF77_7777];

    #[test]
    fn every_dyed_page_reads() {
        for season in SEASONS {
            for (dark, amoled) in [(false, false), (false, true), (true, false), (true, true)] {
                for edge in EDGES {
                    let (t, dye) = dyed_page(edge, season, dark, amoled);
                    let at = format!("{season:?} dark {dark} amoled {amoled}, edge {edge:08x}");
                    for (what, fg) in [("ink", t[ON]), ("quiet ink", t[ON_VARIANT]), ("accent", t[ACCENT])] {
                        let c = calculate_contrast(fg, t[BACKGROUND]);
                        assert!(c >= 4.5, "{at}: {what} {fg:08x} on {:08x} reads at {c:.2}", t[BACKGROUND]);
                    }
                    // The wash, dyed as the page is, reads under the same text whatever its colours.
                    for wash in EDGES.map(|w| dye.wash(w)) {
                        for (what, fg) in [("ink", t[ON]), ("quiet ink", t[ON_VARIANT])] {
                            let c = calculate_contrast(fg, wash);
                            assert!(c >= 4.5, "{at}: {what} on the wash {wash:08x} reads at {c:.2}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_dyed_page_takes_its_depth_and_hue_from_the_cover() {
        let (night, cream, red, grey) = (0xFF3A_1210u32, 0xFFF0_E8D8u32, 0xFFE8_5060u32, 0xFFC8_C8C8u32);
        for season in SEASONS {
            for (edge, dark, want_dark) in [(night, false, true), (cream, true, true), (cream, false, false), (red, false, false), (red, true, true)] {
                let (t, _) = dyed_page(edge, season, dark, false);
                assert_eq!(luminance(t[BACKGROUND]) < 0.5, want_dark, "{season:?} dark {dark}, edge {edge:08x}: page {:08x}", t[BACKGROUND]);
                assert_eq!(luminance(t[ON]) > 0.5, want_dark, "{season:?}: the text follows the page");
            }
            let hue = |c: u32| color_to_hsl(c)[0];
            let (paper, _) = worn(season, false);
            for dark in [false, true] {
                let (t, dye) = dyed_page(red, season, dark, false);
                let off = (hue(t[BACKGROUND]) - hue(red)).abs();
                assert!(off.min(360.0 - off) < 20.0, "{season:?} dark {dark}: {:08x} is not dyed red", t[BACKGROUND]);
                assert_ne!(dye.wash(0xFF30_9050), t[BACKGROUND], "{season:?} dark {dark}: the wash keeps the cover's other colors");
            }
            // A grey cover gives the paper no hue of its own.
            let (t, _) = dyed_page(grey, season, false, false);
            let ok = crate::color::Oklab::new();
            assert!(ok.lightness_chroma(t[BACKGROUND]).1 <= ok.lightness_chroma(paper.page).1 + 0.003, "{season:?}: grey dyed {:08x}", t[BACKGROUND]);
            // On AMOLED the dark paper is black-based.
            let (black, _) = dyed_page(night, season, true, true);
            let (dark, _) = dyed_page(night, season, true, false);
            assert!(luminance(black[BACKGROUND]) < luminance(dark[BACKGROUND]), "{season:?}");
        }
    }

    #[test]
    fn play_is_the_pages_ink_on_its_paper() {
        for (bg, on) in [(0xFF20_3A60u32, WHITE), (0xFFF6_F0E6, 0xFF24_1D16)] {
            let cover = page(bg, bg, on, 0xFF30_C0F0, bg);
            for t in [cover, accented(&cover, 0xFFB6_4D1D, 0xFFE8_743B)] {
                assert_eq!((t[PILL], t[PILL_INK]), (on, bg), "page {bg:08x}");
            }
        }
        assert_eq!(ink_on(0xFFE0_4040), WHITE);
        assert_eq!(ink_on(0xFFA0_A0A0), INK_ON_LIGHT);
    }

}
