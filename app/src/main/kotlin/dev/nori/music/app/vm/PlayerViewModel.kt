package dev.nori.music.app.vm

import android.app.Application
import dev.nori.music.ffi.model.Lyrics
import dev.nori.music.data.FoundLyrics
import dev.nori.music.data.followSong
import dev.nori.music.ffi.settings.LyricsOrigin
import dev.nori.music.net.said
import dev.nori.music.playback.PlayerState
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.stateIn
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.launch

class PlayerViewModel(app: Application) : NoriViewModel(app) {
    private val player = nori.player
    val state: StateFlow<PlayerState> = player.state
    /** Where a seek asked to go, while it is still being watched into place; the seek bar holds this. */
    val pendingSeek: StateFlow<Long?> = player.pendingSeek

    /** A mix is being heard (see [dev.nori.music.playback.PlayerConnection.mixing]): the seek row says so. */
    val mixing: StateFlow<Boolean> = player.mixing

    private val _pluginStatus = kotlinx.coroutines.flow.MutableStateFlow<List<String>>(emptyList())
    /** The lines plugins say under the output row, as last asked. */
    val pluginStatus: StateFlow<List<String>> = _pluginStatus

    /** Asks the plugins for their lines under the output row: when the player opens and when the song changes, never on a timer. */
    fun askPluginStatus() = viewModelScope.launch { _pluginStatus.value = runCatching { nori.plugins.playerStatus() }.getOrDefault(emptyList()) }

    /** Just the id, so a list can highlight its playing row without observing the whole player. */
    val currentId: StateFlow<String?> = state.map { it.current?.id }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * Whether the queue is the one [page] started (nori-queue `playlist_from`): its Play or Shuffle, or a
     * row of its list, and any edit since. Asked when [PlayerState.origin] moves, not on every event.
     */
    fun playsFrom(page: dev.nori.music.ffi.library.PageQueue): Boolean = nori.session.playlistFrom(page)

    /** Whether previous restarts the song at [positionMs] (the player's own rule, `queue_previous_restarts`). */
    fun previousRestarts(positionMs: Long, hasPrevious: Boolean): Boolean = nori.session.queuePreviousRestarts(positionMs, hasPrevious)

    /** The queue's rows as the core orders them (`Core::queue_rows`); a mirrored device's are in play order already. */
    fun queueRows(len: Int, shuffle: Boolean, shown: Int): dev.nori.music.ffi.QueueRows =
        if (state.value.playingOn != null) dev.nori.music.ffi.mirroredQueueRows(len.toUInt(), shuffle, shown)
        else nori.core.queueRows(len.toUInt(), shuffle, shown)

    /** A jam as the player's strip says it: who hosts it ([host] null: this phone) and how many listen. */
    data class JamStripState(val host: String?, val listening: Int, val watching: Boolean)

    /**
     * The jam this phone hosts or is a guest in, for the "Jam · 2 listening" strips (the host not counted
     * among the listeners); null without one.
     */
    val jamStrip: StateFlow<JamStripState?> = nori.remotes.jam
        .map { j ->
            j?.let { v ->
                val host = v.members.firstOrNull { it.role == dev.nori.music.ffi.remote.Role.HOST }
                JamStripState(if (v.hosting) null else host?.name.orEmpty(), v.members.count { it.role != dev.nori.music.ffi.remote.Role.HOST }, !v.hosting && v.listening == dev.nori.music.ffi.Listening.WATCHING)
            }
        }
        .distinctUntilChanged().stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** Who asked for each song of the jam, by song id: the queue's "added by" chips. */
    val jamAdded: StateFlow<Map<String, String>> = nori.remotes.jamAdded

    /** Just the play/pause flag, for the same reason: the marked row's bars move only while it sounds. */
    val advancing: StateFlow<Boolean> = state.map { it.advancing }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    private val lyricsKept = dev.nori.music.data.SongAnswers<Load<FoundLyrics>>()

    /** The lyrics shown last retimed, and the song they are for. */
    private val lyricsRetimed = kotlinx.coroutines.flow.MutableStateFlow<dev.nori.music.data.ForSong<FoundLyrics>?>(null)

