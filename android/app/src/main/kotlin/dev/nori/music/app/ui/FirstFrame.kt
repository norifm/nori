package dev.nori.music.app.ui

import androidx.compose.runtime.staticCompositionLocalOf

/**
 * Holds the window's first frame until the page it opens on has what the device keeps for it, so that page
 * is whole in the first frame instead of filling in over the next ones. Only a page that reads nothing but
 * the device holds it ([claim]); a start on any other page lets it go once the first composition is in
 * ([composed]). Main thread.
 *
 * What it waits for is the device's own reads, tens of milliseconds; a held window is an app that seems
 * frozen, so however slow the storage, the frame goes out after [LONGEST_HOLD_MS] whole or not.
 */
class FirstFrame {
    private val since = android.os.SystemClock.uptimeMillis()
    private var claimed = false
    private var ready = false

    /** Whether the frame may go out; checked on each frame while it is held. */
    val open: Boolean get() = ready || android.os.SystemClock.uptimeMillis() - since >= LONGEST_HOLD_MS

    /** A page holds the frame until it is [ready]. */
    fun claim(ready: Boolean) {
        claimed = true
        if (ready) this.ready = true
    }

    /** The first composition is applied: with no page holding the frame, it goes out. */
    fun composed() {
        if (!claimed) ready = true
    }

    private companion object {
        const val LONGEST_HOLD_MS = 500L
    }
}

val LocalFirstFrame = staticCompositionLocalOf { FirstFrame().also { it.composed() } }
