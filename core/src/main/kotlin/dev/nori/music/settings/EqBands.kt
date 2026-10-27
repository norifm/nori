package dev.nori.music.settings

import dalvik.annotation.optimization.CriticalNative
import dalvik.annotation.optimization.FastNative

/**
 * What the equalizer screen asks of the core about its bands on every frame of a drag, as primitives:
 * what a band's label marks and where a frequency sits on the band scale.
 */
object EqBands {
    init { System.loadLibrary("norimusic") }

    /**
     * What a band's label marks after its frequency (`settings::band_mark`), as its place in
     * `BandMark`: 0 nothing, 1 left, 2 right, 3 low shelf, 4 high shelf, 5 no gain. Primitives only.
     */
    @JvmStatic @CriticalNative external fun mark(kind: Int, channel: Int): Int

    /** Where a frequency sits on a band's frequency scale, 20 Hz at 0 to 20 kHz at 1 (`dsp::band_position`). */
    @JvmStatic @CriticalNative external fun position(freq: Float): Float

    /** The frequency at a place on that scale, rounded as the core rounds it (`dsp::band_freq_at`). */
    @JvmStatic @CriticalNative external fun freqAt(position: Float): Float

    /** A band at [freq] dragged to [position]: [freq] itself while it rounds the same there (`dsp::band_moved`). */
    @JvmStatic @CriticalNative external fun moved(freq: Float, position: Float): Float
}

/**
 * The equalizer's sliders, edited in the core where the settings are kept (settings_store.rs), so a
 * drag builds no settings record and sends none across. -1 means nothing changed.
 */
internal object SoundEdit {
    init { System.loadLibrary("norimusic") }

    // [settings] is the app's settings' handle (`Settings.handle`).

    /** [band] is `[kind, freq, gain, q, channel]` in, and the band as it was kept out; returns the effects. */
    @JvmStatic @FastNative external fun setBand(settings: Long, index: Int, band: FloatArray): Int
    /** [level] an `EqLevel` ordinal; the value as it was kept as float bits in the high 32, the effects in the low. */
    @JvmStatic @CriticalNative external fun setLevel(settings: Long, level: Int, value: Float): Long
    /** One graphic slider: the value as it was kept as float bits in the high 32, the effects in the low; -1 for no change. */
    @JvmStatic @CriticalNative external fun setGraphic(settings: Long, index: Int, value: Float): Long
}
