package dev.nori.music.app.vm

import dev.nori.music.ffi.library.SearchItem
import dev.nori.music.ffi.library.SearchKind
import dev.nori.music.ffi.library.SearchLayout
import dev.nori.music.ffi.library.SearchScope
import dev.nori.music.ffi.library.SearchSection
import dev.nori.music.ffi.library.SearchShown
import dev.nori.music.ffi.library.SearchView
import org.junit.Assert.assertEquals
import org.junit.Test

/** The field is the finger's: an answer to an earlier keystroke arriving late must not put the old text back. */
class SearchUiTest {
    @Test
    fun aLateAnswerLeavesTheFieldAsTypedAndBringsItsPage() {
        val typed = SearchUi(query = "aph")
        val layout = SearchLayout(SearchItem(SearchKind.ARTISTS, 0u), listOf(SearchSection(SearchKind.SONGS, listOf(0u, 1u))))
        val answer = SearchView("ap", "ap", SearchShown.Blank, SearchScope.EVERYTHING, false, listOf(SearchKind.SONGS, SearchKind.ARTISTS), SearchKind.SONGS, layout)
        val ui = typed.with(answer)
        assertEquals("aph", ui.query)
        assertEquals(listOf(SearchKind.SONGS, SearchKind.ARTISTS), ui.kinds)
        assertEquals(SearchKind.SONGS, ui.kind)
        assertEquals(layout, ui.layout)
    }
}
