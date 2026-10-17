//! The effective equalizer pre-amp, for the settings screen.

use jni::objects::{JClass, JFloatArray, JIntArray};
use jni::sys::{jboolean, jfloat};
use jni::JNIEnv;

use crate::{native, Class};

pub(crate) static CLASS: Class = Class {
    name: c"dev/nori/music/playback/Dsp",
    methods: &[
        native!(c"effectivePreampDb", c"(ZFZZ[I[F[F)F", effective_preamp_db),
    ],
};

/// `nori_core::settings::mode_preamp_db`, called on every step of a band drag: the parametric bands as
/// kind and gain arrays, and the graphic sliders; `graphic` picks the equalizer in use.
#[allow(clippy::too_many_arguments)]
extern "system" fn effective_preamp_db(
    env: JNIEnv, _: JClass, eq_enabled: jboolean, eq_preamp_db: jfloat, automatic: jboolean, graphic: jboolean, kinds: JIntArray, gains: JFloatArray, sliders: JFloatArray,
) -> jfloat {
    let n = env.get_array_length(&kinds).unwrap_or(0).max(0) as usize;
    let m = env.get_array_length(&sliders).unwrap_or(0).max(0) as usize;
    let (mut k, mut g, mut sl) = (vec![0i32; n], vec![0f32; n], vec![0f32; m]);
    if env.get_int_array_region(&kinds, 0, &mut k).is_err() || env.get_float_array_region(&gains, 0, &mut g).is_err() || env.get_float_array_region(&sliders, 0, &mut sl).is_err() {
        return 0.0;
    }
    let bands: Vec<_> = k.into_iter().zip(g).map(|(kind, gain)| nori_core::settings::band_from(kind, 1000.0, gain, 1.0, 0)).collect();
    let mode = if graphic != 0 { nori_core::settings::EqMode::Graphic } else { nori_core::settings::EqMode::Parametric };
    nori_core::settings::mode_preamp_db(eq_enabled != 0, (automatic == 0).then_some(eq_preamp_db), mode, &sl, &bands)
}
