package dev.nori.music.app.vm

import org.junit.Assert.assertEquals
import org.junit.Test

class AutoEqUiTest {
    @Test fun aFailureAfterACurveSaysOnlyTheFailure() {
        val applying = AutoEqUi(task = AutoEqTask.Applying(applied = "Sony WH-1000XM5"))
        // Said while the next one is on its way, as before it.
        assertEquals("Sony WH-1000XM5" to true, applying.applied to applying.busy)
        val failed = applying.copy(task = AutoEqTask.Failed("offline"))
        assertEquals(Triple(null, "offline", false), Triple(failed.applied, failed.error, failed.busy))
    }
}
