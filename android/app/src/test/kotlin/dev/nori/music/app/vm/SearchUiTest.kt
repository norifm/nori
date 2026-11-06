package dev.nori.music.app.vm

import dev.nori.music.ffi.library.SearchScope
import dev.nori.music.ffi.library.SearchView
import org.junit.Assert.assertEquals
import org.junit.Test

/** The field is the finger's: an answer to an earlier keystroke arriving late must not put the old text back. */
class SearchUiTest {
    @Test
    fun aLateAnswerLeavesTheFieldAsTyped() {
        val typed = SearchUi(query = "aph")
        val answer = SearchView("ap", "ap", null, false, false, null, SearchScope.EVERYTHING, false, false)
        assertEquals("aph", typed.with(answer).query)
    }
}
