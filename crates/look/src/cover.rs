//! Page colours derived from a cover, ported from the Android app; rules were tuned by eye against real
//! albums (named where relevant).
//!
//! To avoid a seam where the artwork ends, the fade starts from the average of the cover's bottom rows
//! (`edge`), not its dominant colour, and ends on a page colour with enough contrast for text.

use crate::color::*;
use crate::palette;
use crate::random::JavaRandom;

/// Fraction of the sleeve's height that fades into the page.
pub const MELT: f32 = 0.19;

/// Side of the wash computation grid (16 lost all shape).
pub const WASH: usize = 32;

/// Side of the wash handed to the GPU: upsampled and smoothed here, since stretching [`WASH`] directly
/// showed visible steps.
pub const WASH_OUT: usize = WASH * 4;

/// Colours derived from a cover (ARGB).
#[derive(Debug, Clone, PartialEq)]
pub struct CoverColours {
    /// Colour at the cover's bottom, where the fade starts.
    pub edge: u32,
    /// Page background.
    pub background: u32,
    /// Text on the page.
    pub on: u32,
    pub accent: u32,
    /// Blurred cover muted towards the page, [`WASH_OUT`]² pixels; `None` on AMOLED black.
    pub wash: Option<Vec<u32>>,
    /// Average of the wash's bottom [`MELT`] rows (visible where the sleeve fades); `edge` without a wash.
    pub wash_edge: u32,
}

/// Dark page for white text: the colour at its own lightness (at most [`PAGE_LIGHTEST`]), darkened only
/// until its luminance allows white text at about 8:1, and never more colourful than `chroma`, its
/// source's (keeping HSL saturation while darkening Lover's pink F3BBD7 made a wine 8C2157).
const PAGE_LIGHTEST: f32 = 0.34;
const PAGE_MAX_LUMA: f32 = 0.081;

fn dark_page(hsl: [f32; 3], chroma: f32, ok: &Oklab) -> u32 {
    let hi = hsl[2].clamp(0.04, PAGE_LIGHTEST);
    let mut sat = (hsl[1] * 0.95).min(0.62);
    let colour_at = |s: f32| ok.lightness_chroma(hsl_to_color([hsl[0], s, hi])).1;
    if colour_at(sat) > chroma {
        sat = bisect(0.0, sat, |s| colour_at(s) <= chroma);
    }
    let at = |l: f32| hsl_to_color([hsl[0], sat, l]);
    if luminance(at(hi)) <= PAGE_MAX_LUMA {
        return at(hi);
    }
    // The lightest allowed lightness.
    at(bisect(0.04, hi, |l| luminance(at(l)) <= PAGE_MAX_LUMA))
}

/// Binary search between a value that is `ok` and one that is not (12 steps, ~0.001): the last `ok`.
fn bisect(mut good: f32, mut bad: f32, ok: impl Fn(f32) -> bool) -> f32 {
    for _ in 0..12 {
        let mid = (good + bad) / 2.0;
        if ok(mid) {
            good = mid
        } else {
            bad = mid
        }
    }
    good
}

/// Contrast for text (WCAG AA), on the page and in the accent.
const TEXT_CONTRAST: f64 = 4.5;
const INK: u32 = 0xFF0D_0D0D;

/// The text colour that reads better on `page`: white or near-black.
fn text_on(page: u32) -> u32 {
    if calculate_contrast(WHITE, page) >= calculate_contrast(INK, page) { WHITE } else { INK }
}

/// `colour` as a page: itself when white or near-black text reads on it, which is almost always; in the
/// narrow band of luminance where neither does, the least lightness step that lets the nearer one.
fn page_of(colour: u32) -> u32 {
    if calculate_contrast(text_on(colour), colour) >= TEXT_CONTRAST {
        return colour;
    }
    let hsl = color_to_hsl(colour);
    let at = |l: f32| hsl_to_color([hsl[0], hsl[1], l]);
    let reads = |l: f32| calculate_contrast(text_on(at(l)), at(l)) >= TEXT_CONTRAST;
    let darker = at(bisect(0.0, hsl[2], reads));
    let lighter = at(bisect(1.0, hsl[2], reads));
    if hsl[2] - color_to_hsl(darker)[2] <= color_to_hsl(lighter)[2] - hsl[2] { darker } else { lighter }
}

/// Derives page colours from `w` x `h` ARGB pixels (at least 1 x 1: every decoder rejects empty images).
/// The page is a colour of the cover, whatever the app's theme: the one it dissolves into most softly
/// ([`Seam`]). With `amoled` it is black, without a wash.
pub fn derive(pixels: &[u32], w: usize, h: usize, amoled: bool) -> CoverColours {
    let p = palette::generate(pixels, w, h, 16);
    let ok = Oklab::new();
    let seam = Seam::of(pixels, w, h, &ok);
    let body = dominant(pixels, w, h, seam.mean, &ok);
    let body_grey = ok.lightness_chroma(body).1 < GREY;
    let source = seam.frame.unwrap_or(seam.colour);
    let (source_light, source_colour) = ok.lightness_chroma(source);
    // Paper and ink: a near-neutral page, light or dark. Their tint fades out below 0.016 chroma and is
    // gone under 0.006, so JPEG noise or a scanner cast does not tint the page while real creams stay.
    let neutral = source_colour < PAPER_CHROMA;
    let paper = neutral && source_light >= 0.79;
    let ink = source_colour < GREY && source_light < 0.21;
    let mut source_hsl = color_to_hsl(source);
    source_hsl[1] *= ((source_colour - 0.006) / 0.010).clamp(0.0, 1.0);
    let background = if amoled { BLACK } else { page_of(hsl_to_color(source_hsl)) };
    let on = text_on(background);
    // A monochrome sleeve's accent is its own tint, unless a vivid colour covers 1 % of it (Parachutes'
    // orange globe, Turn On the Bright Lights' red box); smaller marks are specks.
    let vivid = p.vibrant.or(p.light_vibrant).filter(|s| s.population * 100 >= p.area && ok.lightness_chroma(s.rgb).1 >= VIVID);
    let accent = if paper {
        // Near-black in the paper's hue.
        hsl_to_color([source_hsl[0], source_hsl[1].min(0.25), 0.17])
    } else {
        let seed = if ink || body_grey {
            vivid.map_or(body, |s| s.rgb)
        } else {
            p.vibrant.or(p.light_vibrant).or(p.light_muted).or(p.dominant).map_or(body, |s| s.rgb)
        };
        readable(seed, background, on, &ok)
    };
    // The fade starts from the frame, or from the seam's average mostly towards the page.
    let edge = match seam.frame {
        Some(colour) => colour,
        None if amoled => seam.mean,
        None => blend_argb(seam.mean, background, 0.75),
    };
    let wash = (!amoled).then(|| wash_of(pixels, w, h, background, on, paper, ink));
    CoverColours {
        edge,
        background,
        on,
        accent,
        wash_edge: wash.as_ref().map_or(edge, |w| w.1),
        wash: wash.map(|w| w.0),
    }
}

