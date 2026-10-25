//! The equalizer page, as the desktop's: its curve and band points (`dsp::eq_drawing`), the band chosen's
//! editor, presets, pre-amp, balance, mono and limiter. Every edit answers with the page as it is now.

use nori_core::dsp::{eq_drawing, eq_presets, graphic_bands, graphic_layouts, CURVE_DB};
use nori_core::numbers::{self, POINT};
use nori_core::settings::{eq_bypass, EqMode, StoredPrefs, EQ_RANGES};
use nori_core::settings_store::{BandPoint, SoundTool};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::Nori;

/// Points across the drawn curve.
const SAMPLES: u32 = 129;

/// The page as JSON: `{ on, graphic, count, layouts, reach, curve, each, bands, presets, preamp, auto,
/// balance, mono, limiter, ceiling, ranges, bypass }`. A band is `{ x, db, gain, label, value, usesGain,
/// kind, channel, q, freq, shape }`: `x` across the curve (0..1), `db` its point's height; the rest
/// what its editor shows. `presets` are `PresetKind` names and `bypass` an `EqBypass` name, for the
/// page to word.
fn page(p: &StoredPrefs) -> String {
    let drawing = eq_drawing(p, SAMPLES);
    let signed = |v: f32| numbers::signed_db(v, POINT);
    let bands: Vec<Value> = if p.eq_mode == EqMode::Graphic {
        let centres = graphic_bands(p.eq_graphic.len() as u32);
        p.eq_graphic
            .iter()
            .zip(&drawing.points)
            .zip(&centres)
            .map(|((g, pt), c)| {
                json!({ "x": pt.x, "db": pt.db, "gain": g, "label": numbers::iso_band(c.label_hz, POINT), "value": signed(*g), "usesGain": true,
                        "kind": 0, "channel": 0, "q": 0.0, "freq": numbers::iso_band(c.label_hz, POINT), "shape": "" })
            })
            .collect()
    } else {
        p.eq_bands
            .iter()
            .zip(&drawing.points)
            .map(|(b, pt)| {
                let freq = numbers::hz(b.freq, POINT);
                json!({ "x": pt.x, "db": pt.db, "gain": b.gain_db, "label": freq, "value": signed(b.gain_db), "usesGain": b.kind.uses_gain(),
                        "kind": b.kind as i32, "channel": b.channel as i32, "q": b.q, "freq": freq, "shape": numbers::fixed(b.q as f64, 2, false, POINT) })
            })
            .collect()
    };
    let r = EQ_RANGES;
    json!({
        "on": p.eq_enabled,
        "graphic": p.eq_mode == EqMode::Graphic,
        "count": p.eq_graphic.len(),
        "layouts": graphic_layouts(),
        "reach": CURVE_DB,
        "curve": drawing.curve,
        "each": drawing.each,
        "bands": bands,
        "presets": eq_presets().iter().map(|x| format!("{:?}", x.kind)).collect::<Vec<_>>(),
        "preamp": nori_core::dsp::effective_preamp_db(p),
        "auto": p.eq_preamp_db.is_none(),
        "balance": p.balance,
        "mono": p.mono,
        "limiter": p.limiter,
        "ceiling": p.limiter_threshold_db,
        "ranges": { "gain": [r.gain.min, r.gain.max], "preamp": [r.preamp.min, r.preamp.max], "balance": [r.balance.min, r.balance.max],
                    "limiter": [r.limiter.min, r.limiter.max], "q": [r.q.min, r.q.max] },
        "bypass": eq_bypass(p.bit_perfect, p.sound_bypass).map(|b| format!("{b:?}")),
    })
    .to_string()
}

#[wasm_bindgen]
impl Nori {
    /// The equalizer page ([`page`]).
    pub fn equalizer(&self) -> String {
        page(&self.app.settings.current().unwrap_or_default())
    }

    /// Sets band `i` as the curve or its editor left it: `pos` its place across the curve (a graphic
    /// band keeps its own), its gain, Q, and kind and channel ordinals.
    pub fn eq_band(&self, i: u32, pos: f32, gain: f32, q: f32, kind: i32, channel: i32) -> String {
        if let Some(effect) = self.app.settings.edit_point(i, BandPoint { pos, gain_db: gain, q, kind, channel }) {
            self.sound_applied(effect);
        }
        self.equalizer()
    }

    /// Adds a parametric band where the curve was clicked.
    pub fn eq_add(&self, pos: f32, gain: f32) -> String {
        self.eq_sound_tool(SoundTool::AddBand { freq: nori_core::dsp::band_freq_at(pos), gain_db: gain })
    }

    /// Preset `i`, or band `i` removed, or the bands reset (`what`: "preset", "remove", "reset").
    pub fn eq_tool(&self, what: String, i: u32) -> String {
        let tool = match what.as_str() {
            "preset" => eq_presets().into_iter().nth(i as usize).map(|preset| SoundTool::Preset { preset }),
            "remove" => Some(SoundTool::RemoveBand { index: i }),
            "reset" => Some(SoundTool::ResetBands),
            _ => None,
        };
        match tool {
            Some(tool) => self.eq_sound_tool(tool),
            None => self.equalizer(),
        }
    }

    fn eq_sound_tool(&self, tool: SoundTool) -> String {
        if let Ok(Some(change)) = self.app.settings.sound_tool(tool) {
            self.sound_applied(change.effect);
        }
        self.equalizer()
    }

    /// A sound edit with effect bits `effect`: the engine hears it.
    fn sound_applied(&self, effect: u32) {
        #[cfg(feature = "engine")]
        self.with(|h| h.applied(effect));
        #[cfg(not(feature = "engine"))]
        let _ = effect;
    }
}
