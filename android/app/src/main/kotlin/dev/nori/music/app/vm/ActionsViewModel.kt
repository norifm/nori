package dev.nori.music.app.vm

import dev.nori.music.ffi.Picked
import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.data.StarKind
import dev.nori.music.downloads.DownloadState
import dev.nori.music.downloads.DownloadMark
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.transfers.DownloadQueueCounts
import dev.nori.music.ffi.transfers.DownloadSections
import dev.nori.music.net.said
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.stateIn
import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.model.OriginKind
import dev.nori.music.ffi.model.PageOrigin
import dev.nori.music.ffi.model.Playlist
import dev.nori.music.ffi.library.PlaylistChoice
import dev.nori.music.ffi.model.Song
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import dev.nori.music.ffi.settings.SwipeAction
import kotlinx.coroutines.flow.last
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import dev.nori.music.ffi.queue.ShufflePlan
import dev.nori.music.ffi.queue.TapPlan
import dev.nori.music.ffi.queue.shufflePlan
import dev.nori.music.app.ui.say
import dev.nori.music.settings.server

/** Everything that can be done to a song, album or playlist from any screen. One instance per activity. */
class ActionsViewModel(app: Application) : NoriViewModel(app) {
    private val _messages = Channel<String>(Channel.CONFLATED)
    /** One-line confirmations and failures, for a snackbar or whatever the UI uses. */
    val messages = _messages.receiveAsFlow()
    fun tell(words: String) { _messages.trySend(words) }
    /** This session's star changes, so every heart on screen can prefer them over its snapshot. */
    val starMarks: StateFlow<dev.nori.music.ffi.library.StarMarks> = nori.library.starMarks
    val downloads: StateFlow<DownloadState> = nori.downloads.state

    // A process started in the background could not restart the download service; with a screen up it can.
    init { nori.downloads.resume() }

    // What plugins show while they run, said as the app's own messages are.
    init { viewModelScope.launch { nori.plugins.notices.collect { _messages.trySend(it.line) } } }

    // Plugins off at this start (safe mode), said once a process.
    init { viewModelScope.launch { nori.plugins.takeSafeStart()?.let { (safe, names) -> _messages.trySend(say.pluginsSafe(safe, names)) } } }

    /** The actions plugins add to each kind of item's menu, read before any menu opens. */
    val pluginMenus = nori.plugins.menus

    /** Runs a plugin's menu action on [items]. */
    fun runPluginMenu(entry: dev.nori.music.ffi.plugins.MenuEntry, items: dev.nori.music.ffi.plugins.Items) {
        viewModelScope.launch { runCatching { nori.plugins.runMenu(entry, items) }.onFailure { dev.nori.music.NoriLog.w("plugin menu ${entry.plugin}: ${it.message}") } }
    }

    /** The lines plugins add to [song]'s info; none when they could not be asked. */
    suspend fun pluginSongInfo(song: Song): List<dev.nori.music.ffi.plugins.InfoGroup> = runCatching { nori.plugins.songInfo(song) }.getOrDefault(emptyList())

    // Plugins updated from their repos as the app came to the front, said quietly.
    init { viewModelScope.launch { nori.plugins.updated.collect { done -> done.forEach { (name, u) -> _messages.trySend(say.pluginUpdated(name, u.to)) } } } }

    internal fun attempt(done: String?, block: suspend () -> Unit) = viewModelScope.launch {
        try { block(); done?.let { _messages.send(it) } } catch (e: Exception) { _messages.send(e.said ?: say.saidFailed) }
    }

    // ---- selection mode: long-press a song anywhere, then act on the whole selection ----

    private val picked = Selection<Song> { it.id }
    val selection: StateFlow<List<Song>> = picked.items
    /** A jam guest has nothing to do with a selection: a long press selects nothing. */
    fun toggleSelected(song: Song) { if (!nori.rules.asks) picked.toggle(song) }

    /** A jam guest's profile (the core's `ProfileRules`): what it plays or queues goes to the jam, by its role. */
    private val guest: Boolean get() = nori.rules.asks

    /** What a jam guest's pick did (the core's `Remote::jam_play`, `jam_add`): asked of the host, or sent to its queue ([sent]). */
    private fun picked(p: Picked, sent: String?) {
        when (p) {
            is Picked.Asked -> _messages.trySend(say.jamAsked(p.title))
            Picked.Sent -> sent?.let { _messages.trySend(it) }
            Picked.Nowhere -> {}
        }
    }

