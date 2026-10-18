package dev.nori.music.playback

import dalvik.annotation.optimization.FastNative

/** The pre-amp in effect, which the settings ask for as they are read; see crates/android/src/dsp.rs. */
object Dsp {
    init { System.loadLibrary("norimusic") }

    /**
     * The pre-amp in effect (the core's `settings::mode_preamp_db`) for the equalizer [graphic] picks: the
     * parametric bands' [kinds] and [gains], or the graphic [sliders]; [preampDb] is ignored when [automatic].
     */
    @JvmStatic @FastNative external fun effectivePreampDb(eqEnabled: Boolean, preampDb: Float, automatic: Boolean, graphic: Boolean, kinds: IntArray, gains: FloatArray, sliders: FloatArray): Float
}

/**
 * The sample-domain chain (pre-amp, parametric equalizer, balance, mono, limiter) as a screen
 * reads it. The chain runs inside the player (nori-engine), which follows the settings by itself; this
 * only asks it, so a screen can show its meter without reaching into the service.
 */
object Equalizer {
    /** Whether the sound chain is in the samples' path now. */
    val inChain: Boolean get() = PlaybackService.rustPlayer?.chainIn ?: false

    /** The limiter's meter: what it takes off right now, dB. */
    val meterDb: Float get() = PlaybackService.rustPlayer?.gainReductionDb ?: 0f

    /** The compressor's meter: what it takes off right now, dB. */
    val compressionDb: Float get() = PlaybackService.rustPlayer?.compressionDb ?: 0f
}
