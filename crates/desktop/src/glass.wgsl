// The compositor's shaders: a quad placed in pixels, a plain copy (the page), a premultiplied copy (the
// pills' layers), a blur pyramid of the page (each level half the last, made with a
// 13-tap downsample so nothing is skipped), the pills' glass and the edge fades, and the lyrics' focus.
// The pyramid is premultiplied by how much of it is the page: what is raised over the page (a menu, a dialog's
// pane, while the lyrics are in focus) is left out of it, and a blur is the page around it alone.
//
// Blur levels: level 0 is the page itself, level k the pyramid's mip k - 1 (1/2^k of the page). A level is
// read with a cubic B-spline over four bilinear taps, so a small level drawn large stays smooth rather than
// blocky; a fractional level mixes the two levels beside it, never the sharp page with a far blur.

struct U {
    rect: vec4<f32>,      // where the quad goes, target pixels (x, y, w, h)
    view: vec4<f32>,      // target size (w, h), source size (w, h)
    shape: vec4<f32>,     // pill: corner radius (pixels)
    face: vec4<f32>,      // pill: blur level in w; fade: the page's colour in rgb
    light: vec4<f32>,     // fade: where its ramp meets the pill, edge at the top; focus: band top, height, pixels per blur step, scale
    gather: vec4<f32>,    // pyramid levels in z
    raised: array<vec4<f32>, 4>, // focus and the pyramid's first level: rounded rects over the page (x, y, w, h), target pixels; none where w is 0
    radii: vec4<f32>,     // their corner radii
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
@group(0) @binding(3) var pyramid: texture_2d<f32>;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let px = u.rect.xy + corner * u.rect.zw;
    var o: V;
    o.pos = vec4<f32>(px.x / u.view.x * 2.0 - 1.0, 1.0 - px.y / u.view.y * 2.0, 0.0, 1.0);
    o.uv = corner;
    return o;
}

@fragment
fn fs_copy(v: V) -> @location(0) vec4<f32> {
    return textureSampleLevel(src, smp, v.uv, 0.0);
}

// The native window is opaque; layer copies keep their own alpha for blending.
@fragment
fn fs_opaque(v: V) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, smp, v.uv, 0.0);
    return vec4<f32>(c.rgb, 1.0);
}

// One pyramid level from the one above it: thirteen taps (Jimenez), every source texel counted. From the page,
// a tap under a raised shape counts for nothing.
@fragment
fn fs_down(v: V) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let t = 1.0 / size;
    let s = v.uv;
    let a = tap(s + t * vec2<f32>(-2.0, -2.0), size);
    let b = tap(s + t * vec2<f32>(0.0, -2.0), size);
    let c = tap(s + t * vec2<f32>(2.0, -2.0), size);
    let d = tap(s + t * vec2<f32>(-2.0, 0.0), size);
    let e = tap(s, size);
    let f = tap(s + t * vec2<f32>(2.0, 0.0), size);
    let g = tap(s + t * vec2<f32>(-2.0, 2.0), size);
    let h = tap(s + t * vec2<f32>(0.0, 2.0), size);
    let i = tap(s + t * vec2<f32>(2.0, 2.0), size);
    let j = tap(s + t * vec2<f32>(-1.0, -1.0), size);
    let k = tap(s + t * vec2<f32>(1.0, -1.0), size);
    let l = tap(s + t * vec2<f32>(-1.0, 1.0), size);
    let m = tap(s + t * vec2<f32>(1.0, 1.0), size);
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

// A tap: the four texels around `at` (a texel corner), less what of them raised shapes cover. Only near a shape's
// edge are they read one by one.
fn tap(at: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let away = raised_distance(at * size);
    if (away > 1.5) {
        return textureSampleLevel(src, smp, at, 0.0);
    }
    if (away < -1.5) {
        return vec4<f32>(0.0);
    }
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let texel = at * size + vec2<f32>(f32(i & 1) - 0.5, f32(i >> 1u) - 0.5);
        sum += textureSampleLevel(src, smp, texel / size, 0.0) * (1.0 - raised_cover(texel));
    }
    return sum * 0.25;
}

// Mip `lod` of the pyramid at `at`, through a cubic B-spline: four bilinear taps.
fn smooth_mip(at: vec2<f32>, lod: i32) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(pyramid, lod));
    let p = at * size - 0.5;
    let i = floor(p);
    let f = p - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 0.5 + w1 / g0) / size;
    let h1 = (i + 1.5 + w3 / g1) / size;
    let l = f32(lod);
    return (textureSampleLevel(pyramid, smp, vec2<f32>(h0.x, h0.y), l) * g0.x + textureSampleLevel(pyramid, smp, vec2<f32>(h1.x, h0.y), l) * g1.x) * g0.y
        + (textureSampleLevel(pyramid, smp, vec2<f32>(h0.x, h1.y), l) * g0.x + textureSampleLevel(pyramid, smp, vec2<f32>(h1.x, h1.y), l) * g1.x) * g1.y;
}