    /** Plays [songs] from [index], or shuffled; a jam guest's goes to the jam (`Remote::jam_play`). */
    private fun start(songs: List<Song>, index: Int = 0, shuffle: Boolean = false, from: PageOrigin? = null) {
        if (guest) nori.remotes.ask({ it.jamPlay(songs, index.toUInt(), shuffle) }) { picked(it, null) }
        else nori.player.play(songs, index, shuffle = shuffle, from = from)
    }
    fun clearSelection() = picked.clear()
    /** Back while songs are selected lets go of them and goes no further (see [Selection.back]). */
    fun backFromSelection(): Boolean = picked.back()
    /** The page on screen is now [key] (a back stack entry): a selection does not outlive its page. */
    fun onPage(key: String) = picked.onPage(key)

    /**
     * What a plain tap on row [index] of [songs] does: the core's answer from the settings (`tap_plan`).
     * [from] is the page whose list [songs] is: playing the list from the row is that page's queue, and
     * its Play reads Pause; one song on its own is no page's. A tap on the song [playing] now does not
     * start it again: the list becomes the queue around it and it goes on. True when the player should
     * open on it.
     */
    fun tap(songs: List<Song>, index: Int, from: PageOrigin? = null, playing: Boolean = false): Boolean {
        if (guest) return false.also { start(songs, index) }
        val plan = nori.session.tapPlan(picked.items.value.isNotEmpty())
        when (plan) {
            TapPlan.SELECT -> toggleSelected(songs[index])
            TapPlan.PLAY_LIST -> if (playing) nori.player.keepPlaying(songs, index, from) else nori.player.play(songs, index, from = from)
            TapPlan.PLAY_ONE -> if (!playing) nori.player.play(listOf(songs[index]))
            TapPlan.QUEUE -> enqueue(listOf(songs[index]))
            TapPlan.PLAY_NEXT -> playNext(listOf(songs[index]))
        }
        return playing && (plan == TapPlan.PLAY_LIST || plan == TapPlan.PLAY_ONE)
    }

    /** What swiping a song row right and left does, as set in Settings. */
    val swipes: Pair<SwipeAction, SwipeAction> get() = nori.settings.value.let { it.swipeRight to it.swipeLeft }

    // By id: the core has the artist's albums from reading the page, so they need not be handed back.
    fun playArtist(artistId: String, shuffle: Boolean = false) = attempt(null) {
        start(nori.library.artistSongs(artistId), shuffle = shuffle, from = PageOrigin(OriginKind.ARTIST, artistId))
    }
    fun queueArtist(artistId: String) = attempt(null) { enqueue(nori.library.artistSongs(artistId)) }
    fun downloadArtist(artistId: String) = attempt(null) { download(nori.library.artistSongs(artistId)) }

    /** An album, playlist or artist [CardMenu] opened on, played or queued whole. */
    fun play(c: CardItem) = attempt(null) { start(songsOf(c), from = originOf(c)) }
    fun queue(c: CardItem, next: Boolean) = attempt(null) { songsOf(c).let { if (next) playNext(it) else enqueue(it) } }
    private suspend fun songsOf(c: CardItem): List<Song> = when (c) {
        is CardItem.OfAlbum -> nori.library.albumSongs(c.album.id)
        is CardItem.OfPlaylist -> nori.library.playlistSongs(c.playlist.id)
        is CardItem.OfArtist -> nori.library.artistSongs(c.artist.id)
    }
    private fun originOf(c: CardItem) = when (c) {
        is CardItem.OfAlbum -> PageOrigin(OriginKind.ALBUM, c.album.id)
        is CardItem.OfPlaylist -> PageOrigin(OriginKind.PLAYLIST, c.playlist.id)
        is CardItem.OfArtist -> PageOrigin(OriginKind.ARTIST, c.artist.id)
    }

    /** [songs] from [index]; [from] the page they are the list of, if they are one page's (see [tap]). */
    fun play(songs: List<Song>, index: Int = 0, from: PageOrigin? = null) {
        start(songs, index, from = from)
    }

    /** Spreads artists and albums apart (in the core) unless the user prefers a plain random order. */
    fun shuffle(songs: List<Song>, from: PageOrigin? = null) {
        if (guest) return start(songs, shuffle = true)
        when (val plan = shufflePlan(songs)) {
            ShufflePlan.Empty -> {}
            ShufflePlan.PlayerShuffle -> nori.player.play(songs, shuffle = true, from = from)
            is ShufflePlan.Order -> nori.player.playShuffledOrder(songs, plan.order, from)
        }
    }

    suspend fun excludedFromMixes(song: Song): Boolean = song.id in nori.library.excludedFromMixes()

    fun excludeFromMixes(song: Song, on: Boolean) = attempt(if (on) say.excludedFromMixes else say.backInMixes) { nori.library.excludeFromMixes(song.id, on) }

