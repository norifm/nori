package dev.nori.music.data

import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.model.ItemKey
import dev.nori.music.ffi.model.Song

/** The song's source and id: what the core keeps a queued or downloaded song, or its mark, by. */
val Song.key: ItemKey get() = ItemKey(source, id)

/** The album's source and id: what the core keeps its mark by. */
val Album.key: ItemKey get() = ItemKey(source, id)

/** The artist's source and id: what the core keeps its mark by. */
val Artist.key: ItemKey get() = ItemKey(source, id)
