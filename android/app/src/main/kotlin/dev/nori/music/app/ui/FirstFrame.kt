package dev.nori.music.app.ui

import androidx.compose.runtime.staticCompositionLocalOf

/**
 * Holds the window's first frame until the page it opens on has what the device keeps for it, so that page
 * is whole in the first frame instead of filling in over the next ones. Only a page that reads nothing but
 * the device holds it ([claim]); a start on any other page lets it go once the first composition is in
 * ([composed]). Main thread.
 */
class FirstFrame {
    private var claimed = false

    /** Whether the frame may go out. */
    var open = false
        private set

    /** A page holds the frame until it is [ready]. */
    fun claim(ready: Boolean) {
        claimed = true
        if (ready) open = true
    }

    /** The first composition is applied: with no page holding the frame, it goes out. */
    fun composed() {
        if (!claimed) open = true
    }
}

val LocalFirstFrame = staticCompositionLocalOf { FirstFrame().also { it.composed() } }