    /**
     * Lyrics of whatever is heard (the page's song, [PlayerState.current]); fetched only while a lyrics
     * view is collecting. Each song starts from Loading, so the view shows its loader and then the new
     * words, instead of holding the last song's lyrics on screen while the next ones are fetched.
     *
     * Every answer names the song it is for, and the view shows it only under that song
     * ([dev.nori.music.data.ForSong.of]): the last answer outlives the collecting, and a panel opened
     * again after the song changed must not be handed the old song's words. Because of that tag the
     * last answer is kept when the panel stops watching: under the same song it is the right one, and
     * the panel opened again shows it at once instead of the loader and a lookup all over again.
     *
     * The answers of the last few songs are kept too ([lyricsKept]): a song come back to - the panel
     * reopened after the upstream stopped, or the song heard for a moment again around a skip - starts
     * from its words, and is not looked up again once its lookup had finished. The same words read again
     * are equal and not handed on as new ones, which faded them out and in and reset their clock.
     */
    val lyrics: StateFlow<dev.nori.music.data.ForSong<Load<FoundLyrics>>> = kotlinx.coroutines.flow.combine(lyricsFound(), lyricsRetimed) { found, retimed ->
        // The words on screen, given the timing the listener just set: the same words only.
        val words = (found.value as? Load.Ready)?.data
        if (retimed != null && retimed.songId == found.songId && words?.source == retimed.value.source && words.lyrics.lines == retimed.value.lyrics.lines) {
            dev.nori.music.data.ForSong(found.songId, Load.Ready(retimed.value))
        } else found
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), dev.nori.music.data.ForSong(null, Load.Loading))

    /**
     * The lyrics shown for [songId] a step sooner ([dir] > 0) or later (< 0), or back to the core's
     * timing (0); the core keeps it for those words.
     */
    fun nudgeLyrics(songId: String, dir: Int) {
        val found = (lyrics.value.of(songId, Load.Loading) as? Load.Ready)?.data ?: return
        viewModelScope.launch {
            val retimed = nori.library.lyricsNudge(songId, found, dir)
            lyricsKept.replace(songId, Load.Ready(retimed))
            lyricsRetimed.value = dev.nori.music.data.ForSong(songId, retimed)
        }
    }

    /** The lyrics view came in sight ([open]) or left it: the core warms the songs ahead's lyrics only while one is. */
    fun lyricsView(open: Boolean) {
        nori.client.lyricsView(open)
        if (open) viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { runCatching { nori.client.lyricsPrefetch() } }
    }

    private fun lyricsFound() = state.map { it.current }
        .followSong(
            id = { it.id },
            loading = Load.Loading,
            none = { Load.Ready(FoundLyrics(dev.nori.music.ffi.library.lyricsNone(), LyricsOrigin.SERVER)) },
            failed = { Load.Failed(it.said ?: it.javaClass.simpleName) },
            answers = lyricsKept,
            keep = { it is Load.Ready && it.data.lyrics.lines.isNotEmpty() },
        ) { song -> (dev.nori.music.app.testLyrics(song) ?: nori.library.lyricsFor(song)).map<FoundLyrics, Load<FoundLyrics>> { Load.Ready(it) } }

    /**
     * The sleeve a plugin gives the song playing (the artwork slot), or null for nori's own cover. Asked
     * only while the player collects this, which is while it is open, once per song; a new song starts
     * from null, so the last one's loop steps back at once rather than playing on under the new cover.
     */
    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    val artwork: StateFlow<dev.nori.music.ffi.plugins.Artwork?> = state.map { it.current }.distinctUntilChanged { a, b -> a?.id == b?.id }
        .flatMapLatest { song ->
            kotlinx.coroutines.flow.flow<dev.nori.music.ffi.plugins.Artwork?> {
                emit(null)
                if (song != null) emit(runCatching { nori.plugins.artwork(song) }.getOrNull())
            }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * The player behind a looping sleeve. The object is only made when the screen first hands it a
     * surface, and its ExoPlayer only when it first plays; without a loop neither ever is.
     */
    private val motionLazy = lazy {
        nori.motionPlayer { status ->
            val shown = artwork.value ?: return@motionPlayer
            val song = state.value.current ?: return@motionPlayer
            viewModelScope.launch { nori.plugins.artworkFailed(shown, song, status) }
        }
    }
    private val motion by motionLazy

    /** The loop whose first frame is on its surface; the screen fades it in after this. */
    val motionReady: StateFlow<String?> get() = motion.ready

    /** The loop's surface arrived on screen ([shown]) or left it. */
    fun motionView(view: android.view.TextureView, shown: Boolean) {
        if (shown) motion.show(view) else if (motionLazy.isInitialized()) motion.hide(view)
    }

    fun motionPlay(url: String) = motion.play(url)

    /** Where the loop is (ms), -1 before one was made; for the test bridge. */
    val motionAtMs: Long get() = if (motionLazy.isInitialized()) motion.position else -1

    fun motionPause() {
        if (motionLazy.isInitialized()) motion.pause()
    }

    fun motionRelease() {
        if (motionLazy.isInitialized()) motion.release()
    }

    override fun onCleared() {
        motionRelease()
        super.onCleared()
    }

    private val _coversNear = kotlinx.coroutines.flow.MutableStateFlow<List<String>>(emptyList())
    /** The playing song's cover and those a skip either way lands on, for the bar to work their colours out ahead. */
    val coversNear: StateFlow<List<String>> = _coversNear

    init {
        // The artwork either side of what is playing, fetched before it is asked for, so a skip does not
        // show an empty sleeve while the server renders the next cover. Same sizes and requests as the player and the rows, so a warmed cover is a cache hit.
        // How far ahead is the core's (`Client::covers_ahead`, as many as songs are prepared ahead); already-cached ones cost a memory lookup
        // and nothing else.
        // Which positions, in which order, is the core's (`covers_around`, over the queue it keeps, which
        // also names the covers either side whose colours the bar works out ahead); it is asked once, only
        // when the queue, the playing song or a skip's target moves, not on every play/pause or buffering change.
        viewModelScope.launch {
            state
                .distinctUntilChanged { a, b -> a.queue === b.queue && a.index == b.index && a.previousIndex == b.previousIndex && a.nextIndex == b.nextIndex }
                .map { s -> nori.core.coversAround(s.index, s.previousIndex, s.nextIndex, nori.client.coversAhead()) }
                .distinctUntilChanged()
                .collect { around ->
                    _coversNear.value = around.near
                    // The covers a skip lands on (either side of the song playing) into memory, decoded: a
                    // skip lands on a picture that is already there. The ones further out onto the disk
                    // only, as the server sent them: decoded they held a megabyte or two each, ten either
                    // way, and a skip that far is a disk read away.
                    val loader = dev.nori.music.data.CoverLoader.get(getApplication<Application>())
                    for (want in around.wants) {
                        val url = nori.library.coverUrl(want.id, want.size.toInt()) ?: continue
                        if (want.id in around.near) loader.prefetch(url) else loader.warm(url)
                    }
                }
        }
    }

    /** Pull, do not push: the UI reads this on its own clock while the seek bar is on screen. */
    val positionMs: Long get() = player.positionMs
    /** The player's own place, unshaped: for the test bridge's traces. */
    val playerPositionMs: Long get() = player.playerPositionMs

    /** [positionMs] while the song playing is still [songId]; null once the player has left it. */
    fun positionIn(songId: String?): Long? = dev.nori.music.data.playheadFor(songId, state.value.current?.id) { player.positionMs }

    fun connect() = player.connect()
    fun toggle() = player.toggle()
    fun next() = player.next()
    fun previous() = player.previous()
    fun previousItem() = player.previousItem()
    fun seekTo(ms: Long) = player.seekTo(ms)
    fun skipTo(index: Int) = player.skipTo(index)
    fun remove(index: Int) = player.remove(index)
    fun restore(song: dev.nori.music.ffi.model.Song, index: Int) = player.restore(song, index)
    fun move(from: Int, to: Int) = player.move(from, to)
    fun toggleShuffle() = player.setShuffle(!state.value.shuffle)
    fun cycleRepeat() = player.cycleRepeat()
    fun sleep(minutes: Int, endOfTrack: Boolean = false, songs: Int = 0) = player.sleep(minutes, endOfTrack, songs)
}