/// The colours for a page that is always drawn in white (the desktop's Now Playing, as Music's): `c` when
/// its page is dark already; a light page (a paper sleeve, any page in the light theme) taken down to a
/// dark page of its own hue, and its wash and edges moved down by the same lightness, so the glow stays.
pub fn under_white(c: &CoverColours) -> CoverColours {
    if c.on == WHITE {
        return c.clone();
    }
    let page = color_to_hsl(c.background);
    let ok = Oklab::new();
    let background = dark_page(page, ok.lightness_chroma(c.background).1, &ok);
    let drop = page[2] - color_to_hsl(background)[2];
    let down = |argb: u32| {
        let mut hsl = color_to_hsl(argb);
        hsl[2] = (hsl[2] - drop).clamp(0.0, 1.0);
        hsl_to_color(hsl)
    };
    CoverColours {
        edge: down(c.edge),
        background,
        on: WHITE,
        accent: readable(c.accent, background, WHITE, &ok),
        wash: c.wash.as_ref().map(|w| w.iter().map(|&p| down(p)).collect()),
        wash_edge: down(c.wash_edge),
    }
}

/// Blend of each wash pixel towards the flat page colour (2/3 looked flat).
const MUTE: f32 = 0.38;

/// The page wash: the cover downscaled to [`WASH`], heavily blurred, with lightness held near the page's
/// and saturation capped so text contrast holds. Returns the upsampled wash and its bottom average.
fn wash_of(pixels: &[u32], w: usize, h: usize, background: u32, on: u32, paper: bool, ink: bool) -> (Vec<u32>, u32) {
    let ink_or_paper = paper || ink;
    let dark = on == WHITE;
    let mut px = scale_bilinear(pixels, w, h, WASH, WASH);
    // Five passes: at three, strong central shapes still showed.
    for _ in 0..5 {
        blur(&mut px);
    }
    let page_hsl = color_to_hsl(background);
    let mut mean_l = 0f32;
    for &p in &px {
        mean_l += color_to_hsl(p)[2];
    }
    mean_l /= px.len() as f32;
    // Max lightness deviation from the page (0.11 keeps white text at about 5:1 on dark pages).
    let spread = if paper {
        0.04
    } else if dark {
        0.11
    } else {
        0.055
    };
    let pull = if dark && !paper { 1.0 } else { 0.85 };
    let max_sat = if ink_or_paper {
        0.04
    } else if dark {
        0.62
    } else {
        0.40
    };
    let mute = if ink_or_paper { 0.78 } else { MUTE };
    for p in px.iter_mut() {
        let mut hsl = color_to_hsl(*p);
        if ink_or_paper {
            // Only lightness varies; hue and saturation are the page's.
            hsl[0] = page_hsl[0];
            hsl[1] = page_hsl[1];
        } else {
            hsl[1] = (hsl[1] * pull).min(max_sat);
        }
        // On ink pages never go darker than the page (it showed as a darker band under the sleeve).
        let floor = if ink { page_hsl[2] } else { page_hsl[2] - spread };
        hsl[2] = (page_hsl[2] + (hsl[2] - mean_l) * spread * 2.5).max(floor).min(page_hsl[2] + spread).clamp(0.0, 1.0);
        // Mostly back to the page colour so distinct hue regions read as a glow, not patches.
        *p = blend_argb(hsl_to_color(hsl), background, mute);
        // The page reads at 4.5:1; where the glow would not, it is the page.
        if calculate_contrast(on, *p) < TEXT_CONTRAST {
            *p = background;
        }
    }
    let first = ((WASH as f32 * (1.0 - MELT)) as usize).min(WASH - 1);
    let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
    for y in first..WASH {
        for x in 0..WASH {
            let p = px[y * WASH + x];
            r += red(p) as f32;
            g += green(p) as f32;
            b += blue(p) as f32;
        }
    }
    let n = ((WASH - first) * WASH * 255) as f32;
    (smooth(&px), from_floats(r / n, g / n, b / n))
}

/// Skia's `Bitmap.createScaledBitmap(..., filter = true)`, pixel-exact: 16.16 fixed-point positions
/// stepped along each row (accumulating rounding like Skia), 4-bit weights, truncated sum.
fn scale_bilinear(pixels: &[u32], w: usize, h: usize, dw: usize, dh: usize) -> Vec<u32> {
    let fixed = |v: f32| (v as f64 * 65536.0) as i32;
    let inv_x = 1.0f32 / (dw as f32 / w as f32);
    let inv_y = 1.0f32 / (dh as f32 / h as f32);
    let step = fixed(inv_x);
    let clamp = |i: i32, n: usize| i.clamp(0, n as i32 - 1) as usize;
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let fy = fixed((y as f32 + 0.5) * inv_y) - 0x8000;
        let (y0, y1, ty) = (clamp(fy >> 16, h), clamp((fy >> 16) + 1, h), (fy >> 12) & 15);
        let mut fx = fixed(0.5 * inv_x) - 0x8000;
        for _ in 0..dw {
            let (x0, x1, tx) = (clamp(fx >> 16, w), clamp((fx >> 16) + 1, w), (fx >> 12) & 15);
            let (a, b, c, d) = (pixels[y0 * w + x0], pixels[y0 * w + x1], pixels[y1 * w + x0], pixels[y1 * w + x1]);
            let ch = |f: fn(u32) -> i32| (f(a) * (16 - tx) * (16 - ty) + f(b) * tx * (16 - ty) + f(c) * (16 - tx) * ty + f(d) * tx * ty) >> 8;
            out.push(rgb(ch(red), ch(green), ch(blue)));
            fx += step;
        }
    }
    out
}

/// Upsamples [`WASH`] to [`WASH_OUT`]: bilinear, two box passes, then ±0.5 level dither against banding.
fn smooth(px: &[u32]) -> Vec<u32> {
    let n = WASH_OUT;
    let (mut r, mut g, mut b) = (vec![0f32; n * n], vec![0f32; n * n], vec![0f32; n * n]);
    let scale = (WASH - 1) as f32 / (n - 1) as f32;
    for y in 0..n {
        let fy = y as f32 * scale;
        let y0 = (fy as usize).min(WASH - 2);
        let ty = fy - y0 as f32;
        for x in 0..n {
            let fx = x as f32 * scale;
            let x0 = (fx as usize).min(WASH - 2);
            let tx = fx - x0 as f32;
            let (a, bb, c, d) = (px[y0 * WASH + x0], px[y0 * WASH + x0 + 1], px[(y0 + 1) * WASH + x0], px[(y0 + 1) * WASH + x0 + 1]);
            let lerp = |f: fn(u32) -> i32| {
                (f(a) as f32 * (1.0 - tx) + f(bb) as f32 * tx) * (1.0 - ty) + (f(c) as f32 * (1.0 - tx) + f(d) as f32 * tx) * ty
            };
            let i = y * n + x;
            r[i] = lerp(red);
            g[i] = lerp(green);
            b[i] = lerp(blue);
        }
    }
    for _ in 0..2 {
        box_blur(&mut r, n);
        box_blur(&mut g, n);
        box_blur(&mut b, n);
    }
    // Fixed seed: deterministic texture per cover.
    let mut noise = JavaRandom::new(0x5EED);
    let mut q = |v: f32| ((v + noise.next_float() - 0.5) as i32).clamp(0, 255);
    (0..n * n).map(|i| rgb(q(r[i]), q(g[i]), q(b[i]))).collect()
}

