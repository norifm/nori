package dev.nori.music.playback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TransportTest {
    @Test fun theEnginesFocusHoldMovesWhatIsWanted() {
        // was, the engine's hold -> now.
        val table = listOf(
            (Want.PLAYING to 1) to Want.HELD_BY_CALL,
            (Want.PLAYING to 2) to Want.PAUSED,
            (Want.HELD_BY_CALL to 2) to Want.PAUSED,
            (Want.HELD_BY_CALL to 0) to Want.PLAYING,
            (Want.PAUSED to 0) to Want.PAUSED,
            (Want.PAUSED to 1) to Want.PAUSED,
            (Want.PLAYING to 0) to Want.PLAYING,
        )
        for ((given, now) in table) assertEquals("$given", now, given.first.held(given.second))
    }

    @Test fun followingCodesDecodeBothBits() {
        assertNull(Followed.of(-1))
        assertEquals(listOf(Followed.HOST_PAUSED, Followed.PLAYING, Followed.HELD_HOST_PAUSED, Followed.HELD_HOST_PLAYING), (0..3).map(Followed::of))
        assertEquals(listOf(true, false), listOf(Followed.of(3)!!.playing, Followed.of(2)!!.playing))
        assertEquals(listOf(true, false), listOf(Followed.of(2)!!.held, Followed.of(1)!!.held))
    }

    @Test fun aMoveSaysThePlaceToo() {
        assertEquals(Discontinuity.MOVED, Discontinuity.MOVED.and(Discontinuity.PLACED))
        assertEquals(Discontinuity.MOVED, Discontinuity.PLACED.and(Discontinuity.MOVED))
        assertEquals(Discontinuity.PLACED, Discontinuity.NONE.and(Discontinuity.PLACED))
    }

    @Test fun engineStatesFollowTheEngineCodes() {
        assertEquals(listOf(EngineState.IDLE, EngineState.PLAYING, EngineState.PAUSED, EngineState.ENDED), (0..3).map(EngineState::of))
    }
}
