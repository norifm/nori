package dev.nori.music.app.vm

import dev.nori.music.ffi.sync.Confirmation
import dev.nori.music.ffi.sync.MergeOffer
import dev.nori.music.ffi.sync.PairingRefused
import dev.nori.music.ffi.sync.PairingView
import dev.nori.music.ffi.sync.Refusal
import dev.nori.music.ffi.sync.Seen
import dev.nori.music.ffi.sync.SyncEvent
import dev.nori.music.ffi.sync.SyncState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the pairing and Your devices screens make of where the core's sync stands. */
class SyncScreensTest {
    private val code = PairingView.Code("nori://pair#v=1", listOf("acorn", "maple", "pine"))

    @Test fun `the new device keeps its code after a key it waits on`() {
        for (reason in listOf(PairingRefused.Tampered, PairingRefused.Spent)) {
            assertEquals(PairStep.Code(code.link, code.words, reason), pairStep(PairingView.Refused(reason), PairRole.SHOWS, code))
        }
    }

    @Test fun `a code that ran out offers a new one only on the device that showed it`() {
        assertEquals(PairStep.RanOut, pairStep(PairingView.Refused(PairingRefused.Expired), PairRole.SHOWS, code))
        assertEquals(PairStep.Refused(PairingRefused.Expired), pairStep(PairingView.Refused(PairingRefused.Expired), PairRole.TAKES, null))
    }

    @Test fun `every other step is the core's`() {
        val c = Confirmation("MacBook", 3u, 1u, listOf("a", "b", "c"), true)
        val table = listOf(
            PairingView.Idle to PairStep.Idle,
            PairingView.Starting to PairStep.Starting,
            code to PairStep.Code(code.link, code.words),
            PairingView.Confirm(c) to PairStep.Confirm(c),
            PairingView.Paired to PairStep.Paired,
            PairingView.Sent to PairStep.Sent,
            PairingView.Refused(PairingRefused.Gone) to PairStep.Refused(PairingRefused.Gone),
            PairingView.Refused(PairingRefused.Server(Refusal.DAILY_WALL)) to PairStep.Refused(PairingRefused.Server(Refusal.DAILY_WALL)),
        )
        for ((view, step) in table) assertEquals(step, pairStep(view, PairRole.TAKES, code))
        // A refusal the shown code cannot outlive stays a refusal on the new device too.
        assertEquals(PairStep.Refused(PairingRefused.SyncOff), pairStep(PairingView.Refused(PairingRefused.SyncOff), PairRole.SHOWS, code))
    }

    @Test fun `a screen opened later takes up the pairing under way`() {
        val c = Confirmation("MacBook", 3u, null, null, false)
        assertEquals(PairRole.SHOWS, pairRole(code))
        assertEquals(PairRole.TAKES, pairRole(PairingView.Confirm(c)))
        for (view in listOf(PairingView.Idle, PairingView.Starting, PairingView.Sent, PairingView.Paired, PairingView.Refused(PairingRefused.Gone))) {
            assertEquals(null, pairRole(view))
        }
    }

    @Test fun `only a device in no group shows its code at once`() {
        val alone = SyncView(read = true, on = true, state = SyncState.Idle)
        assertTrue(showsCodeAtOnce(alone))
        assertFalse("paired", showsCodeAtOnce(alone.copy(state = SyncState.Connected)))
        assertFalse("paired and offline", showsCodeAtOnce(alone.copy(state = SyncState.Offline(0))))
        // Off, the core cannot tell whether this device is paired, and pairing is refused anyway.
        assertFalse("sync off", showsCodeAtOnce(alone.copy(on = false)))
        assertTrue(takesCode(profiles = 1))
        assertFalse(takesCode(profiles = 0))
    }

    @Test fun `devices in the room come first, then the most recently seen`() {
        val old = Seen("1", "iPad", 1_000, false)
        val recent = Seen("2", "MacBook", 5_000, false)
        val here = Seen("3", "PC", 2_000, true)
        assertEquals(listOf(here, recent, old), deviceOrder(listOf(old, recent, here)))
    }

    @Test fun `a merge keeps the profile in use`() {
        val offer = MergeOffer("home", "public")
        assertEquals("public" to "home", mergeKeeps(offer, active = "public"))
        assertEquals("home" to "public", mergeKeeps(offer, active = "home"))
        assertEquals("home" to "public", mergeKeeps(offer, active = "other"))
    }

    @Test fun `only a profile that syncs asks about the other devices`() {
        val paired = SyncView(state = SyncState.Connected, synced = setOf("a"))
        assertTrue(syncOffAsks("a", paired))
        assertFalse(syncOffAsks("b", paired))
        assertFalse("not paired", syncOffAsks("a", paired.copy(state = SyncState.Idle)))
        // Offline is still paired: the profile syncs once the connection is back.
        assertTrue(syncOffAsks("a", paired.copy(state = SyncState.Offline(0))))
    }

    @Test fun `install is offered for a plugin with a repo to fetch it from`() {
        val fromRepo = dev.nori.music.ffi.sync.NeededPlugin("yt", "https://repo.example", "k")
        val fromFile = dev.nori.music.ffi.sync.NeededPlugin("mine", null, null)
        assertTrue(installOffered(listOf(fromFile, fromRepo)))
        assertFalse("nothing to fetch", installOffered(listOf(fromFile)))
        assertFalse(installOffered(emptyList()))
    }

    @Test fun `a refusal shows where sync stands, the rest as the app's notice`() {
        assertTrue(told(SyncEvent.NewDevice("MacBook")))
        assertTrue(told(SyncEvent.UpdateOther("MacBook")))
        assertTrue(told(SyncEvent.UpdateThis))
        assertTrue(told(SyncEvent.DeviceLeft("MacBook")))
        assertFalse(told(SyncEvent.Refused(Refusal.LIMIT)))
    }
}
