package dev.nori.music.look

import dalvik.annotation.optimization.CriticalNative
import dalvik.annotation.optimization.FastNative

/**
 * How a page looks, worked out in Rust (crates/look) so every app built on it dresses the same record
 * the same way: a cover's colours and everything dressed in them - the theme's roles, the plates behind
 * the buttons, the chrome, the status bar, the gradients' stops - as one table of ints, looked up and
 * never recomputed. See crates/lyrics/src/look.rs and crates/look/src/dress.rs, whose indices these are.
 */
object CoverLook {
    init { System.loadLibrary("norimusic") }

    /** Pixels a side of the wash handed back. */
    const val WASH = 128

    // A look's entries (nori_look::dress). Colours are ARGB; PAPER, BAND_TINT and the BAND_K* are Float
    // bits, and STATUS_LIGHT is 0 or 1.
    const val EDGE = 0
    const val BACKGROUND = 1
    const val ON = 2
    const val ON_VARIANT = 3
    const val ACCENT = 4
    const val MELT = 5
    const val ON_PRIMARY = 6
    const val SURFACE_VARIANT = 7
    const val SURFACE_CONTAINER = 8
    const val SURFACE_CONTAINER_HIGH = 9
    const val SECONDARY_CONTAINER = 10
    const val OUTLINE_VARIANT = 11
    const val PAPER = 12
    const val PILL = 13
    const val PILL_INK = 14
    const val PILL_PLATE = 15
    const val TINT_INK = 16
    const val FIELD = 17
    const val FORM = 18
    const val SWITCH_OFF = 19
    const val VEIL_13 = 20
    const val VEIL_6 = 21
    const val VEIL_10 = 22
    const val ON_60 = 23
    const val ON_45 = 24
    const val ON_55 = 25
    const val ON_22 = 26
    const val ON_85 = 27
    const val ON_35 = 28
    const val ON_80 = 29
    const val ON_VARIANT_70 = 30
    const val CHROME_SLAB = 31
    const val CHROME_CONTENT = 32
    const val CHROME_PAGE = 33
    const val CHROME_EDGE = 34
    const val CHROME_CONTENT_65 = 35
    const val CHROME_CONTENT_75 = 36
    const val CHROME_FILL = 37
    const val STATUS_LIGHT = 38
    const val BAND_TINT = 39
    const val BAND_KR = 40
    const val BAND_KG = 41
    const val BAND_KB = 42
    const val HERO_EDGE = 43
    const val HERO_MID = 44
    const val FLOOR_0 = 45
    const val FLOOR_22 = 46
    const val FLOOR_75 = 47
    /** The see-through chrome's outline. */
    const val CHROME_OUTLINE = 48
    const val LEN = 49

    /**
     * Looks [from] and [to] mixed at [t] into [out], all [LEN] entries (`nori_look::dress::mix`): one frame
     * of a page cross-fading. Primitives only, so a frame crosses once and allocates nothing.
     */
    @JvmStatic @FastNative external fun mix(from: IntArray, to: IntArray, t: Float, out: IntArray)

    /**
     * A page's look ([LEN] entries) and its wash, [WASH] x [WASH] pixels, or null on AMOLED black. The
     * cover loader works them out from the cover's own pixels ([dev.nori.music.data.CoverLoader.colours]).
     */
    class Colours(val look: IntArray, val wash: android.graphics.Bitmap?)

    /**
     * The look of a page in the theme's own colours, from its roles: background, on surface, on surface
     * variant, primary, on primary, surface variant, surface container, surface container high,
     * secondary container, outline variant. Once per theme.
     */
    fun plain(roles: IntArray): IntArray = IntArray(LEN).also { plain(roles, it) }

    /**
     * [look] wearing the theme's accent in place of its cover's: [light] on a pale page, [dark] on a dark
     * one, moved until it reads there (`nori_look::dress::accented`).
     */
    fun accented(look: IntArray, light: Int, dark: Int): IntArray = IntArray(LEN).also { accented(look, light, dark, it) }

