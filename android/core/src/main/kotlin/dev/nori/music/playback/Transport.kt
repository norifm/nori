package dev.nori.music.playback

import androidx.media3.common.Player

/** What the listener wants of [EnginePlayer]'s music: media3's playWhenReady with its suppression. */
internal enum class Want {
    PAUSED,
    PLAYING,
    /** Wanted, held paused while another app has the focus for a while (a call). */
    HELD_BY_CALL;

    val playWhenReady: Boolean get() = this != PAUSED
    val suppression: Int get() =
        if (this == HELD_BY_CALL) Player.PLAYBACK_SUPPRESSION_REASON_TRANSIENT_AUDIO_FOCUS_LOSS else Player.PLAYBACK_SUPPRESSION_REASON_NONE

    /**
     * What is wanted once the engine says where the audio focus holds the music (nori-engine's
     * `Event::Focus`; [hold] 0 nothing, 1 a call for a while, 2 for good, a refused play too).
     */
    fun held(hold: Int): Want = when {
        hold == 1 && this != PAUSED -> HELD_BY_CALL
        hold == 2 -> PAUSED
        hold == 0 && this == HELD_BY_CALL -> PLAYING
        else -> this
    }
}

/** nori-engine's state (crates/android/src/player.rs `state_code`). */
internal enum class EngineState {
    IDLE, PLAYING, PAUSED, ENDED;

    companion object {
        private val all = entries.toTypedArray()
        fun of(code: Int): EngineState = all.getOrElse(code) { IDLE }
    }
}

/**
 * How the engine follows a jam's host (nori-engine's `Followed`): the music [playing] there, and this
 * phone's own pause [held] holding it here.
 */
internal enum class Followed(val playing: Boolean, val held: Boolean) {
    HOST_PAUSED(false, false),
    PLAYING(true, false),
    HELD_HOST_PAUSED(false, true),
    HELD_HOST_PLAYING(true, true);

    companion object {
        private val all = entries.toTypedArray()
        /** The engine's `EVENT_FOLLOWING` code: -1 not following, else bit 0 [playing] and bit 1 [held]. */
        fun of(code: Int): Followed? = if (code < 0) null else all[code and 3]
    }
}

/** A jump the next state says once, so the session and its controllers anchor their clocks there. */
internal enum class Discontinuity {
    NONE,
    /** The engine moved the music to another path (the audio chip to the CPU or back), or landed a jump. */
    PLACED,
    /** The engine moved on by itself: a song ended into the next, or looped. Says the place too. */
    MOVED;

    /** This and [other] said by the same state: a move says the place as well. */
    fun and(other: Discontinuity): Discontinuity = if (other.ordinal > ordinal) other else this
}
