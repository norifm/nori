//! Sound-setting queries for the platform: the pre-amp in effect, graphic equalizer data and the
//! built-in curves.

pub use nori_player::dsp::*;
pub use nori_player::graphic;

/// The pre-amp in effect for the equalizer in use; 0 when it is off.
pub fn effective_preamp_db(s: &crate::settings::StoredPrefs) -> f32 {
    crate::settings::mode_preamp_db(s.eq_enabled, s.eq_preamp_db, s.eq_mode, &s.eq_graphic, &s.eq_bands)
}

/// The graphic sliders when the graphic equalizer is on; empty otherwise.
pub fn graphic_sliders(s: &crate::settings::StoredPrefs) -> Vec<f64> {
    if s.eq_enabled && s.eq_mode == crate::settings::EqMode::Graphic {
        s.eq_graphic.iter().map(|g| *g as f64).collect()
    } else {
        Vec::new()
    }
}

/// Where `freq` sits on a band's frequency scale: 20 Hz at 0 to 20 kHz at 1, logarithmic.
pub fn band_position(freq: f32) -> f32 {
    ((freq.max(1.0) as f64 / 20.0).log10() / 3.0) as f32
}

/// The frequency at `position` on a band's frequency scale ([`band_position`]), to two significant digits:
/// steps of 10 Hz from 100 Hz, 100 Hz from 1 kHz.
pub fn band_freq_at(position: f32) -> f32 {
    let f = 20.0 * 1000f64.powf(position.clamp(0.0, 1.0) as f64);
    let step = 10f64.powi(f.log10().floor() as i32 - 1);
    ((f / step).round() * step).clamp(20.0, 20_000.0) as f32
}

/// The graphic equalizer's layouts, by band count.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn graphic_layouts() -> Vec<u32> {
    nori_player::graphic::LAYOUTS.iter().map(|&n| n as u32).collect()
}

/// The graphic band centres for `count` sliders, low to high, with their ISO labels. Empty for an
/// invalid count.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn graphic_bands(count: u32) -> Vec<GraphicBand> {
    nori_player::graphic::centres(count as usize).into_iter().map(|f| GraphicBand { freq: f as f32, label_hz: nori_player::graphic::nominal(f) as f32 }).collect()
}

/// How closely the graphic sliders follow a correction, dB: RMS and maximum difference over 20 Hz-20 kHz,
/// overall level removed.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct GraphicFollow {
    pub rms_db: f32,
    pub max_db: f32,
}

/// [`GraphicFollow`] for `sliders` against `target` (`StoredPrefs::eq_graphic_target`); None without a
/// target.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn graphic_follow(sliders: Vec<f32>, target: Vec<f32>) -> Option<GraphicFollow> {
    if target.len() != nori_player::graphic::TARGET_POINTS {
        return None;
    }
    let s: Vec<f64> = sliders.iter().map(|v| *v as f64).collect();
    let t: Vec<f64> = target.iter().map(|v| *v as f64).collect();
    let (rms, max) = nori_player::graphic::follow(&s, &t);
    Some(GraphicFollow { rms_db: rms as f32, max_db: max as f32 })
}

/// One band of the graphic equalizer.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct GraphicBand {
    pub freq: f32,
    /// Its ISO 266 nominal frequency (31.5, 63, 125 ... 16000).
    pub label_hz: f32,
}

/// The graphic equalizer's response in dB at each of `freqs` (48 kHz), for drawing the curve.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn graphic_response(sliders: Vec<f32>, freqs: Vec<f32>) -> Vec<f32> {
    let g: Vec<f64> = sliders.iter().map(|v| *v as f64).collect();
    let bands = nori_player::graphic::design(48_000.0, &g);
    freqs.iter().map(|f| nori_player::graphic::response_db(48_000.0, &bands, *f as f64) as f32).collect()
}

/// What the parametric bands play, dB at each of `freqs` (48 kHz), for drawing the curve: `right` only when
/// a band is for one side; `each`, every band's own, in order.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BandsResponse {
    pub left: Vec<f32>,
    pub right: Option<Vec<f32>>,
    pub each: Vec<Vec<f32>>,
}

/// [`BandsResponse`] for `bands`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn bands_response(bands: Vec<crate::settings::SoundBand>, freqs: Vec<f32>) -> BandsResponse {
    use crate::settings::BandChannel;
    let curve = |heard: &[Band]| -> Vec<f32> { freqs.iter().map(|f| nori_player::graphic::response_db(48_000.0, heard, *f as f64) as f32).collect() };
    let side = |skip: BandChannel| curve(&bands.iter().filter(|b| b.channel != skip).map(Band::from).collect::<Vec<_>>());
    let split = bands.iter().any(|b| b.channel != BandChannel::Both);
    let each = bands.iter().map(|b| curve(&[Band::from(b)])).collect();
    BandsResponse { left: side(BandChannel::Right), right: split.then(|| side(BandChannel::Left)), each }
}

/// How far an equalizer's drawn curve reaches up and down, dB.
pub const CURVE_DB: f32 = 15.0;

/// What the equalizer in use plays, for drawing: `curve` in dB at `samples` frequencies spread evenly on
/// the band scale ([`band_position`], 20 Hz to 20 kHz; the left side where the sides differ), `each`
/// parametric band's own the same way, and each band's point on it.
#[derive(Debug, Clone, PartialEq)]
pub struct EqDrawing {
    pub curve: Vec<f32>,
    pub each: Vec<Vec<f32>>,
    pub points: Vec<EqPoint>,
}