/// Separable radius-2 box blur of an n x n channel, in place.
fn box_blur(c: &mut [f32], n: usize) {
    let mut tmp = vec![0f32; c.len()];
    let pass = |src: &[f32], dst: &mut [f32], across: bool| {
        for y in 0..n {
            for x in 0..n {
                let at = |d: isize| {
                    let v = ((if across { x } else { y }) as isize + d).clamp(0, n as isize - 1) as usize;
                    src[if across { y * n + v } else { v * n + x }]
                };
                dst[y * n + x] = (-2..=2).map(at).sum::<f32>() / 5.0;
            }
        }
    };
    pass(c, &mut tmp, true);
    pass(&tmp, c, false);
}

/// Separable 3-tap box blur of the [`WASH`] grid: horizontally into a buffer, vertically back.
fn blur(px: &mut [u32]) {
    let mut out = vec![0u32; px.len()];
    let at = |v: isize| v.clamp(0, WASH as isize - 1) as usize;
    for horizontal in [true, false] {
        for y in 0..WASH {
            for x in 0..WASH {
                let (mut r, mut g, mut b) = (0, 0, 0);
                for d in -1..=1isize {
                    let i = if horizontal { y * WASH + at(x as isize + d) } else { at(y as isize + d) * WASH + x };
                    let p = if horizontal { px[i] } else { out[i] };
                    r += red(p);
                    g += green(p);
                    b += blue(p);
                }
                let v = rgb(r / 3, g / 3, b / 3);
                if horizontal {
                    out[y * WASH + x] = v;
                } else {
                    px[y * WASH + x] = v;
                }
            }
        }
    }
}

/// Histogram buckets: `HUES` hue buckets, then neutral (greys, whites), then dark.
const HUES: usize = 18;
const NEUTRAL: usize = HUES;
const DARK: usize = HUES + 1;

/// A cell at least this light (Oklab) and at most this colourful is paper: white, grey, cream or a pale
/// tint (DJ_Dave's Hardcore Software: lavender E0E2F4 under a grey grid).
const PAPER_LIGHT: f32 = 0.90;
const PAPER_CHROMA: f32 = 0.045;

/// Below this Oklab lightness a cell is ink whatever its colour, and below `DIM` if grey.
const INK_LIGHT: f32 = 0.15;
const DIM: f32 = 0.26;
/// Oklab chroma under which a colour is grey, from which a cell's hue vote counts fully, and from which
/// it is vivid (part of a subject, or an accent on a monochrome sleeve).
const GREY: f32 = 0.015;
const HUED: f32 = 0.04;
const VIVID: f32 = 0.08;

/// Cells per side of the grid [`dominant`] votes on. Each cell is the mean of its pixels, so thin lines
/// count the same at every resolution of a cover.
const CELLS: usize = 64;

/// The colour covering most of the cover (Palette's dominant filters out whole families). Grid cells
/// vote into 20° hue buckets (plus neutral and dark); the heaviest bucket with its neighbours wins, and
/// the result is that family's weighted mean. Large dull fields beat small vivid marks.
fn dominant(pixels: &[u32], w: usize, h: usize, fallback: u32, ok: &Oklab) -> u32 {
    if w == 0 || h == 0 {
        return fallback;
    }
    let (cols, rows) = (w.min(CELLS), h.min(CELLS));
    let mut weight = [0f32; HUES + 2];
    let (mut paper, mut ink, mut total) = (0f32, 0f32, 0f32);
    // Unweighted coloured-cell counts per hue, comparable with the paper and ink counts.
    let mut coloured = [0f32; HUES];
    // Hue bucket per cell (-1: none), to tell solid fields from lines.
    let mut grid = vec![-1i32; cols * rows];
    for gy in 0..rows {
        let (y0, y1) = (gy * h / rows, (gy + 1) * h / rows);
        for gx in 0..cols {
            let (x0, x1) = (gx * w / cols, (gx + 1) * w / cols);
            let cell = (y0..y1).flat_map(|y| pixels[y * w + x0..y * w + x1].iter().copied());
            let px = mean_rgb(cell).expect("every cell holds a pixel");
            let Vote { bucket, black, white, colour } = vote(px, ok);
            total += 1.0;
            if black {
                ink += 1.0;
            }
            if white {
                paper += 1.0;
            }
            if !black && !white && colour >= VIVID {
                coloured[bucket] += 1.0;
                grid[gy * cols + gx] = bucket as i32;
            }
            let wt = match bucket {
                // Full weight so JPEG specks cannot outvote a white or black field.
                DARK => 1.0,
                NEUTRAL => {
                    if white {
                        1.0
                    } else {
                        0.35
                    }
                }
                _ => 0.25 + 0.75 * (colour / HUED).min(1.0),
            };
            weight[bucket] += wt;
        }
    }
    // A hue scores with its two neighbours (one perceived colour spans several buckets); neutral and dark
    // score alone.
    let mut score = [0f32; HUES + 2];
    for i in 0..HUES {
        score[i] = weight[i] + weight[(i + HUES - 1) % HUES] + weight[(i + 1) % HUES];
    }
    score[NEUTRAL] = weight[NEUTRAL];
    score[DARK] = weight[DARK];
    let mut best = (0..score.len()).fold(0, |best, i| if score[i] > score[best] { i } else { best });
    // Majority paper or ink wins unless a colour family is the subject: >= 30 % of the cover and mostly
    // solid blocks (Amnesiac: 48 % black, 52 % red book).
    if total > 0.0 {
        let (mut family, mut family_share) = (0usize, 0f32);
        for i in 0..HUES {
            let share = (coloured[i] + coloured[(i + HUES - 1) % HUES] + coloured[(i + 1) % HUES]) / total;
            if share > family_share {
                family_share = share;
                family = i;
            }
        }
        // Solid cells: all eight neighbours in the family (lettering and rings do not count).
        let in_family = |v: i32| v >= 0 && {
            let d = (v - family as i32).unsigned_abs() as usize;
            d.min(HUES - d) <= 1
        };
        let mut solid = 0;
        for gy in 1..rows.saturating_sub(1) {
            for gx in 1..cols.saturating_sub(1) {
                if !in_family(grid[gy * cols + gx]) {
                    continue;
                }
                let mut around = (gy - 1..=gy + 1).flat_map(|y| (gx - 1..=gx + 1).map(move |x| y * cols + x));
                if around.all(|at| in_family(grid[at])) {
                    solid += 1;
                }
            }
        }
        let subject = family_share >= 0.30 && solid as f32 / total >= 0.18;
        // Paper yields to a subject only if the subject already won the score (Villains stays paper);
        // ink always yields to a subject.
        if paper / total >= 0.45 {
            best = if subject && paper / total < 0.60 && best != NEUTRAL { family } else { NEUTRAL };
        }
        if ink / total >= 0.45 {
            best = if subject && ink / total < 0.60 { family } else { DARK };
        }
    }
    let run: &[usize] = if best < HUES { &[(best + HUES - 1) % HUES, best, (best + 1) % HUES] } else { std::slice::from_ref(&best) };
    // Stripes finer than a cell vote for their mix, which no pixel has: then the commonest colour at all.
    cluster(pixels, w, h, |c| run.contains(&vote(c, ok).bucket)).or_else(|| cluster(pixels, w, h, |_| true)).unwrap_or(fallback)
}

