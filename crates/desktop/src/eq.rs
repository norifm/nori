//! The equalizer page, as Android's EqualizerScreen: mode, bands and curve, presets, pre-amp, balance,
//! mono and limiter. All numbers come from the core.

use nori_core::dsp::{eq_drawing, effective_preamp_db, eq_presets, graphic_bands, graphic_layouts, EqDrawing, EqPoint, CURVE_DB};
use nori_core::numbers::{self, POINT};
use nori_core::settings::{EqMode, StoredPrefs, EQ_RANGES};
use nori_core::{EqKind, PresetKind};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{AppWindow, EqBandRow};

/// "+1.5", "−3.0": the core's decibels with a typographic minus.
pub fn signed(v: f32) -> String {
    numbers::signed_db(v, POINT).replace('-', "−")
}

pub fn preset_name(kind: PresetKind) -> &'static str {
    match kind {
        PresetKind::Flat => "Flat",
        PresetKind::BassBoost => "Bass boost",
        PresetKind::BassCut => "Bass cut",
        PresetKind::TrebleBoost => "Treble boost",
        PresetKind::TrebleCut => "Treble cut",
        PresetKind::VocalBoost => "Vocal boost",
        PresetKind::Loudness => "Loudness",
        PresetKind::SmallSpeakers => "Small speakers",
    }
}

fn kind_name(k: EqKind) -> &'static str {
    match k {
        EqKind::Peaking => "Peak",
        EqKind::LowShelf | EqKind::LowShelfSlope => "Low shelf",
        EqKind::HighShelf | EqKind::HighShelfSlope => "High shelf",
        EqKind::LowPass => "Low pass",
        EqKind::HighPass => "High pass",
        EqKind::BandPass => "Band pass",
        EqKind::Notch => "Notch",
        EqKind::AllPass => "All pass",
    }
}

/// The curve's points as SVG path commands in a `w` x `h` box: 20 Hz..20 kHz across, ±[`CURVE_DB`]. `closed`
/// runs back along the 0 dB line, to fill what it lifts or cuts.
fn trace(r: &[f32], w: f32, h: f32, closed: bool) -> String {
    let last = r.len().saturating_sub(1).max(1) as f32;
    let mut out = String::new();
    for (i, db) in r.iter().enumerate() {
        let x = w * i as f32 / last;
        let y = h / 2.0 - db.clamp(-CURVE_DB, CURVE_DB) / CURVE_DB * (h / 2.0 - 3.0);
        out.push_str(&format!("{}{:.1} {:.1} ", if i == 0 { "M" } else { "L" }, x, y));
    }
    if closed && !r.is_empty() {
        out.push_str(&format!("L{w:.1} {:.1} L0.0 {:.1} Z", h / 2.0, h / 2.0));
    }
    out
}

/// What the equalizer plays, drawn: its curve as a line and as an area, and each parametric band's own
/// curve as an area (the graphic sliders have none).
struct Drawn {
    line: String,
    area: String,
    areas: Vec<String>,
}

fn curve(d: &EqDrawing, w: f32, h: f32) -> Drawn {
    Drawn {
        line: trace(&d.curve, w, h, false),
        area: trace(&d.curve, w, h, true),
        areas: d.each.iter().map(|e| trace(e, w, h, true)).collect(),
    }
}

fn show_curve(ui: &AppWindow, drawing: &EqDrawing) {
    let d = curve(drawing, ui.get_eq_curve_w(), ui.get_eq_curve_h());
    ui.set_eq_curve(d.line.into());
    ui.set_eq_area(d.area.into());
    ui.set_eq_shape_areas(ModelRc::new(VecModel::from(d.areas.into_iter().map(SharedString::from).collect::<Vec<_>>())));
}

/// The bands as the page draws them: where each sits on the curve, and what its editor shows.
fn band_rows(p: &StoredPrefs, points: &[EqPoint]) -> Vec<EqBandRow> {
    let point = |i: usize| points.get(i).copied().unwrap_or(EqPoint { x: 0.0, db: 0.0 });
    if p.eq_mode == EqMode::Graphic {
        let centres = graphic_bands(p.eq_graphic.len() as u32);
        p.eq_graphic
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let centre = centres.get(i);
                EqBandRow {
                    label: centre.map_or(String::new(), |b| numbers::iso_band(b.label_hz, POINT)).into(),
                    gain: *g,
                    value: signed(*g).into(),
                    uses_gain: true,
                    x: point(i).x,
                    db: point(i).db,
                    ..Default::default()
                }
            })
            .collect()
    } else {
        p.eq_bands
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let side = match b.channel as i32 {
                    1 => " L",
                    2 => " R",
                    _ => "",
                };
                EqBandRow {
                    label: format!("{}{side}", numbers::hz(b.freq, POINT)).into(),
                    gain: b.gain_db,
                    value: if b.kind.uses_gain() { signed(b.gain_db).into() } else { kind_name(b.kind).into() },
                    uses_gain: b.kind.uses_gain(),
                    x: point(i).x,
                    db: point(i).db,
                    kind: b.kind as i32,
                    channel: b.channel as i32,
                    q: b.q,
                    freq: numbers::hz(b.freq, POINT).into(),
                    shape: numbers::fixed(b.q as f64, 2, false, POINT).into(),
                }
            })
            .collect()
    }
}