/// A band's point: `x` on the band scale (0..1), `db` its height (0 for a kind without gain).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqPoint {
    pub x: f32,
    pub db: f32,
}

/// [`EqDrawing`] of `p`'s equalizer.
pub fn eq_drawing(p: &crate::settings::StoredPrefs, samples: u32) -> EqDrawing {
    let last = samples.saturating_sub(1).max(1) as f32;
    let freqs: Vec<f32> = (0..samples).map(|i| 20.0 * 1000f32.powf(i as f32 / last)).collect();
    if p.eq_mode == crate::settings::EqMode::Graphic {
        let centres = graphic_bands(p.eq_graphic.len() as u32);
        let points = p.eq_graphic.iter().zip(&centres).map(|(g, b)| EqPoint { x: band_position(b.freq), db: *g }).collect();
        EqDrawing { curve: graphic_response(p.eq_graphic.clone(), freqs), each: Vec::new(), points }
    } else {
        let points = p.eq_bands.iter().map(|b| EqPoint { x: band_position(b.freq), db: if b.kind.uses_gain() { b.gain_db } else { 0.0 } }).collect();
        let r = bands_response(p.eq_bands.clone(), freqs);
        EqDrawing { curve: r.left, each: r.each, points }
    }
}

/// A band at `freq` moved to `position` on the band scale: the frequency there, or `freq` itself when
/// that rounds the same, so a band moved only up or down keeps its exact frequency.
pub fn band_moved(freq: f32, position: f32) -> f32 {
    let to = band_freq_at(position);
    if to == band_freq_at(band_position(freq)) { freq } else { to }
}

/// The built-in equalizer curves.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn eq_presets() -> Vec<nori_model::NamedPreset> {
    nori_player::dsp::eq_presets()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BandChannel, SoundBand};
    use nori_model::EqKind;

    #[test]
    fn band_scale_rounds_to_two_digits() {
        for (position, freq) in [(0.0, 20.0), (1.0, 20_000.0), (-1.0, 20.0), (2.0, 20_000.0), (band_position(2109.0), 2100.0), (band_position(503.0), 500.0), (band_position(31.4), 31.0)] {
            assert_eq!(band_freq_at(position), freq, "at {position}");
        }
        assert!((band_position(632.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_drawing_puts_each_band_on_its_curve() {
        use crate::settings::{EqMode, StoredPrefs};
        let graphic = StoredPrefs { eq_mode: EqMode::Graphic, eq_graphic: vec![0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], ..StoredPrefs::default() };
        let d = eq_drawing(&graphic, 49);
        assert_eq!((d.curve.len(), d.each.len(), d.points.len()), (49, 0, 10));
        let at = |d: &EqDrawing, x: f32| d.curve[(x * 48.0).round() as usize];
        assert!((at(&d, d.points[1].x) - 6.0).abs() < 1.0, "the curve passes the raised slider: {}", at(&d, d.points[1].x));
        let cut = SoundBand { kind: EqKind::Peaking, freq: 1000.0, gain_db: -4.0, q: 1.0, channel: BandChannel::Both };
        let pass = SoundBand { kind: EqKind::HighPass, freq: 40.0, gain_db: 3.0, q: 0.7, channel: BandChannel::Both };
        let parametric = StoredPrefs { eq_mode: EqMode::Parametric, eq_bands: vec![cut, pass], ..StoredPrefs::default() };
        let d = eq_drawing(&parametric, 49);
        assert_eq!(d.each.len(), 2);
        assert_eq!(d.points[1].db, 0.0, "a high pass has no gain to show");
        assert!((at(&d, d.points[0].x) + 4.0).abs() < 1.0);
    }

    #[test]
    fn a_band_moved_up_keeps_its_frequency() {
        assert_eq!(band_moved(1234.0, band_position(1234.0)), 1234.0);
        assert_eq!(band_moved(1234.0, band_position(2000.0)), 2000.0);
    }

    #[test]
    fn bands_response_follows_each_side() {
        let bell = |channel| SoundBand { kind: EqKind::Peaking, freq: 1000.0, gain_db: 6.0, q: 1.0, channel };
        let at = |r: &[f32]| (r[0], r[1]);
        let both = bands_response(vec![bell(BandChannel::Both)], vec![100.0, 1000.0]);
        assert!((at(&both.left).1 - 6.0).abs() < 0.05 && at(&both.left).0 < 1.0, "{both:?}");
        assert_eq!(both.right, None, "both sides hear the same");
        let two = bands_response(vec![bell(BandChannel::Both), SoundBand { freq: 100.0, gain_db: -3.0, ..bell(BandChannel::Both) }], vec![100.0, 1000.0]);
        assert!((at(&two.each[0]).1 - 6.0).abs() < 0.05 && (at(&two.each[1]).0 + 3.0).abs() < 0.05, "each band alone: {two:?}");
        let left = bands_response(vec![bell(BandChannel::Left)], vec![100.0, 1000.0]);
        assert!((at(&left.left).1 - 6.0).abs() < 0.05, "{left:?}");
        assert!(left.right.as_deref().is_some_and(|r| r.iter().all(|d| d.abs() < 1e-3)), "{left:?}");
    }
}
