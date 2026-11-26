package dev.nori.music.app.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

/** Words name things by what people call them; an id (a route's, a server's) is never shown. */
class WordsTest {
    private val say = listOf(File("src/main/kotlin/dev/nori/music/app/ui/Say.kt"), File("android/app/src/main/kotlin/dev/nori/music/app/ui/Say.kt")).first { it.isFile }

    @Test fun `no words take an id`() {
        val takes = Regex("""fun (\w+)\([^)]*\b(id|\w+Id)\s*:\s*String""")
        val found = say.readLines().mapNotNull { takes.find(it)?.groupValues?.get(1) }
        assertEquals(emptyList<String>(), found)
    }
}