// The page at `at` (0..1) blurred to `level`: 0 sharp, each level about twice the blur of the one before. Opaque:
// divided by how much of the page went into it.
fn blurred(at: vec2<f32>, level: f32) -> vec4<f32> {
    let c = gathered(at, level);
    return vec4<f32>(c.rgb / max(c.a, 0.0001), 1.0);
}

fn gathered(at: vec2<f32>, level: f32) -> vec4<f32> {
    let top = u.gather.z;
    let l = clamp(level, 0.0, top);
    if (l < 1.0) {
        return mix(textureSampleLevel(src, smp, at, 0.0), smooth_mip(at, 0), l);
    }
    let k = min(floor(l), top - 1.0);
    let below = smooth_mip(at, i32(k) - 1);
    if (k >= top - 1.0 || l == k) {
        return below;
    }
    return mix(below, smooth_mip(at, i32(k)), l - k);
}

// Half a step of 8-bit colour of noise, so a smooth gradient does not band.
fn dither(p: vec2<f32>) -> vec3<f32> {
    let n = fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    return vec3<f32>((n - 0.5) / 255.0);
}

fn sd_round_rect(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// How far pixel `p` is outside the nearest raised shape (negative inside it), and far when there is none.
fn raised_distance(p: vec2<f32>) -> f32 {
    var away = 1e9;
    for (var i = 0; i < 4; i = i + 1) {
        let r = u.raised[i];
        if (r.z > 0.0) {
            let half = r.zw * 0.5;
            away = min(away, sd_round_rect(p - (r.xy + half), half, u.radii[i]));
        }
    }
    return away;
}

// How much of pixel `p` the raised shapes cover, 0 to 1.
fn raised_cover(p: vec2<f32>) -> f32 {
    return clamp(0.5 - raised_distance(p), 0.0, 1.0);
}

fn field(p: vec2<f32>) -> f32 {
    let half = u.rect.zw * 0.5;
    return sd_round_rect(p - (u.rect.xy + half), half, u.shape.x);
}

// A pill's glass: the page blurred to `face.w`, inside the rounded rect. The pill's own layer lays its
// tint and outline over it.
@fragment
fn fs_pill(v: V) -> @location(0) vec4<f32> {
    let p = u.rect.xy + v.uv * u.rect.zw;
    let cover = clamp(0.5 - field(p), 0.0, 1.0);
    if (cover <= 0.0) {
        discard;
    }
    let c = blurred(clamp(p / u.view.xy, vec2<f32>(0.0), vec2<f32>(1.0)), u.face.w).rgb;
    return vec4<f32>((c + dither(p)) * cover, cover);
}

// The page's colour over what runs out under a pill (Android's `edgeFade`): along the strip, 0 at its far
// end to 1 at the window's edge. Clear far off, still mostly clear where the pill begins (`light.x`), and
// thinning out to a trace left at the edge, so the pill's glass has something to show. `light.y` is 1 when
// the window's edge is at the top of the strip; `face.rgb` the page.
fn fade_alpha(t: f32, at: f32) -> f32 {
    let stops = array<f32, 6>(0.0, at, at + (1.0 - at) * 0.3, at + (1.0 - at) * 0.6, at + (1.0 - at) * 0.85, 1.0);
    let alphas = array<f32, 6>(0.0, 0.14, 0.38, 0.64, 0.84, 0.94);
    for (var i = 1; i < 6; i = i + 1) {
        if (t <= stops[i]) {
            return mix(alphas[i - 1], alphas[i], (t - stops[i - 1]) / max(stops[i] - stops[i - 1], 0.0001));
        }
    }
    return alphas[5];
}

@fragment
fn fs_fade(v: V) -> @location(0) vec4<f32> {
    let t = select(v.uv.y, 1.0 - v.uv.y, u.light.y > 0.5);
    let a = fade_alpha(t, u.light.x);
    return vec4<f32>(u.face.rgb * a, a);
}

// Focus: the lyrics beside the artwork in Now Playing. Within `rect`, the page sharp in a band around the line
// sung (`light.x` its top, `light.y` its height, pixels), each line further off blurred a step more (`light.z`
// pixels a step, as a line and its gap), up to five points' blur, as Music draws the lines not sung. What is
// raised over the page stays sharp.
@fragment
fn fs_focus(v: V) -> @location(0) vec4<f32> {
    let p = u.rect.xy + v.uv * u.rect.zw;
    let away = max(max(u.light.x - p.y, p.y - (u.light.x + u.light.y)), 0.0);
    // Points of blur: a point at the band's edge, ramping in over its first 16, then one more a line.
    let points = min(min(away / (16.0 * u.light.w), 1.0) + away / u.light.z, 5.0);
    // The level whose blur that is: level k blurs about 0.6 * 2^k pixels.
    let sigma = points * u.light.w;
    let level = select(0.0, log2(max(sigma, 0.6) / 0.6), sigma > 0.0);
    let at = p / u.view.xy;
    let soft = blurred(at, level).rgb + dither(p);
    let sharp = textureSampleLevel(src, smp, at, 0.0).rgb;
    return vec4<f32>(mix(soft, sharp, raised_cover(p)), 1.0);
}