/// A colour's bucket in [`dominant`], whether it is ink or paper, and its Oklab chroma.
struct Vote {
    bucket: usize,
    black: bool,
    white: bool,
    colour: f32,
}

fn vote(px: u32, ok: &Oklab) -> Vote {
    let (light, colour) = ok.lightness_chroma(px);
    let black = light < INK_LIGHT || (light < DIM && colour < GREY);
    let white = light >= PAPER_LIGHT && colour <= PAPER_CHROMA;
    let bucket = if black {
        DARK
    } else if white || colour < GREY {
        NEUTRAL
    } else {
        ((color_to_hsl(px)[0] / (360.0 / HUES as f32)) as usize).min(HUES - 1)
    };
    Vote { bucket, black, white, colour }
}

/// Point samples per side for [`cluster`].
const SAMPLES: usize = 128;

/// The commonest colour of the cover among those `wanted`: sampled pixels counted in 16-level bins,
/// the heaviest bin with its wanted neighbours, and their mean. A real colour of the cover, never the
/// mean of a family of them (a figure over stripes averaged into a colour that is nowhere).
fn cluster(pixels: &[u32], w: usize, h: usize, wanted: impl Fn(u32) -> bool) -> Option<u32> {
    let bin = |c: u32| ((red(c) >> 4) << 8 | (green(c) >> 4) << 4 | blue(c) >> 4) as usize;
    // Count and channel sums per bin.
    let mut bins = vec![[0u32; 4]; 4096];
    let (sx, sy) = (w.min(SAMPLES), h.min(SAMPLES));
    for y in 0..sy {
        for x in 0..sx {
            let c = pixels[(y * h / sy) * w + x * w / sx];
            let b = &mut bins[bin(c)];
            b[0] += 1;
            b[1] += red(c) as u32;
            b[2] += green(c) as u32;
            b[3] += blue(c) as u32;
        }
    }
    let mean = |b: &[u32; 4]| rgb((b[1] / b[0]) as i32, (b[2] / b[0]) as i32, (b[3] / b[0]) as i32);
    let keep: Vec<bool> = bins.iter().map(|b| b[0] > 0 && wanted(mean(b))).collect();
    // A bin with the kept bins around it (one colour straddles bin edges).
    let around = |i: usize| {
        let (r, g, b) = ((i >> 8) as i32, ((i >> 4) & 15) as i32, (i & 15) as i32);
        (-1..=1).flat_map(move |dr| (-1..=1).flat_map(move |dg| (-1..=1).map(move |db| (r + dr, g + dg, b + db))))
            .filter(|&(r, g, b)| (0..16).contains(&r) && (0..16).contains(&g) && (0..16).contains(&b))
            .map(|(r, g, b)| (r << 8 | g << 4 | b) as usize)
    };
    let total = |i: usize| around(i).filter(|&j| keep[j]).fold([0u32; 4], |a, j| [a[0] + bins[j][0], a[1] + bins[j][1], a[2] + bins[j][2], a[3] + bins[j][3]]);
    let best = (0..4096).filter(|&i| keep[i]).max_by_key(|&i| total(i)[0])?;
    Some(mean(&total(best)))
}

/// Cells per side of the seam model, about the scale of the player's blur (its sigma is about 2.4 % of
/// the cover). Each cell is the mean of its pixels, as the blur shows it: a thin line is gone, a fine
/// texture is its average (the Grinch EP's green and black speckle is dark green).
const GRID: usize = 32;
/// Width over height of the player's sleeve: the square cover is scaled up and cropped at both sides.
const PLAYER_SLEEVE: f32 = 0.74;
/// The most colours tried as the page, and the share of the lower half and edges each must cover: a
/// cell straddling two colours is a mix, not a colour of the cover.
const CANDIDATES: usize = 8;
const CANDIDATE_SHARE: f32 = 0.06;
/// The band the artwork visibly dissolves in, as fractions of its height: from where the blurred copy is
/// half in to where the player has nearly rubbed the sleeve out (`sleeve::RUB_OUT`) and the album's
/// dissolve is nearly opaque (`sleeve::HERO_STOPS`).
const SEAM_FROM: f32 = 1.0 - MELT * 1.6;
const SEEN_TO: f32 = 1.0 - MELT * 0.55;

/// What the page meets. A frame round the whole cover thicker than the blur is the page (Syro's white, In
/// Utero's cream). Otherwise each main colour of the cover's lower half is tried as the page, the
/// player's sleeve and the album's dissolve are drawn over it as the app draws them, and the page with the
/// softest seam wins: the least change, steep steps counting most, from the artwork down into the page.
/// A solid band under a picture is kept even when the fade has nearly hidden it (Demon Days' navy foot),
/// and a line hidden wholly under the fade counts for nothing.
struct Seam {
    frame: Option<u32>,
    colour: u32,
    /// Mean of the band the artwork dissolves in, where the fade starts from.
    mean: u32,
}

