package dev.nori.music.data

import dev.nori.music.ffi.lyrics.LyricsNow
import dev.nori.music.ffi.model.ItemKey

/** The song [LyricsNow] is for; null when nothing plays. */
val LyricsNow.songKey: ItemKey?
    get() = when (this) {
        LyricsNow.NoSong -> null
        is LyricsNow.Looking -> song
        is LyricsNow.Shown -> song
        is LyricsNow.Failed -> song
    }

/** The words shown, when they are song [key]'s and have lines. */
fun LyricsNow.wordsOf(key: ItemKey?): FoundLyrics? =
    (this as? LyricsNow.Shown)?.takeIf { it.song == key && it.pick.lyrics.lines.isNotEmpty() }?.let { FoundLyrics(it.pick.lyrics, it.pick.origin) }

/**
 * The playhead read by [position] when the song playing ([playing]) is still [song], else null. What
 * is drawn for one song must not follow the next one's playhead: a skip moves the player before the page
 * that shows the old song's words has changed, and read then the new song's first second put those words
 * back before their first line, which scrolled them to the top in one frame as they began to fade.
 */
inline fun playheadFor(song: ItemKey?, playing: ItemKey?, position: () -> Long): Long? =
    if (song != null && song == playing) position() else null
