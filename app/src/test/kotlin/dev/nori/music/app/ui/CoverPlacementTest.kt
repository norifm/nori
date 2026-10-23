package dev.nori.music.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class CoverPlacementTest {
    @Test fun `a small stand-in and the full picture land on the same pixels`() {
        // A hero taller than square on a 1080 px wide phone: the card's 300 px copy, then the full picture.
        for ((w, h) in listOf(1080f to 1212f, 1080f to 1080f, 1440f to 1600f, 720f to 803f)) {
            val full = coverPlacement(1200, 1200, w, h)
            for (standIn in listOf(150, 300, 600)) {
                assertEquals("$standIn px into $w x $h", full, coverPlacement(standIn, standIn, w, h))
            }
        }
    }

    @Test fun `the picture covers the area, centred`() {
        val (at, size) = coverPlacement(1000, 500, 300f, 300f)!!
        assertEquals(600, size.width)
        assertEquals(300, size.height)
        assertEquals(-150, at.x)
        assertEquals(0, at.y)
    }
}