impl Seam {
    fn of(pixels: &[u32], w: usize, h: usize, ok: &Oklab) -> Seam {
        let n = GRID;
        let span = |c: usize, s: usize| c * s / n..((c + 1) * s / n).max(c * s / n + 1);
        let cell = |x: usize, y: usize| mean_rgb(span(y, h).flat_map(|py| pixels[py * w..(py + 1) * w][span(x, w)].iter().copied()));
        let sharp: Vec<u32> = (0..n * n).map(|i| cell(i % n, i / n).expect("every cell holds a pixel")).collect();
        let ring: Vec<u32> = (0..n).flat_map(|i| [sharp[i], sharp[(n - 1) * n + i], sharp[i * n], sharp[i * n + n - 1]]).collect();
        let lower = &sharp[n / 2 * n..];
        let band = (SEAM_FROM * n as f32) as usize * n..(SEEN_TO * n as f32) as usize * n;
        let mean = mean_rgb(sharp[band].iter().copied()).expect("the band has rows");
        let frame = one_colour(&ring);
        let mut left: Vec<(u32, f32)> = lower.iter().chain(&ring).map(|&c| (c, 1.0)).collect();
        let least = left.len() as f32 * CANDIDATE_SHARE;
        let mut candidates = Vec::with_capacity(CANDIDATES);
        while !left.is_empty() && candidates.len() < CANDIDATES {
            let c = heaviest(&left);
            let before = left.len();
            left.retain(|p| !near(p.0, c));
            if candidates.is_empty() || (before - left.len()) as f32 >= least {
                candidates.push(c);
            }
        }
        let blurred = soften(&sharp);
        let colour = candidates
            .into_iter()
            .map(|c| (seam_cost(&sharp, &blurred, c, ok), c))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, c)| c)
            .expect("a candidate");
        Seam { frame, colour, mean }
    }
}

/// The [`GRID`] as the player's blurred copy shows it: two passes of a [1 2 1] filter.
fn soften(px: &[u32]) -> Vec<u32> {
    let n = GRID;
    let mut cur = px.to_vec();
    for across in [true, false, true, false] {
        cur = (0..n * n)
            .map(|i| {
                let (x, y) = (i % n, i / n);
                let at = |d: isize| {
                    let (x, y) = if across { ((x as isize + d).clamp(0, n as isize - 1) as usize, y) } else { (x, (y as isize + d).clamp(0, n as isize - 1) as usize) };
                    cur[y * n + x]
                };
                let ch = |f: fn(u32) -> i32| (f(at(-1)) + 2 * f(at(0)) + f(at(1)) + 2) / 4;
                rgb(ch(red), ch(green), ch(blue))
            })
            .collect();
    }
    cur
}

/// How hard the seam is with `page` under the artwork: down each column of the lower half, the sum of
/// squared Oklab steps through what the player and the album page draw, ending on the page.
fn seam_cost(sharp: &[u32], blurred: &[u32], page: u32, ok: &Oklab) -> f32 {
    use crate::sleeve::{alpha_at, HERO_ALPHAS, HERO_STOPS, RUB_OUT, SOFT, SOFT_FROM, SOFT_TO, Stop};
    let n = GRID;
    let hero: [Stop; 4] = std::array::from_fn(|i| Stop { at: HERO_STOPS[i], alpha: HERO_ALPHAS[i] });
    let crop = (1.0 - PLAYER_SLEEVE) / 2.0;
    let page_lab = ok.lab(page);
    let step = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
    let mut cost = 0.0;
    for x in 0..n {
        let centre = (x as f32 + 0.5) / n as f32;
        let in_player = (crop..1.0 - crop).contains(&centre);
        let (mut player, mut album) = (None::<[f32; 3]>, None::<[f32; 3]>);
        for y in n / 2..n {
            let t = (y as f32 + 0.5) / n as f32;
            let i = y * n + x;
            if in_player {
                let soft = alpha_at(&SOFT, (t - SOFT_FROM) / (SOFT_TO - SOFT_FROM));
                let shown = 1.0 - alpha_at(&RUB_OUT, (t - (1.0 - MELT)) / MELT);
                let lab = ok.lab(blend_argb(page, blend_argb(sharp[i], blurred[i], soft), shown));
                cost += player.map_or(0.0, |p| step(p, lab));
                player = Some(lab);
            }
            let lab = ok.lab(blend_argb(sharp[i], page, alpha_at(&hero, t)));
            cost += album.map_or(0.0, |p| step(p, lab));
            album = Some(lab);
        }
        cost += player.map_or(0.0, |p| step(p, page_lab)) + album.map_or(0.0, |p| step(p, page_lab));
    }
    cost
}

/// The colour with the most weight near it, as the weighted mean of those near it.
fn heaviest(px: &[(u32, f32)]) -> u32 {
    let near_weight = |c: u32| px.iter().filter(|p| near(p.0, c)).map(|p| p.1).sum::<f32>();
    let best = px.iter().map(|p| (near_weight(p.0), p.0)).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, c)| c).expect("at least one colour");
    let (mut r, mut g, mut b, mut total) = (0f32, 0f32, 0f32, 0f32);
    for &(c, wt) in px.iter().filter(|p| near(p.0, best)) {
        (r, g, b, total) = (r + red(c) as f32 * wt, g + green(c) as f32 * wt, b + blue(c) as f32 * wt, total + wt);
    }
    rgb((r / total).round() as i32, (g / total).round() as i32, (b / total).round() as i32)
}

/// The colour 85 % of `px` share, as the mean of those pixels; None for a mix.
fn one_colour(px: &[u32]) -> Option<u32> {
    let mean = mean_rgb(px.iter().copied())?;
    let share = px.iter().filter(|&&p| near(p, mean)).count() as f32 / px.len() as f32;
    (share >= 0.85).then(|| mean_rgb(px.iter().copied().filter(|&p| near(p, mean))).unwrap_or(mean))
}

/// Whether two colours look alike (within 48 levels).
fn near(a: u32, b: u32) -> bool {
    let (dr, dg, db) = (red(a) - red(b), green(a) - green(b), blue(a) - blue(b));
    dr * dr + dg * dg + db * db < 48 * 48
}

/// The mean colour of `px`, each channel's sum divided down; None for no pixels.
fn mean_rgb(px: impl Iterator<Item = u32>) -> Option<u32> {
    let (mut r, mut g, mut b, mut n) = (0i64, 0i64, 0i64, 0i64);
    for p in px {
        (r, g, b, n) = (r + red(p) as i64, g + green(p) as i64, b + blue(p) as i64, n + 1);
    }
    (n > 0).then(|| rgb((r / n) as i32, (g / n) as i32, (b / n) as i32))
}

/// Contrast for glyphs and controls.
const GLYPH_CONTRAST: f64 = 3.2;

/// Steps `color`'s lightness towards `fallback`, the page's text colour, until it reads as text on the
/// page, else `fallback`.
fn readable(color: u32, background: u32, fallback: u32, ok: &Oklab) -> u32 {
    stepped(color, background, luminance(fallback) > luminance(background), TEXT_CONTRAST, ok).unwrap_or(fallback)
}

/// Like [`readable`] for a glyph, also trying the opposite direction before `fallback`, for mid-tone
/// surfaces.
pub fn readable_either_way(color: u32, background: u32, fallback: u32) -> u32 {
    either_way(color, background, fallback, GLYPH_CONTRAST)
}

/// [`readable_either_way`] for text: until it reads at 4.5:1.
pub fn text_either_way(color: u32, background: u32, fallback: u32) -> u32 {
    either_way(color, background, fallback, TEXT_CONTRAST)
}

