//! The equalizer page, as Android's EqualizerScreen: mode, bands and curve, presets, pre-amp, balance,
//! mono and limiter. All numbers come from the core.

use nori_core::dsp::{bands_response, effective_preamp_db, eq_presets, graphic_bands, graphic_layouts, graphic_response};
use nori_core::numbers::{self, POINT};
use nori_core::settings::{EqMode, StoredPrefs, EQ_RANGES};
use nori_core::{EqKind, PresetKind};
use slint::{ModelRc, SharedString, VecModel};

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
        EqKind::LowPass => "Low-pass",
        EqKind::HighPass => "High-pass",
        EqKind::BandPass => "Band-pass",
        EqKind::Notch => "Notch",
        EqKind::AllPass => "All-pass",
    }
}

/// The equalizer's response as an SVG path in a `w` x `h` box (20 Hz..20 kHz, ±15 dB): the graphic
/// sliders', or the parametric bands' (the left channel's, where the sides differ).
fn curve(p: &StoredPrefs, w: f32, h: f32) -> String {
    let freqs: Vec<f32> = (0..97).map(|i| 20.0 * 1000f32.powf(i as f32 / 96.0)).collect();
    let r = if p.eq_mode == EqMode::Graphic { graphic_response(p.eq_graphic.clone(), freqs) } else { bands_response(p.eq_bands.clone(), freqs).left };
    let mut out = String::new();
    for (i, db) in r.iter().enumerate() {
        let x = w * i as f32 / 96.0;
        let y = h / 2.0 - db.clamp(-15.0, 15.0) / 15.0 * (h / 2.0 - 3.0);
        out.push_str(&format!("{}{:.1} {:.1} ", if i == 0 { "M" } else { "L" }, x, y));
    }
    out
}

/// Updates only the curve, while a band is dragged.
pub fn curve_only(ui: &AppWindow, p: &StoredPrefs) {
    ui.set_eq_curve(curve(p, ui.get_eq_curve_w(), ui.get_eq_curve_h()).into());
}

pub fn fill(ui: &AppWindow, p: &StoredPrefs) {
    let graphic = p.eq_mode == EqMode::Graphic;
    ui.set_eq_on(p.eq_enabled);
    ui.set_eq_graphic(graphic);
    ui.set_eq_count(p.eq_graphic.len() as i32);
    ui.set_eq_layouts(ModelRc::new(VecModel::from(graphic_layouts().into_iter().map(|n| n as i32).collect::<Vec<_>>())));
    let bands: Vec<EqBandRow> = if graphic {
        let centres = graphic_bands(p.eq_graphic.len() as u32);
        p.eq_graphic
            .iter()
            .enumerate()
            .map(|(i, g)| EqBandRow { label: centres.get(i).map_or(String::new(), |b| numbers::iso_band(b.label_hz, POINT)).into(), gain: *g, value: signed(*g).into(), uses_gain: true })
            .collect()
    } else {
        p.eq_bands
            .iter()
            .map(|b| {
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
                }
            })
            .collect()
    };
    ui.set_eq_bands(ModelRc::new(VecModel::from(bands)));
    ui.set_eq_curve(curve(p, ui.get_eq_curve_w(), ui.get_eq_curve_h()).into());
    let presets: Vec<SharedString> = eq_presets().iter().map(|x| preset_name(x.kind).into()).collect();
    ui.set_eq_presets(ModelRc::new(VecModel::from(presets)));
    let auto = p.eq_preamp_db.is_none();
    ui.set_eq_auto_preamp(auto);
    let effective = effective_preamp_db(p);
    ui.set_eq_preamp(p.eq_preamp_db.unwrap_or(effective));
    ui.set_eq_preamp_label(if auto { format!("Pre-amp {} dB (automatic)", signed(effective)) } else { format!("Pre-amp {} dB", signed(effective)) }.into());
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
    ui.set_eq_ranges(ModelRc::new(VecModel::from(vec![r.gain.min, r.gain.max, r.preamp.min, r.preamp.max, r.balance.min, r.balance.max, r.limiter.min, r.limiter.max])));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_drawing() {
        // Curve spans the box flat at zero.
        let flat = StoredPrefs { eq_mode: EqMode::Graphic, eq_graphic: vec![0.0; 10], ..StoredPrefs::default() };
        let c = curve(&flat, 960.0, 96.0);
        assert_eq!(c.matches('L').count(), 96);
        assert!(c.starts_with("M0.0 48.0"), "{c}");
        assert!(c.trim_end().ends_with("L960.0 48.0"), "{c}");
        // The parametric bands draw their own curve: a boost at 1 kHz rises in the middle.
        let bell = nori_core::settings::SoundBand { kind: EqKind::Peaking, freq: 632.5, gain_db: 6.0, q: 1.0, channel: nori_core::settings::BandChannel::Both };
        let parametric = StoredPrefs { eq_mode: EqMode::Parametric, eq_bands: vec![bell], ..StoredPrefs::default() };
        let c = curve(&parametric, 960.0, 96.0);
        let middle: f32 = c.split_whitespace().nth(97).unwrap().parse().unwrap();
        assert!(middle < 35.0, "{c}");
    }

}
