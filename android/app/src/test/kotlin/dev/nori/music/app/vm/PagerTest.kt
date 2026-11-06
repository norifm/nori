package dev.nori.music.app.vm

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Test

class PagerTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)

    @Test fun aFailedPageIsAskedForAgain() {
        // Both answered at once, as a stored page is.
        val pages = Pager(scope)
        var asked = 0
        pages.next { asked++; error("offline") }
        pages.next { asked++; true }
        assertEquals(2, asked)
    }

    @Test fun nothingIsAskedAfterTheLastPage() {
        val pages = Pager(scope)
        var asked = 0
        pages.next { asked++; false }
        pages.next { asked++; true }
        assertEquals(1, asked)
        // Read again from the first page after a reset.
        pages.reset()
        pages.next { asked++; false }
        pages.next { asked++; true }
        assertEquals(2, asked)
    }

    @Test fun onePageAtATime() {
        val pages = Pager(scope)
        val answer = CompletableDeferred<Boolean>()
        var asked = 0
        pages.next { asked++; answer.await() }
        pages.next { asked++; true }
        assertEquals(1, asked)
        answer.complete(true)
        pages.next { asked++; true }
        assertEquals(2, asked)
    }

    @Test fun aResetDropsThePageOnItsWay() {
        val pages = Pager(scope)
        val late = CompletableDeferred<Boolean>()
        var asked = 0
        pages.next { late.await() }
        pages.reset()
        pages.next { asked++; CompletableDeferred<Boolean>().await() }
        // The dropped page's answer, had it come, ends nothing: the new one is still on its way.
        late.complete(false)
        pages.next { asked++; true }
        assertEquals(1, asked)
        pages.end()
        pages.next { asked++; true }
        assertEquals(1, asked)
    }
}