    /** Creates a server playlist from an M3U file; tracks that are not in the index are reported, not guessed. */
    fun importM3u(name: String, text: String) = attempt(null) {
        val imported = withContext(Dispatchers.IO) { nori.core.m3uImport(text) }
        if (imported.songIds.isNotEmpty()) nori.library.createPlaylist(name, imported.songIds)
        _messages.send(say.m3uImported(imported.songIds.size, imported.entries.toInt(), name))
    }
    /** [songs] added by hand. */
    fun playNext(songs: List<Song>) = add(songs, next = true, say.playingNext)
    fun enqueue(songs: List<Song>) = add(songs, next = false, say.addedToQueue)
    /** A jam guest's goes to the jam (`Remote::jam_add`). */
    private fun add(songs: List<Song>, next: Boolean, done: String) {
        if (guest) return nori.remotes.ask({ it.jamAdd(songs, next) }) { picked(it, done) }
        if (next) nori.player.playNext(songs) else nori.player.enqueue(songs)
        _messages.trySend(done)
    }

    // Each shuffle marks its queue, so its refills go on the same way whatever the autoplay setting says.
    fun shuffleAll() = attempt(null) { start(nori.library.shuffleAll(), from = PageOrigin(OriginKind.SHUFFLE_SONGS, "")) }
    /** Random albums, each whole and in its own order. */
    fun shuffleAlbums() = attempt(null) { start(nori.library.shuffleAlbums(), from = PageOrigin(OriginKind.SHUFFLE_ALBUMS, "")) }

    /** An endless-ish mix seeded from one song. */
    /** A radio from the song playing goes on with it: only the songs after it change. */
    fun startRadio(song: Song) = attempt(null) { nori.library.radio(song).let { if (guest) start(it) else nori.player.keepPlaying(it, 0) } }

    /**
     * octo-fiesta downloads a provider item into the library when it is starred: a song on its own, an album
     * or playlist in full, on the server, without streaming it to the phone.
     */
    fun addToLibrary(id: String, isAlbum: Boolean) = attempt(say.serverDownloading) {
        nori.library.star(if (isAlbum) StarKind.ALBUM else StarKind.SONG, id, true)
    }

    fun star(song: Song, on: Boolean) = attempt(null) {
        if (!nori.remotes.starWherePlaying(song.id, on)) nori.library.star(StarKind.SONG, song.id, on)
    }
    fun starAlbum(id: String, on: Boolean) = attempt(null) { nori.library.star(StarKind.ALBUM, id, on) }
    fun starArtist(id: String, on: Boolean) = attempt(null) { nori.library.star(StarKind.ARTIST, id, on) }

    private val _shares = Channel<String>(Channel.BUFFERED)
    /** Links ready to hand to the system share sheet. */
    val shares = _shares.receiveAsFlow()
    fun share(id: String) = attempt(null) { _shares.send(nori.library.share(id)) }

    private val _mobileDownload = MutableStateFlow<List<Song>?>(null)
    /** Songs asked for on mobile data, waiting for a yes before they download. */
    val mobileDownload: StateFlow<List<Song>?> = _mobileDownload.asStateFlow()

    fun download(songs: List<Song>) {
        viewModelScope.launch {
            if (withContext(Dispatchers.IO) { nori.http.metered }) _mobileDownload.value = songs else downloadNow(songs)
        }
    }

    /** The answer to downloading [mobileDownload] over mobile data. */
    fun downloadOnMobileData(yes: Boolean) {
        val songs = _mobileDownload.value ?: return
        _mobileDownload.value = null
        if (yes) downloadNow(songs)
    }

    private fun downloadNow(songs: List<Song>) {
        nori.downloads.download(songs)
        warmCovers(songs)
        _messages.trySend(say.downloadingSongs(songs.size))
    }

    /**
     * Settings' "Analyse downloaded songs": the downloads with no current analysis are read back one at a time
     * under the download service.
     */
    fun analyseDownloads() = nori.downloads.analyse { n -> _messages.trySend(if (n > 0) say.analysingDownloads(n) else say.nothingToAnalyse) }