fn either_way(color: u32, background: u32, fallback: u32, contrast: f64) -> u32 {
    let ok = Oklab::new();
    let light = luminance(background) < DARK_PAGE;
    let step = |towards_light| stepped(color, background, towards_light, contrast, &ok);
    step(light).or_else(|| step(!light)).unwrap_or(fallback)
}

/// Below this luminance a page is dark: what is drawn on it is stepped lighter first.
pub(crate) const DARK_PAGE: f32 = 0.4;

/// Each step also raises saturation, but never the chroma above `color`'s: HSL calls a near-black
/// 020305 43 % saturated, and lightening it made a vivid blue.
fn stepped(color: u32, background: u32, towards_light: bool, contrast: f64, ok: &Oklab) -> Option<u32> {
    if calculate_contrast(color, background) >= contrast {
        return Some(color);
    }
    let chroma = ok.lightness_chroma(color).1;
    let mut hsl = color_to_hsl(color);
    for _ in 1..=8 {
        hsl[2] = if towards_light { (hsl[2] + 0.07).min(0.92) } else { (hsl[2] - 0.07).max(0.15) };
        hsl[1] = (hsl[1] * 1.05).min(1.0);
        let mut candidate = hsl_to_color(hsl);
        let reached = ok.lightness_chroma(candidate).1;
        if reached > chroma {
            candidate = hsl_to_color([hsl[0], hsl[1] * chroma / reached, hsl[2]]);
        }
        if calculate_contrast(candidate, background) >= contrast {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_text() {
        let ok = Oklab::new();
        let (white, black, fallback) = (0xFFFF_FFFF, 0xFF12_1212, 0xFF67_50A4);
        assert_eq!(readable(0xFF1E_5AA0, white, fallback, &ok), 0xFF1E_5AA0);
        let on_white = readable(0xFFFF_E08A, white, fallback, &ok);
        let on_black = readable(0xFF20_1060, black, fallback, &ok);
        assert!(calculate_contrast(on_white, white) >= 4.5 && calculate_contrast(on_black, black) >= 4.5);
        assert!(on_white != fallback && on_black != fallback);

        // A near-black stays nearly grey as it lightens (HSL calls 020305 43 % saturated).
        let lifted = readable(0xFF02_0305, black, fallback, &ok);
        assert!(calculate_contrast(lifted, black) >= 4.5 && ok.lightness_chroma(lifted).1 < 0.012, "{lifted:08x}");

        // Readable either way tries darker.
        // Pale blue accent on a mid-grey bar.
        let (accent, bar, ink) = (0xFFA0_B0C8, 0xFF8A_8878, 0xFFFF_FFFF);
        assert_eq!(readable(accent, bar, ink, &ok), ink);
        let got = readable_either_way(accent, bar, ink);
        assert!(got != ink && calculate_contrast(got, bar) >= 3.2 && luminance(got) < luminance(accent));
        let h = |c: u32| color_to_hsl(c)[0];
        assert!((h(got) - h(accent)).abs() < 12.0, "hue kept");
        assert_eq!(readable_either_way(0xFF1E_5AA0, 0xFFFF_FFFF, ink), 0xFF1E_5AA0);

        // Text contrast on page.
        for &c0 in &[0xFFE0_2020u32, 0xFF20_E020, 0xFF20_20E0, 0xFFE0_E020, 0xFF80_8080, 0xFF77_7777, 0xFF30_1060] {
            let c = derive(&solid(c0), S, S, false);
            assert!(calculate_contrast(c.on, c.background) >= 4.5, "{c0:08x}: {:08x} on {:08x}", c.on, c.background);
            assert!(calculate_contrast(c.accent, c.background) >= 4.5, "{c0:08x}: accent {:08x} on {:08x}", c.accent, c.background);
        }
    }

    const S: usize = 160;

    fn solid(c: u32) -> Vec<u32> {
        vec![c; S * S]
    }

    /// `top` colour with the last `rows` rows `bottom`.
    fn footed(top: u32, bottom: u32, rows: usize) -> Vec<u32> {
        let mut v = solid(top);
        v[(S - rows) * S..].fill(bottom);
        v
    }

    #[test]
    fn plain_covers() {
        // Black and white covers keep their own page.
        let c = derive(&solid(0xFF08_0808), S, S, false);
        assert!(luminance(c.background) < 0.01, "{:08x}", c.background);
        assert_eq!(c.on, WHITE);
        let c = derive(&solid(0xFFF6_F6F4), S, S, false);
        assert!(luminance(c.background) > 0.8, "{:08x}", c.background);
        assert_eq!(c.on, INK);

        // Cream and yellow keep their hue.
        let cream = derive(&solid(0xFFEF_E4C8), S, S, false);
        let [h, s, _] = color_to_hsl(cream.background);
        assert!((35.0..55.0).contains(&h) && s > 0.2, "cream bleached to {:08x}", cream.background);
        let yellow = derive(&solid(0xFFF2_D544), S, S, false);
        let [h, s, _] = color_to_hsl(yellow.background);
        assert!((40.0..60.0).contains(&h) && s > 0.3, "yellow bleached to {:08x}", yellow.background);

        // Amoled is black with no wash.
        let c = derive(&solid(0xFF30_60A0), S, S, true);
        assert_eq!(c.background, BLACK);
        assert!(c.wash.is_none());
        assert_eq!(c.wash_edge, c.edge);
    }

    #[test]
    fn under_white_darkens_only_light_pages() {
        // A paper sleeve: its light page and wash go dark enough for white text, in its own hue.
        let paper = derive(&solid(0xFFE8_EEF4), S, S, false);
        assert_ne!(paper.on, WHITE, "a paper sleeve keeps a light page");
        let c = under_white(&paper);
        assert_eq!(c.on, WHITE);
        assert!(luminance(c.background) <= PAGE_MAX_LUMA + 0.005, "{:08x}", c.background);
        let wash = c.wash.as_ref().expect("a wash");
        assert!(wash.iter().all(|&p| calculate_contrast(WHITE, p) >= 4.5), "white text reads on all of the wash");
        assert!((color_to_hsl(c.background)[0] - color_to_hsl(paper.background)[0]).abs() < 10.0, "its hue kept");

        // A dark page is left as it is.
        let dark = derive(&solid(0xFF20_3060), S, S, false);
        assert_eq!(under_white(&dark), dark);
    }

    /// `base` plus ±2 levels of noise per channel, offset by `cast`.
    fn noisy(base: u32, cast: [i32; 3], noise: &mut JavaRandom) -> u32 {
        let mut ch = |v: i32, c: i32| (v + c + (noise.next_float() * 5.0) as i32 - 2).clamp(0, 255);
        rgb(ch(red(base), cast[0]), ch(green(base), cast[1]), ch(blue(base), cast[2]))
    }

    #[test]
    fn white_paper_covers() {
        // Regression: noisy white paper with a bluish CD voted the page blue.
        let mut noise = JavaRandom::new(7);
        let c = S as f32 / 2.0;
        let v: Vec<u32> = (0..S * S)
            .map(|i| {
                let (x, y) = ((i % S) as f32 + 0.5 - c, (i / S) as f32 + 0.5 - c);
                let r = (x * x + y * y).sqrt() / S as f32;
                if r < 0.3 && r > 0.03 {
                    let sheen = (40.0 * (0.5 + 0.5 * (2.0 * (y.atan2(x) - 0.7)).cos())) as i32;
                    noisy(rgb(176 + sheen, 180 + sheen, 190 + sheen), [0, 0, 0], &mut noise)
                } else {
                    noisy(0xFFF6_F7FA, [-1, 0, 2], &mut noise)
                }
            })
            .collect();
        let p = derive(&v, S, S, false);
        assert!(Oklab::new().lightness_chroma(p.background).1 < 0.005 && luminance(p.background) > 0.7, "{:08x}", p.background);

        // Off-white paper inside a band of splashes: the page is one of their colours, never their average
        // (Cage the Elephant: they averaged into olive).
        let mut noise = JavaRandom::new(11);
        let v: Vec<u32> = (0..S * S)
            .map(|i| {
                let (x, y) = (i % S, i / S);
                let edge = x.min(y).min(S - 1 - x).min(S - 1 - y);
                match (edge < S / 12, (x / 8 + y / 8) % 3) {
                    (true, 0) => noisy(0xFFE8_D850, [0, 0, 0], &mut noise),
                    (true, 1) => noisy(0xFFE0_C890, [0, 0, 0], &mut noise),
                    (true, _) => noisy(0xFFD0_3048, [0, 0, 0], &mut noise),
                    _ => noisy(0xFFEE_EEE8, [0, 0, 0], &mut noise),
                }
            })
            .collect();
        let p = derive(&v, S, S, false);
        assert!([0xFFE8_D850, 0xFFE0_C890, 0xFFD0_3048, 0xFFEE_EEE8].iter().any(|&c| near(p.background, c)), "{:08x}", p.background);

        // Off-white paper, a red figure over a third of it, a dark coat below: the page is one of them,
        // never a mix of the figure's edge.
        let mut noise = JavaRandom::new(3);
        let v: Vec<u32> = (0..S * S)
            .map(|i| {
                let (x, y) = (i % S, i / S);
                if x > S / 4 && y > S / 5 && x < S * 5 / 6 {
                    let c = if y > S * 4 / 5 { 0xFF1C_1D27 } else { 0xFFD2_4850 };
                    noisy(c, [0, 0, 0], &mut noise)
                } else {
                    noisy(0xFFF7_F8F1, [0, 0, 0], &mut noise)
                }
            })
            .collect();
        let p = derive(&v, S, S, false);
        assert!([0xFFF7_F8F1, 0xFFD2_4850, 0xFF1C_1D27].iter().any(|&c| near(p.background, c)), "{:08x}", p.background);
    }

    #[test]
    fn dark_fields() {
        // A thin black line along the bottom is rubbed out and blurred away: the red above it is the page.
        let mut v = solid(0xFFB0_1818);
        v[(S - 3) * S..].fill(0xFF05_0505);
        let c = derive(&v, S, S, false);
        let [h, s, _] = color_to_hsl(c.background);
        assert!((h < 15.0 || h > 345.0) && s > 0.3, "not red: {:08x}", c.background);
        // A black frame thicker than the blur is the page, and the fade starts from it (Amnesiac).
        let edge = S / 12;
        let v: Vec<u32> = (0..S * S)
            .map(|i| if (i % S).min(i / S).min(S - 1 - i % S).min(S - 1 - i / S) < edge { 0xFF05_0505 } else { 0xFFB0_1818 })
            .collect();
        let c = derive(&v, S, S, false);
        assert!(luminance(c.background) < 0.01 && c.edge == c.background, "{:08x} from {:08x}", c.background, c.edge);

        // A dark subject in a coloured field: the field reaching the sides is the page (Westside Whimsy).
        let v: Vec<u32> = (0..S * S).map(|i| if (S / 4..S * 3 / 4).contains(&(i % S)) && i / S > S / 3 { 0xFF06_0606 } else { 0xFF34_9CAA }).collect();
        let c = derive(&v, S, S, false);
        assert!(near(c.background, 0xFF34_9CAA), "{:08x}", c.background);

        // A solid dark foot is continued under a light picture.
        let c = derive(&footed(0xFFD8_D8D0, 0xFF06_0606, 40), S, S, false);
        assert!(luminance(c.background) < 0.01, "{:08x}", c.background);

        // And a solid light foot under a dark one (In Utero, Dreamland).
        let c = derive(&footed(0xFF20_1810, 0xFFEE_E8C8, 56), S, S, false);
        assert!(luminance(c.background) > 0.7 && color_to_hsl(c.background)[1] > 0.3, "{:08x}", c.background);

        // The fade starts from what shows last, not from the hidden bottom rows.
        let c = derive(&footed(0xFF30_60A0, 0xFF05_0505, 6), S, S, false);
        assert!(near(c.edge, 0xFF30_60A0) && near(c.background, 0xFF30_60A0), "{:08x} from {:08x}", c.background, c.edge);
    }

    /// Hardcore Software's layout: lavender paper under a grey grid, thin blue swooshes,
    /// two black bars.
    fn tinted_paper() -> Vec<u32> {
        let mut noise = JavaRandom::new(5);
        (0..S * S)
            .map(|i| {
                let (x, y) = ((i % S) as i32, (i / S) as i32);
                let swoosh = (x - y * 2 + 40).rem_euclid(48) < 4 || (x + y).rem_euclid(64) < 2;
                let c = if (y == 20 || y == 120) && x > 20 && x < 140 {
                    0xFF10_1010
                } else if swoosh {
                    0xFF20_30E0
                } else if x % 16 == 0 || y % 16 == 0 {
                    0xFFB8_BAC8
                } else {
                    0xFFE0_E2F4
                };
                noisy(c, [0, 0, 0], &mut noise)
            })
            .collect()
    }

    #[test]
    fn pale_tinted_paper_is_paper() {
        let c = derive(&tinted_paper(), S, S, false);
        assert!(luminance(c.background) > 0.55, "{:08x}", c.background);
        assert_eq!(c.on, INK);
    }

    #[test]
    fn page_is_the_covers_own_colour() {
        // Light, mid and dark colours stay as they are; only the text colour follows them.
        for (colour, on) in [(0xFFF3_BBD7u32, INK), (0xFF86_7FC2, INK), (0xFFA4_4C2B, WHITE), (0xFFE3_001B, WHITE), (0xFF40_B5D9, INK)] {
            let c = derive(&solid(colour), S, S, false);
            assert_eq!((c.background, c.on), (colour, on), "{colour:08x}");
        }
        // Where neither text colour reads, the least lightness step that lets one.
        let grey = 0xFF77_7777;
        let c = derive(&solid(grey), S, S, false);
        assert!(calculate_contrast(c.on, c.background) >= 4.5, "{:08x}", c.background);
        assert!((color_to_hsl(c.background)[2] - color_to_hsl(grey)[2]).abs() < 0.02, "{:08x}", c.background);
    }

    /// Vertical stripes of `a` and `b`, `width` pixels each.
    fn stripes(a: u32, b: u32, width: usize) -> Vec<u32> {
        (0..S * S).map(|i| if (i % S / width).is_multiple_of(2) { a } else { b }).collect()
    }

    #[test]
    fn stripes_give_what_the_blur_shows() {
        let (red, blue) = (0xFFD0_2020u32, 0xFF20_30D0);
        // Finer than the blur, their mix; as wide as a quarter of the cover, one of them.
        for width in [1, 3] {
            let c = derive(&stripes(red, blue, width), S, S, false);
            assert!(near(c.background, blend_argb(red, blue, 0.5)), "{width} px: {:08x}", c.background);
        }
        let c = derive(&stripes(red, blue, S / 4), S, S, false);
        assert!(near(c.background, red) || near(c.background, blue), "{:08x}", c.background);
        // A figure that never reaches the striped foot is not the page (Melophobia).
        let mut v = stripes(0xFF10_1010, 0xFFF0_F0F0, 6);
        for y in S / 6..S * 3 / 5 {
            for x in S / 4..S * 3 / 4 {
                v[y * S + x] = 0xFFE0_7A60;
            }
        }
        let c = derive(&v, S, S, false);
        assert!(Oklab::new().lightness_chroma(c.background).1 < GREY, "{:08x}", c.background);
    }

    /// `around` with a disc of `inside` covering most of it (Syro).
    fn disc_in(around: u32, inside: u32) -> Vec<u32> {
        let c = S as f32 / 2.0;
        (0..S * S)
            .map(|i| {
                let (x, y) = ((i % S) as f32 + 0.5 - c, (i / S) as f32 + 0.5 - c);
                if x * x + y * y < (S as f32 * 0.46).powi(2) { inside } else { around }
            })
            .collect()
    }

    #[test]
    fn a_border_of_one_colour_is_the_page() {
        let c = derive(&disc_in(WHITE, 0xFFBA_D033), S, S, false);
        assert!(luminance(c.background) > 0.9, "{:08x}", c.background);
        let c = derive(&disc_in(0xFFBA_D033, WHITE), S, S, false);
        assert!(near(c.background, 0xFFBA_D033), "{:08x}", c.background);
    }

    #[test]
    fn dark_page_is_no_more_colourful_than_its_cover() {
        let ok = Oklab::new();
        // Lover's pink and a pale lavender; a saturated red keeps its depth.
        for colour in [0xFFF3_BBD7u32, 0xFFC7_B0F9, 0xFFB0_1818] {
            let page = dark_page(color_to_hsl(colour), ok.lightness_chroma(colour).1, &ok);
            let (got, cover) = (ok.lightness_chroma(page).1, ok.lightness_chroma(colour).1);
            assert!(got <= cover + 0.005 && got > cover * 0.5, "{colour:08x} made {page:08x}: chroma {got:.3}, cover {cover:.3}");
            let hue = |c: u32| color_to_hsl(c)[0];
            assert!((hue(page) - hue(colour)).abs() < 15.0, "{colour:08x} made {page:08x}");
            assert!(calculate_contrast(WHITE, page) >= 8.0, "{page:08x}");
        }
    }

    /// Black with an orange disc of `radius` pixels in the middle.
    fn black_with_disc(radius: f32) -> Vec<u32> {
        let c = S as f32 / 2.0;
        (0..S * S)
            .map(|i| {
                let (x, y) = ((i % S) as f32 + 0.5 - c, (i / S) as f32 + 0.5 - c);
                if x * x + y * y < radius * radius { 0xFFE0_7810 } else { 0xFF08_0808 }
            })
            .collect()
    }

    #[test]
    fn black_sleeve_accent_is_its_vivid_mark() {
        let ok = Oklab::new();
        // A 22 px disc covers 6 %: the accent is its orange.
        let c = derive(&black_with_disc(22.0), S, S, false);
        let [h, _, _] = color_to_hsl(c.accent);
        assert!((20.0..45.0).contains(&h) && ok.lightness_chroma(c.accent).1 >= VIVID, "{:08x}", c.accent);
        // A 2 px speck (0.05 %) is not the cover's colour: the accent stays grey.
        let c = derive(&black_with_disc(2.0), S, S, false);
        assert!(ok.lightness_chroma(c.accent).1 < GREY, "{:08x}", c.accent);
    }

    /// `n` x `n` cover: a blue disc on white paper under a grey grid of lines `n / 160` pixels wide.
    fn gridded(n: usize) -> Vec<u32> {
        let unit = n / 160;
        (0..n * n)
            .map(|i| {
                let (x, y) = (i % n / unit, i / n / unit);
                let (dx, dy) = (x as f32 - 100.0, y as f32 - 70.0);
                if x % 8 == 0 || y % 8 == 0 {
                    0xFF80_8080
                } else if dx * dx + dy * dy < 50.0 * 50.0 {
                    0xFF20_30C0
                } else {
                    WHITE
                }
            })
            .collect()
    }

    /// Mean of each `k` x `k` block, as a server's smaller copy of a cover.
    fn shrunk(px: &[u32], n: usize, k: usize) -> Vec<u32> {
        let m = n / k;
        (0..m * m)
            .map(|i| {
                let (bx, by) = (i % m * k, i / m * k);
                mean_rgb((by..by + k).flat_map(|y| px[y * n + bx..y * n + bx + k].iter().copied())).expect("k > 0")
            })
            .collect()
    }

    #[test]
    fn same_page_at_every_size() {
        let big = gridded(S * 6);
        let small = shrunk(&big, S * 6, 6);
        let (a, b) = (derive(&big, S * 6, S * 6, false), derive(&small, S, S, false));
        assert!(calculate_contrast(a.background, b.background) < 1.05, "{:08x} vs {:08x}", a.background, b.background);
        assert_eq!(a.on, b.on);
    }

    #[test]
    fn wash_size_and_determinism() {
        let v: Vec<u32> = (0..S * S).map(|i| rgb((i % S) as i32, (i / S) as i32, 90)).collect();
        let a = derive(&v, S, S, false);
        let b = derive(&v, S, S, false);
        assert_eq!(a.wash.as_ref().map(Vec::len), Some(WASH_OUT * WASH_OUT));
        assert_eq!(a, b);
    }
}