/// While a band is dragged: the curve, and the bands changed in place, so the one held stays held.
pub fn curve_only(ui: &AppWindow, p: &StoredPrefs) {
    let drawing = eq_drawing(p, 97);
    show_curve(ui, &drawing);
    let rows = band_rows(p, &drawing.points);
    let model = ui.get_eq_bands();
    match model.as_any().downcast_ref::<VecModel<EqBandRow>>().filter(|m| m.row_count() == rows.len()) {
        Some(m) => {
            for (i, r) in rows.into_iter().enumerate() {
                if m.row_data(i).as_ref() != Some(&r) {
                    m.set_row_data(i, r);
                }
            }
        }
        None => ui.set_eq_bands(ModelRc::new(VecModel::from(rows))),
    }
}

/// The band kinds' names, in `EqKind` order.
pub fn kind_names() -> Vec<SharedString> {
    [
        EqKind::Peaking,
        EqKind::LowShelf,
        EqKind::HighShelf,
        EqKind::LowPass,
        EqKind::HighPass,
        EqKind::BandPass,
        EqKind::Notch,
        EqKind::AllPass,
        EqKind::LowShelfSlope,
        EqKind::HighShelfSlope,
    ]
    .map(|k| match k {
        EqKind::LowShelfSlope => "Low shelf (slope)".into(),
        EqKind::HighShelfSlope => "High shelf (slope)".into(),
        k => kind_name(k).into(),
    })
    .to_vec()
}

pub fn fill(ui: &AppWindow, p: &StoredPrefs) {
    let graphic = p.eq_mode == EqMode::Graphic;
    ui.set_eq_on(p.eq_enabled);
    ui.set_eq_graphic(graphic);
    ui.set_eq_count(p.eq_graphic.len() as i32);
    ui.set_eq_layouts(ModelRc::new(VecModel::from(graphic_layouts().into_iter().map(|n| n as i32).collect::<Vec<_>>())));
    curve_only(ui, p);
    ui.set_eq_kinds(ModelRc::new(VecModel::from(kind_names())));
    let presets: Vec<SharedString> = eq_presets().iter().map(|x| preset_name(x.kind).into()).collect();
    ui.set_eq_presets(ModelRc::new(VecModel::from(presets)));
    let auto = p.eq_preamp_db.is_none();
    ui.set_eq_auto_preamp(auto);
    let effective = effective_preamp_db(p);
    ui.set_eq_preamp(p.eq_preamp_db.unwrap_or(effective));
    ui.set_eq_preamp_label(format!("{} dB", signed(effective)).into());
    ui.set_eq_balance(p.balance);
    ui.set_eq_balance_label(
        match (p.balance * 100.0).round() as i32 {
            0 => "center".to_string(),
            n if n < 0 => format!("L {}%", -n),
            n => format!("R {n}%"),
        }
        .into(),
    );
    ui.set_eq_mono(p.mono);
    ui.set_eq_limiter(p.limiter);
    ui.set_eq_ceiling(p.limiter_threshold_db);
    ui.set_eq_ceiling_label(format!("Ceiling {} dB", signed(p.limiter_threshold_db)).into());
    ui.set_eq_bypass(if p.sound_bypass { "No processing on this output, so nothing here changes the sound. Turn it off in Settings, under Sound." } else { "" }.into());
    let r = EQ_RANGES;
    ui.set_eq_ranges(ModelRc::new(VecModel::from(vec![r.gain.min, r.gain.max, r.preamp.min, r.preamp.max, r.balance.min, r.balance.max, r.limiter.min, r.limiter.max, r.q.min, r.q.max])));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_traces_the_drawing() {
        // Curve spans the box flat at zero.
        let flat = StoredPrefs { eq_mode: EqMode::Graphic, eq_graphic: vec![0.0; 10], ..StoredPrefs::default() };
        let c = curve(&eq_drawing(&flat, 97), 960.0, 96.0).line;
        assert_eq!(c.matches('L').count(), 96);
        assert!(c.starts_with("M0.0 48.0"), "{c}");
        assert!(c.trim_end().ends_with("L960.0 48.0"), "{c}");
        // The parametric bands draw their own curve: a boost at 1 kHz rises in the middle.
        let bell = nori_core::settings::SoundBand { kind: EqKind::Peaking, freq: 632.5, gain_db: 6.0, q: 1.0, channel: nori_core::settings::BandChannel::Both };
        let parametric = StoredPrefs { eq_mode: EqMode::Parametric, eq_bands: vec![bell], ..StoredPrefs::default() };
        let drawn = curve(&eq_drawing(&parametric, 97), 960.0, 96.0);
        assert_eq!(drawn.areas.len(), 1, "the band's own curve");
        assert!(drawn.area.ends_with("Z"), "the area closes along 0 dB");
        let c = drawn.line;
        let middle: f32 = c.split_whitespace().nth(97).unwrap().parse().unwrap();
        assert!(middle < 35.0, "{c}");
    }

}
