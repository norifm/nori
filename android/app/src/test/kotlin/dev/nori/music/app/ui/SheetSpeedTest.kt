package dev.nori.music.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class SheetSpeedTest {
    private val travel = 2000f

    @Test fun `the sheet carries on the finger's way, and from rest when the finger went the other way`() {
        // target, finger velocity (positive towards putting it away), sheets a second (positive towards opening)
        val cases = listOf(
            Triple(1f, -400f, 0.2f),
            Triple(0f, 400f, -0.2f),
            Triple(1f, 400f, 0f),
            Triple(0f, -400f, 0f),
            Triple(1f, 0f, 0f),
        )
        for ((target, velocity, speed) in cases) {
            assertEquals("to $target at $velocity px/s", speed, sheetSpeed(target, velocity, travel), 1e-6f)
        }
    }
}
