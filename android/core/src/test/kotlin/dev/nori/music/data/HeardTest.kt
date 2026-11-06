package dev.nori.music.data

import org.junit.Assert.assertEquals
import org.junit.Test

class HeardTest {
    @Test fun `a song's words follow only that song's playhead`() {
        var read = 0
        val at = { read++; 42_000L }
        assertEquals(42_000L, playheadFor("a", "a", at))
        assertEquals("skipped to b: a's words stand where they were", null, playheadFor("a", "b", at))
        assertEquals("nothing playing", null, playheadFor("a", null, at))
        assertEquals("words of no song", null, playheadFor(null, null, at))
        assertEquals("the player is asked only for its own song", 1, read)
    }
}
