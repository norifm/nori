package dev.nori.music.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class UiScaleTest {
    @Test fun `a large display size keeps a phone's proportions, never enlarged or below three quarters`() {
        for ((widthDp, scale) in listOf(358 to 358f / 411f, 411 to 1f, 600 to 1f, 200 to 0.75f, 0 to 1f)) {
            assertEquals("$widthDp dp", scale, uiScale(widthDp), 1e-4f)
        }
    }
}
