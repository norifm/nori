package dev.nori.music.app.widget

import org.junit.Assert.assertEquals
import org.junit.Test

class MiniFitTest {
    @Test
    fun theOneRowPlayerKeepsWhatFitsInItsWidth() {
        // width, height (dp) to the cover's side, whether the words show, whether Next shows
        val cases = listOf(
            Triple(250, 56, MiniFit(40, words = true, next = true)),
            Triple(320, 110, MiniFit(94, words = true, next = true)),
            Triple(200, 56, MiniFit(40, words = true, next = false)),
            Triple(170, 103, MiniFit(87, words = false, next = false)),
            Triple(110, 139, MiniFit(50, words = false, next = false)),
        )
        for ((w, h, fit) in cases) assertEquals("$w x $h", fit, miniFit(w, h))
    }
}
