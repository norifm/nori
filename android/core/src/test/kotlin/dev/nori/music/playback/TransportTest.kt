package dev.nori.music.playback

import dev.nori.music.ffi.model.Focus
import dev.nori.music.ffi.model.FocusAct
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TransportTest {
    @Test fun focusActsMoveWhatIsWanted() {
        // was, focus, act -> now.
        val table = listOf(
            Triple(Want.PLAYING, Focus.LOST, FocusAct.PAUSE) to Want.PAUSED,
            Triple(Want.PLAYING, Focus.LOST_TRANSIENT, FocusAct.PAUSE) to Want.HELD_BY_CALL,
            Triple(Want.HELD_BY_CALL, Focus.LOST, FocusAct.PAUSE) to Want.PAUSED,
            Triple(Want.HELD_BY_CALL, Focus.GAINED, FocusAct.RESUME) to Want.PLAYING,
            Triple(Want.PAUSED, Focus.GAINED, FocusAct.RESUME) to Want.PAUSED,
            Triple(Want.PAUSED, Focus.LOST, FocusAct.PAUSE) to Want.PAUSED,
            Triple(Want.PLAYING, Focus.DUCK, FocusAct.DUCK) to Want.PLAYING,
            Triple(Want.PLAYING, Focus.LOST, FocusAct.KEEP_PLAYING) to Want.PLAYING,
        )
        for ((given, now) in table) {
            val (was, focus, act) = given
            assertEquals("$was $focus $act", now, was.afterFocus(focus, act))
        }
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
