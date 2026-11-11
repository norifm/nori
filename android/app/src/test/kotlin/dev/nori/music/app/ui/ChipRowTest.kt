package dev.nori.music.app.ui

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ChipRowTest {
    @Test
    fun shareRow() {
        val cases = listOf(
            // natural widths, room, widths
            Triple(intArrayOf(50, 70, 60, 40), 300, intArrayOf(75, 75, 75, 75)),
            Triple(intArrayOf(50, 70, 60, 40), 302, intArrayOf(76, 76, 75, 75)),
            Triple(intArrayOf(150, 50, 60), 300, intArrayOf(150, 75, 75)),
            Triple(intArrayOf(100, 100, 100), 300, intArrayOf(100, 100, 100)),
            Triple(intArrayOf(200, 90, 30), 320, intArrayOf(200, 90, 30)),
        )
        for ((natural, room, widths) in cases) assertArrayEquals(widths, shareRow(natural, room))
    }

    @Test
    fun wrapsWhenTheWordsDontFit() {
        assertNull(shareRow(intArrayOf(100, 120, 90), 300))
    }
}