    /**
     * [look] (a cover's page) on the season's paper dyed with its edge (`nori_look::dress::dyed`). [palettes]
     * is the light then the dark palette the app wears, each page, surface, ink, accent, on; [dark] the theme.
     */
    fun dyed(look: IntArray, palettes: IntArray, dark: Boolean): IntArray = IntArray(LEN).also { dyed(look, palettes, dark, it, null) }

    /** [dyed] with the cover's [wash], if any, dyed too: a copy, the original is left as it is. */
    fun dyed(look: IntArray, palettes: IntArray, dark: Boolean, wash: android.graphics.Bitmap?): Colours {
        val out = IntArray(LEN)
        val copy = wash?.copy(android.graphics.Bitmap.Config.ARGB_8888, true)
        return Colours(out, copy.takeIf { dyed(look, palettes, dark, out, copy) == 2 })
    }

    /**
     * One step of the seek bar towards where the song is (`nori_look::motion::seek_step`): the new place,
     * and the milliseconds to wait before the next step (0 the next frame, -1 stop). Primitives only.
     */
    /**
     * The seek bar's two times (`nori-core stage::seek_times`) as `atS shl 32 or leftS`, whole seconds:
     * the finger's place while [dragging], else a held seek ([heldMs] -1 for none), else the music's.
     * Primitives only: it is asked on every frame a finger moves the bar.
     */
    @JvmStatic @CriticalNative external fun seekTimes(dragging: Boolean, drag: Float, heldMs: Long, positionMs: Long, durationMs: Long): Long

    /**
     * The seek bar on an e-ink screen (`nori_look::eink::EinkSeek`), packed: bits 56+ how many blocks
     * (0 for an unknown length), 48-55 how many are played, 32-47 whole minutes played, the low 32 the
     * song milliseconds until either changes.
     */
    @JvmStatic @CriticalNative external fun einkSeek(positionMs: Long, durationMs: Long): Long

    @JvmStatic @CriticalNative external fun seekStep(bar: Float, target: Float, dtS: Float, widthPx: Float, speed: Float): Long

    // The seek bar's pace (nori_look::motion::SeekPace), through [SeekPace]: a handle and primitives in,
    // what it shows left in [SeekPace]'s view.
    @JvmStatic @CriticalNative external fun seekPaceNew(view: Long, reduced: Boolean): Long
    @JvmStatic @CriticalNative external fun seekPaceFree(h: Long)
    @JvmStatic @CriticalNative external fun seekPaceSync(h: Long, positionMs: Long, durationMs: Long)
    @JvmStatic @CriticalNative external fun seekPaceHold(h: Long, bar: Float, positionMs: Long, durationMs: Long)
    @JvmStatic @CriticalNative external fun seekPaceStep(h: Long, positionMs: Long, durationMs: Long, dtS: Float, widthPx: Float, rate: Float): Int

    /** Which glyph the play button shows (`nori-core stage::transport_glyph`): 0 play, 1 pause, 2 spinner. */
    @JvmStatic @CriticalNative external fun transportGlyph(playing: Boolean, buffering: Boolean, waited: Boolean): Int

    /**
     * A page's Shuffle and Play (`nori-core pages::hero_buttons`), packed by `HeroButtons::pack`: bit 0
     * Shuffle lit, 1 Shuffle enabled, 2 pausing, 3 Play enabled, bits 4-5 and 6-7 what Shuffle and Play
     * press (0 start, 1 toggle, 2 shuffle off). Primitives only: it is asked on every play and pause.
     */
    @JvmStatic @CriticalNative external fun heroButtons(here: Boolean, shuffle: Boolean, playing: Boolean, buffering: Boolean, canPlay: Boolean, canShuffle: Boolean): Int

    @JvmStatic @FastNative private external fun plain(roles: IntArray, out: IntArray)
    @JvmStatic @FastNative private external fun accented(look: IntArray, light: Int, dark: Int, out: IntArray)
    /** Returns 0 nothing written, 1 the look, 2 the look and [wash], a mutable ARGB_8888 copy dyed in place. */
    @JvmStatic @FastNative private external fun dyed(look: IntArray, palettes: IntArray, dark: Boolean, out: IntArray, wash: android.graphics.Bitmap?): Int
}
