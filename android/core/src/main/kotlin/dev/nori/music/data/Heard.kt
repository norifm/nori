package dev.nori.music.data

import dev.nori.music.ffi.lyrics.LyricsNow

/** The song [LyricsNow] is for; null when nothing plays. */
val LyricsNow.songId: String?
    get() = when (this) {
        LyricsNow.NoSong -> null
        is LyricsNow.Looking -> song
        is LyricsNow.Shown -> song
        is LyricsNow.Failed -> song
    }

/** The words shown, when they are song [id]'s and have lines. */
fun LyricsNow.wordsOf(id: String?): FoundLyrics? =
    (this as? LyricsNow.Shown)?.takeIf { it.song == id && it.pick.lyrics.lines.isNotEmpty() }?.let { FoundLyrics(it.pick.lyrics, it.pick.origin) }

/**
 * The playhead read by [position] when the song playing ([playing]) is still [songId], else null. What
 * is drawn for one song must not follow the next one's playhead: a skip moves the player before the page
 * that shows the old song's words has changed, and read then the new song's first second put those words
 * back before their first line, which scrolled them to the top in one frame as they began to fade.
 */
inline fun playheadFor(songId: String?, playing: String?, position: () -> Long): Long? =
    if (songId != null && songId == playing) position() else null