    /**
     * Fetches the artwork of songs being downloaded onto the disk; which covers, at which addresses, is
     * the core's (`Core::download_cover_urls`). Not decoded: nothing is drawing them now, and hundreds of
     * them would push the covers on screen out of memory.
     */
    private fun warmCovers(songs: List<Song>) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
        val loader = dev.nori.music.data.CoverLoader.get(getApplication<Application>())
        for (url in nori.core.downloadCoverUrls(songs.mapNotNull { it.coverArt })) loader.warm(url)
    }

    private val _removal = MutableStateFlow<List<String>?>(null)
    /** Downloads asked to be removed, waiting for a yes. */
    val removal: StateFlow<List<String>?> = _removal.asStateFlow()

    /** Gives the downloads back after a yes: the same menu entry that offered them should be able to take them away. */
    fun undownload(songs: List<Song>) { _removal.value = songs.map { it.id } }

    /** The answer to removing [removal]. */
    fun removeAsked(yes: Boolean) {
        val ids = _removal.value ?: return
        _removal.value = null
        if (!yes) return
        removeDownloads(ids)
        _messages.trySend(say.downloadsRemoved(ids.size))
    }
    fun removeDownloads(ids: List<String>) = nori.downloads.remove(ids)

    /** What each download this session touched is doing; see [dev.nori.music.downloads.Downloads.marks]. */
    val downloadMarks: StateFlow<Map<String, DownloadMark>> = nori.downloads.marks

    /** The table or any song's phase changed. */
    private val phaseChanges = combine(nori.downloads.state, nori.downloads.marks) { _, _ -> }

    /**
     * [read] from the core, asked again whenever [changes] emits, and only while watched. Null
     * until the first answer, which is not the same as empty.
     */
    private fun <T : Any> readOnDownloadChange(changes: Flow<*>, read: () -> T): StateFlow<T?> =
        // A song finishing changes the table and its mark together: the changes are only the signal, and
        // the ones that arrive while it is being read are asked for once, not once each.
        changes
            .conflate()
            .map { runCatching(read).getOrNull() }
            .flowOn(kotlinx.coroutines.Dispatchers.IO)
            .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * The downloads screen's lists: downloading, waiting (in the order they will run), failed, and
     * finished this session, split in the core.
     */
    val downloadSections: StateFlow<DownloadSections?> = readOnDownloadChange(phaseChanges) { nori.core.downloadSections() }

    /** How many of [downloadSections]' songs are on their way and how many failed, without the songs. */
    val downloadQueueCounts: StateFlow<DownloadQueueCounts?> = readOnDownloadChange(phaseChanges) { nori.core.downloadQueueCounts() }

    /** Finished albums and unfiled songs, read only while the downloads page is watching; only the table changes them, not a phase. */
    val downloadedLibrary: StateFlow<dev.nori.music.ffi.library.DownloadedLibrary?> =
        readOnDownloadChange(nori.downloads.state) { nori.core.downloadedLibrary() }

    fun retryDownloads(songs: List<Song>) = nori.downloads.retry(songs)
    fun cancelDownloads(songs: List<Song>) = nori.downloads.cancel(songs.map { it.id })
    fun cancelAllDownloads() = nori.downloads.cancelAll()

    /** The server's answer when the stored one is stale: the first is the stored one, which a playlist just made is not in yet. */
    suspend fun playlists(): List<Playlist> = runCatching { nori.library.playlists().last() }.getOrDefault(emptyList())
    /** Where [songs] can be added, each playlist with what it already has of them; empty when they cannot be read. */
    suspend fun playlistChoices(songs: List<Song>): List<PlaylistChoice> = runCatching { nori.library.playlistChoices(songs.map { it.id }) }.getOrDefault(emptyList())
    /** Adds what [c] lacks of the songs it was offered for. */
    fun addToPlaylist(c: PlaylistChoice) = attempt(say.addedToPlaylist(c.playlist.name)) { nori.library.addToPlaylist(c.playlist.id, c.adds) }
    /** Keeps a mix as a playlist on the account, named by the core (`saved_mix_name`) so a second save does not repeat a name. */
    /** Whether [p] is the account's own, to change or delete (the core's `playlist_editable`). */
    fun editable(p: Playlist): Boolean = dev.nori.music.ffi.library.playlistEditable(p, nori.settings.value.server?.user.orEmpty(), nori.rules.account)

    fun deletePlaylist(id: String) = viewModelScope.launch { runCatching { nori.library.deletePlaylist(id) } }

    fun saveAsPlaylist(title: String, songs: List<Song>) = viewModelScope.launch {
        try {
            val name = dev.nori.music.ffi.library.savedMixName(title, playlists().map { it.name })
            nori.library.createPlaylist(name, songs.map { it.id })
            _messages.send(say.playlistCreated(name))
        } catch (e: Exception) { _messages.send(e.said ?: say.saidFailed) }
    }
    fun addToNewPlaylist(name: String, songs: List<Song>) = attempt(say.playlistCreated(name)) { nori.library.createPlaylist(name, songs.map { it.id }) }
}

/** An album's, playlist's or artist's card or row, which a long press opens a menu on. */
sealed interface CardItem {
    data class OfAlbum(val album: Album) : CardItem
    data class OfPlaylist(val playlist: Playlist) : CardItem
    data class OfArtist(val artist: Artist) : CardItem
}
