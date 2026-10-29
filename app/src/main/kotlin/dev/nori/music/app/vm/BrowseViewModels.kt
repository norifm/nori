package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.favoritesInOrder
import dev.nori.music.ffi.library.AlbumSort
import dev.nori.music.ffi.library.albumSortKept
import dev.nori.music.ffi.library.albumSortSaved
import dev.nori.music.ffi.library.albumsExhausted
import dev.nori.music.ffi.library.artistRows
import dev.nori.music.ffi.library.songSortKept
import dev.nori.music.ffi.library.songSortSaved
import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.library.AlbumDetail
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.library.ArtistDetail
import dev.nori.music.ffi.model.ArtistInfo
import dev.nori.music.ffi.model.Genre
import dev.nori.music.ffi.model.Playlist
import dev.nori.music.ffi.library.PlaylistDetail
import dev.nori.music.ffi.model.RadioStation
import dev.nori.music.ffi.model.Song
import dev.nori.music.ffi.library.Starred
import kotlinx.coroutines.Job
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.distinctUntilChanged
import dev.nori.music.ffi.settings.HomeRow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import dev.nori.music.ffi.library.HomeShelf
import dev.nori.music.ffi.library.browsePaging
import dev.nori.music.ffi.library.homePinned
import dev.nori.music.ffi.library.homeRefreshDrops
import dev.nori.music.ffi.library.homeShelves
import dev.nori.music.ffi.library.homeRowsShown

/**
 * One row of the home page. Not every shelf is a shelf of albums: the playlists are playlists and the
 * songs someone plays most are songs, and both were asked for by name. Each kind knows whether it has
 * anything to show, so an empty shelf can be left out without the page having to ask what it holds.
 */
sealed interface Shelf {
    val row: HomeRow
    val isEmpty: Boolean

    data class Albums(override val row: HomeRow, val albums: List<Album>) : Shelf {
        override val isEmpty get() = albums.isEmpty()
    }

    data class Playlists(override val row: HomeRow, val playlists: List<Playlist>) : Shelf {
        override val isEmpty get() = playlists.isEmpty()
    }

    data class Songs(override val row: HomeRow, val songs: List<Song>) : Shelf {
        /** What a queue played from this shelf carries. */
        val origin = dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SHELF, row.name)
        override val isEmpty get() = songs.isEmpty()
    }
}

/** The home page: the library's shelves, or ([feed]) the source's own shelves (`ProfileRules.feed`). */
data class HomeUi(val rows: List<Shelf> = emptyList(), val pinned: List<Playlist> = emptyList(), val feed: List<dev.nori.music.ffi.model.FeedShelf> = emptyList())

@OptIn(ExperimentalCoroutinesApi::class)
class HomeViewModel(app: Application) : NoriViewModel(app) {
    // The app was opened: a good moment to replay whatever was starred, rated or played while offline.
    init { viewModelScope.launch { runCatching { nori.library.flushPending() } } }

    private val _offer = MutableStateFlow<dev.nori.music.ffi.library.QueueOffer?>(null)
    /** The queue another device or app left on the server, offered at the top of the page; null for none. */
    val offer: StateFlow<dev.nori.music.ffi.library.QueueOffer?> = _offer

    init {
        // Music started, or the player came to show the offered song (another device's it controls, found
        // after the page asked): the page lets the offer go.
        viewModelScope.launch { nori.player.state.collect { s -> _offer.value?.let { if (!stands(it, s)) _offer.value = null } } }
    }

    /** Asked each time the page comes into sight; a jam guest's Home is the host's, and offers nothing. */
    fun lookForOffer() {
        if (!nori.rules.account) return
        val asked = nori.player.state.value
        viewModelScope.launch {
            val offer = runCatching { nori.library.queueOffer(asked.playing, asked.current?.id) }.getOrNull()
            _offer.value = offer?.takeIf { stands(it, nori.player.state.value) }
        }
    }

    /** Whether [offer] still stands against what the player [state] shows (the core's `offer_stands`). */
    private fun stands(offer: dev.nori.music.ffi.library.QueueOffer, state: dev.nori.music.playback.PlayerState): Boolean {
        val offered = offer.songs.getOrNull(offer.index.toInt())?.id ?: return false
        return dev.nori.music.ffi.library.offerStands(offered, state.current?.id, state.playing)
    }

    /** Plays the offered queue where it was left. */
    fun takeOffer() = closeOffer()?.let { o ->
        nori.player.play(o.songs, o.index.toInt())
        nori.player.seekTo(o.positionMs.toLong())
    }

    fun dismissOffer() { closeOffer() }

    private fun closeOffer(): dev.nori.music.ffi.library.QueueOffer? = _offer.value?.also {
        _offer.value = null
        nori.library.queueOfferDone(it)
    }

    /**
     * Bumped by [refresh]. Every shelf hangs off it rather than the page hanging off it as a whole, so
     * a re-query replaces each shelf's contents where they are instead of emptying the page first and
     * filling it again - the difference between a refresh and the page being built a second time.
     */
    private val refreshes = MutableStateFlow(0)
    private val _refreshing = MutableStateFlow(false)
    /** True while a manual refresh is running, so the page can show that it is and then stop. */
    val refreshing: StateFlow<Boolean> = _refreshing

    private val _closedToGuests = MutableStateFlow(false)
    /**
     * The jam host's server refused a shelf to this guest (an octo-fiesta from before it shared the
     * library with guests): the page says so instead of standing empty.
     */
    val closedToGuests: StateFlow<Boolean> = _closedToGuests

    private fun failed(e: Throwable) {
        val code = (e as? dev.nori.music.ffi.model.CoreException.Api)?.code ?: (e as? dev.nori.music.ffi.net.NetException.Api)?.code
        if (code == 50 && !nori.rules.account) _closedToGuests.value = true
    }

    /** What each shelf is and where it comes from is the core's (browse.rs); this only makes the requests. */
    private fun source(r: HomeRow, shelf: HomeShelf): kotlinx.coroutines.flow.Flow<Shelf> = when (shelf) {
        is HomeShelf.Albums -> {
            // Re-read on every star change, and the core lays this session's marks over what it reads.
            if (shelf.followsStars) combine(refreshes, nori.library.starsVersion) { _, v -> v }.flatMapLatest { nori.library.shelfAlbums(shelf) }
                .map { Shelf.Albums(r, it) }
                .catch { emit(Shelf.Albums(r, emptyList())) }.onStart { emit(Shelf.Albums(r, emptyList())) }
            else refreshes.flatMapLatest { nori.library.shelfAlbums(shelf) }
                .catch { failed(it); emit(emptyList()) }.onStart { emit(emptyList()) }.map { Shelf.Albums(r, it) }
        }
        is HomeShelf.Playlists -> combine(refreshes, nori.library.playlistsVersion) { _, v -> v }.flatMapLatest { nori.library.playlists() }.map { Shelf.Playlists(r, it.take(shelf.take.toInt())) }
            .catch { emit(Shelf.Playlists(r, emptyList())) }.onStart { emit(Shelf.Playlists(r, emptyList())) }
        // Worth reading again after a refresh, which is the one thing that changes what the index holds.
        is HomeShelf.Songs -> refreshes.flatMapLatest {
            flow { emit(Shelf.Songs(r, runCatching { nori.library.browseSongs(shelf.sort, shelf.descending, false, null, 0, shelf.limit.toInt()) }.onFailure(::failed).getOrDefault(emptyList()))) }
        }.onStart { emit(Shelf.Songs(r, emptyList())) }
        HomeShelf.Pinned -> flowOf(Shelf.Playlists(r, emptyList()))
    }

    /** Only the rows the user kept are requested at all; a hidden shelf costs no request. */
    val ui: StateFlow<Load<HomeUi>> = nori.settings.prefs.map { Triple(it.homeRows, it.pinnedPlaylists, nori.rules) }.distinctUntilChanged().flatMapLatest { (rows, pins, rules) ->
        // A source's own Home is its feed, asked again on a refresh.
        if (rules.feed) return@flatMapLatest refreshes.flatMapLatest { nori.library.homeFeed() }
            .catch { failed(it); emit(emptyList()) }.map { HomeUi(feed = it) }
        val shown = homeRowsShown(rows, rules.account)
        val shelves = homeShelves(shown)
        val pinned = if (HomeShelf.Pinned in shelves && pins.isNotEmpty()) combine(refreshes, nori.library.playlistsVersion) { _, v -> v }.flatMapLatest { nori.library.playlists() }
            .map { all -> withContext(Dispatchers.Default) { homePinned(all, pins) } }.catch { emit(emptyList()) }.onStart { emit(emptyList()) } else flowOf(emptyList())
        combine(combine(shown.zip(shelves, ::source)) { it.toList() }.onStart { emit(emptyList()) }, pinned) { s, p -> HomeUi(s, p) }
    }.asLoad()

    /**
     * The pull at the top of the page. The stored answers behind the shelves go first, so asking again
     * really does reach the server rather than being told the two-minute-old copy is still fresh, and
     * then the offline index is walked the way the settings page walks it. A refresh that fails is
     * still over: the page has whatever it had before, and the spinner must not be left turning.
     */
    fun refresh() {
        if (_refreshing.value) return
        _refreshing.value = true
        viewModelScope.launch {
            try {
                runCatching { nori.library.dropCached(*homeRefreshDrops().toTypedArray()) }
                refreshes.update { it + 1 }
                if (nori.rules.indexed) runCatching { nori.library.sync() }
            } finally {
                _refreshing.value = false
            }
        }
    }
}

/** What a home row's full view holds: entries of one kind, as many as are read so far. */
sealed interface RowEntries {
    data class Albums(val albums: List<Album>) : RowEntries
    data class Playlists(val playlists: List<Playlist>) : RowEntries
    data class Songs(val songs: List<Song>, val origin: dev.nori.music.ffi.model.PageOrigin) : RowEntries
}

/**
 * The bigger view of one home row. A library row is read a page at a time as the core's `home_row_page`
 * lays it out; a source's feed shelf (`feed-<place>-<kind>`) shows the items it already has. This only
 * makes the requests.
 */
class HomeRowViewModel(app: Application) : NoriViewModel(app) {
    private val _entries = MutableStateFlow<RowEntries?>(null)
    val entries: StateFlow<RowEntries?> = _entries

    private val _title = MutableStateFlow<String?>(null)
    /** A feed shelf's own title; a library row is worded by the page. */
    val title: StateFlow<String?> = _title

    private var key: String? = null
    private var row: HomeRow? = null
    private var albums = Grown.empty<Album>()
    private var songs = Grown.empty<Song>()
    private var loading = false
    private var exhausted = false

    fun open(key: String) {
        if (key == this.key) return
        this.key = key
        val feed = key.split('-').takeIf { it.size == 3 && it[0] == "feed" }
        if (feed != null) readFeed(feed[1].toInt(), feed[2]) else { row = HomeRow.valueOf(key); loadMore() }
    }

    private fun readFeed(place: Int, kind: String) {
        viewModelScope.launch {
            val shelf = runCatching { nori.library.homeFeed().first().getOrNull(place) }.getOrNull() ?: return@launch
            _title.value = shelf.title
            _entries.value = when (kind) {
                "songs" -> RowEntries.Songs(shelf.songs, dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SHELF, shelf.title))
                "albums" -> RowEntries.Albums(shelf.albums)
                else -> RowEntries.Playlists(shelf.playlists)
            }
        }
    }

    fun loadMore() {
        val row = row ?: return
        if (loading || exhausted) return
        loading = true
        val offset = when (val e = _entries.value) {
            is RowEntries.Albums -> e.albums.size
            is RowEntries.Songs -> e.songs.size
            is RowEntries.Playlists -> e.playlists.size
            null -> 0
        }
        viewModelScope.launch {
            try {
                val shelf = dev.nori.music.ffi.library.homeRowPage(row, offset.toUInt())
                if (shelf == null) { exhausted = true; return@launch }
                when (shelf) {
                    is HomeShelf.Albums -> {
                        val got = runCatching { nori.library.shelfAlbums(shelf, offset).first() }.getOrDefault(emptyList())
                        albums = albums.from(offset, got)
                        _entries.value = RowEntries.Albums(albums)
                        exhausted = !dev.nori.music.ffi.library.homeRowMore(shelf, got.size.toUInt())
                    }
                    is HomeShelf.Songs -> {
                        val got = runCatching { nori.library.browseSongs(shelf.sort, shelf.descending, false, null, offset, shelf.limit.toInt()) }.getOrDefault(emptyList())
                        songs = songs.from(offset, got)
                        _entries.value = RowEntries.Songs(songs, dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SHELF, row.name))
                        exhausted = !dev.nori.music.ffi.library.homeRowMore(shelf, got.size.toUInt())
                    }
                    is HomeShelf.Playlists -> {
                        val all = runCatching { nori.library.playlists().first() }.getOrDefault(emptyList())
                        _entries.value = RowEntries.Playlists(all.take(shelf.take.toInt()))
                    }
                    HomeShelf.Pinned -> {
                        val all = runCatching { nori.library.playlists().first() }.getOrDefault(emptyList())
                        _entries.value = RowEntries.Playlists(homePinned(all, nori.settings.value.pinnedPlaylists))
                    }
                }
            } finally {
                loading = false
            }
        }
    }
}

/**
 * The album grid: one sort order at a time, pages appended as the list nears its end; with [favorites],
 * the starred albums in that order, all at once. In artist rows, also those albums a row per artist ([rows]).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class AlbumsViewModel(app: Application) : NoriViewModel(app) {
    private val pageSize by lazy { browsePaging().albums.toInt() }
    // The order it was left in, and how that is kept, are the core's (`album_sort_saved`, `album_sort_kept`).
    private val _sort = MutableStateFlow(albumSortSaved(nori.settings.value.listPrefs))
    val sort: StateFlow<AlbumSort> = _sort
    private val _favorites = MutableStateFlow(false)
    val favorites: StateFlow<Boolean> = _favorites
    private val _albums = MutableStateFlow(Grown.empty<Album>())
    val albums: StateFlow<List<Album>> = _albums
    private val _none = MutableStateFlow(false)
    /** The first page came back empty: there are no albums, rather than none read yet. */
    val none: StateFlow<Boolean> = _none
    private val _rows = MutableStateFlow<List<List<Album>>>(emptyList())
    val rows: StateFlow<List<List<Album>>> = _rows
    // Where each of [rows] starts in [albums].
    private var rowStarts: List<Int> = emptyList()
    private var job: Job? = null
    private var exhausted = false

    init { loadMore() }

    fun setSort(s: AlbumSort) {
        if (s == _sort.value) return
        _sort.value = s
        val kept = albumSortKept(s)
        nori.settings.update { it.copy(listPrefs = it.listPrefs + (kept.key to kept.value)) }
        reset()
    }

    fun setFavorites(on: Boolean) { _favorites.value = on; reset() }

    private fun reset() {
        job?.cancel()
        _albums.value = Grown.empty()
        _none.value = false
        _rows.value = emptyList()
        rowStarts = emptyList()
        exhausted = false
        loadMore()
    }

    fun loadMore() {
        if (exhausted || job?.isActive == true) return
        val sort = _sort.value
        if (_favorites.value) return loadFavorites(sort)
        val offset = _albums.value.size
        job = viewModelScope.launch {
            nori.library.albums(sort, pageSize, offset).catch { }.collect { page ->
                val all = _albums.value.from(offset, page)
                if (sort == AlbumSort.ARTIST_ROWS) regroup(all, offset)
                _albums.value = all
                exhausted = albumsExhausted(page.size.toUInt())
                _none.value = offset == 0 && page.isEmpty()
            }
        }
    }

    /** Every favorite at once, in [sort]'s order (the core's `favorites_in_order`): the stored answer, then the server's. */
    private fun loadFavorites(sort: AlbumSort) {
        exhausted = true
        job = viewModelScope.launch {
            nori.library.starred().catch { }.collect { starred ->
                val all = withContext(Dispatchers.Default) { favoritesInOrder(starred.albums, sort) }
                if (sort == AlbumSort.ARTIST_ROWS) regroup(all, 0)
                _albums.value = _albums.value.from(0, all)
                _none.value = all.isEmpty()
            }
        }
    }

    /** [rows] for [all], whose albums from [offset] on are new: the row they may continue is grouped again with them. */
    private suspend fun regroup(all: List<Album>, offset: Int) {
        val from = rowStarts.lastOrNull { it < offset } ?: 0
        val kept = rowStarts.count { it < from }
        val tail = all.subList(from, all.size).toList()
        val added = withContext(Dispatchers.Default) { artistRows(tail) }.map { it.toInt() }
        rowStarts = rowStarts.take(kept) + added.map { from + it }
        _rows.value = _rows.value.take(kept) + added.mapIndexed { i, s -> tail.subList(s, added.getOrElse(i + 1) { tail.size }) }
    }
}

class ArtistsViewModel(app: Application) : NoriViewModel(app) {
    val artists: StateFlow<Load<List<Artist>>> = nori.library.artists().asLoad()
}

class PlaylistsViewModel(app: Application) : NoriViewModel(app) {
    @OptIn(ExperimentalCoroutinesApi::class)
    val playlists: StateFlow<Load<List<Playlist>>> = nori.library.playlistsVersion.flatMapLatest { nori.library.playlists() }.asLoad()

    fun create(name: String) = viewModelScope.launch { runCatching { nori.library.createPlaylist(name, emptyList()) } }
}

class StarredViewModel(app: Application) : NoriViewModel(app) {
    // Re-queried on every star change: the one-shot read would otherwise keep a removed favourite
    // until the screen is reopened. The stored answer paints first, so there is no loading flash.
    // This session's marks are laid over it by the core as it is read, so an unstarred item leaves the
    // list at once rather than when the server's new answer arrives.
    @OptIn(ExperimentalCoroutinesApi::class)
    val starred: StateFlow<Load<Starred>> = nori.library.starsVersion.flatMapLatest { nori.library.starred() }.asLoad()
}

class GenresViewModel(app: Application) : NoriViewModel(app) {
    val genres: StateFlow<Load<List<Genre>>> = nori.library.genres().asLoad()
}

/** The Library page's own: what the downloads weigh, for its row of places. */
class LibraryHomeViewModel(app: Application) : NoriViewModel(app) {
    /** This profile's downloaded songs' size, worded (the download cache holds every profile's); null until weighed. */
    val downloadsSize: StateFlow<String?> = nori.downloads.state
        .map { dev.nori.music.text.Fmt.bytes(nori.sources.downloadBytes(it.doneIds)) }
        .flowOn(Dispatchers.IO)
        .stateIn(viewModelScope, kotlinx.coroutines.flow.SharingStarted.WhileSubscribed(5_000), null)
}

class RadioViewModel(app: Application) : NoriViewModel(app) {
    private val refresh = MutableStateFlow(0)
    @OptIn(ExperimentalCoroutinesApi::class)
    val stations: StateFlow<Load<List<RadioStation>>> = refresh.flatMapLatest { nori.library.radio() }.asLoad()
    fun play(s: RadioStation) = nori.player.playRadio(s)
    fun add(name: String, url: String) = viewModelScope.launch { runCatching { nori.library.createRadio(name, url) }; refresh.value++ }
    fun delete(id: String) = viewModelScope.launch { runCatching { nori.library.deleteRadio(id) }; refresh.value++ }
}

// ---- detail screens; ids arrive through [open] because the UI layer owns navigation ----

@OptIn(ExperimentalCoroutinesApi::class)
abstract class DetailViewModel<T>(app: Application) : NoriViewModel(app) {
    protected val id = MutableStateFlow<String?>(null)
    fun open(id: String) { this.id.value = id }
    protected abstract fun load(id: String): kotlinx.coroutines.flow.Flow<T>
    val ui: StateFlow<Load<T>> by lazy { id.flatMapLatest { if (it == null) flow { } else load(it) }.asLoad() }
}

open class AlbumViewModel(app: Application) : DetailViewModel<AlbumDetail>(app) {
    /**
     * Read again when a provider's song (octo-fiesta) starts sounding and when it stops: playing it made
     * the server download it into the library, and its row's cloud goes only once the server is asked
     * again (the core never takes a stored page with a provider's song for fresh). Only while the page
     * is watched, and only on those moves, not on every player event.
     */
    @OptIn(ExperimentalCoroutinesApi::class)
    override fun load(id: String) = remoteSounding().flatMapLatest { nori.library.album(id) }

    private fun remoteSounding() = nori.player.state.map { s -> s.current?.takeIf { s.playing && it.isExternal }?.id }.distinctUntilChanged()
}

class DownloadedAlbumViewModel(app: Application) : AlbumViewModel(app) {
    override fun load(id: String) = nori.downloads.state.map {
        withContext(Dispatchers.IO) {
            nori.core.downloadedAlbum(id) ?: error(getApplication<Application>().getString(dev.nori.music.app.R.string.note_no_downloads))
        }
    }
}

data class ArtistUi(val detail: ArtistDetail, val info: ArtistInfo?, val top: List<Song>)

class ArtistViewModel(app: Application) : DetailViewModel<ArtistUi>(app) {
    @OptIn(ExperimentalCoroutinesApi::class)
    override fun load(id: String) = nori.library.artist(id).flatMapLatest { d ->
        combine(
            nori.library.artistInfo(id).map<ArtistInfo, ArtistInfo?> { it }.catch { emit(null) }.onStart { emit(null) },
            nori.library.topSongs(d.artist.name).catch { emit(emptyList()) }.onStart { emit(emptyList()) },
        ) { info, top -> ArtistUi(d, info, top) }
    }
}

class PlaylistViewModel(app: Application) : DetailViewModel<PlaylistDetail>(app) {
    private val version = MutableStateFlow(0)
    @OptIn(ExperimentalCoroutinesApi::class)
    override fun load(id: String) = version.flatMapLatest { nori.library.playlist(id) }
}

class GenreViewModel(app: Application) : DetailViewModel<List<Song>>(app) {
    override fun load(id: String) = flow { emit(nori.library.songsByGenre(id)) }
}

/** The orders of the "all songs" list; what each sorts on is the core's (`song_sorts`), its name Say's. */
enum class SongSort { TITLE, ARTIST, ALBUM, YEAR, ADDED, PLAYS, LONGEST }

/** Every song of the offline index, a page at a time; the server's, where the profile keeps no index (the core's `songs_listed`). */
class SongsViewModel(app: Application) : NoriViewModel(app) {
    private val _sort = MutableStateFlow(SongSort.valueOf(songSortSaved(nori.settings.value.listPrefs)))
    val sort: StateFlow<SongSort> = _sort
    private val _starred = MutableStateFlow(false)
    val starredOnly: StateFlow<Boolean> = _starred
    private val _songs = MutableStateFlow(Grown.empty<Song>())
    val songs: StateFlow<List<Song>> = _songs
    private var years: IntRange? = null
    private var job: Job? = null
    private var exhausted = false

    init { loadMore() }

    fun setYears(range: IntRange?) { if (range != years) { years = range; reset() } }
    fun setSort(s: SongSort) {
        _sort.value = s
        val kept = songSortKept(s.name)
        nori.settings.update { it.copy(listPrefs = it.listPrefs + (kept.key to kept.value)) }
        reset()
    }
    fun setStarredOnly(on: Boolean) { _starred.value = on; reset() }
    private fun reset() { job?.cancel(); _songs.value = Grown.empty(); exhausted = false; loadMore() }

    fun loadMore() {
        if (exhausted || job?.isActive == true) return
        val s = _sort.value
        val st = _starred.value
        val y = years
        val offset = _songs.value.size
        job = viewModelScope.launch {
            // A page that could not be read ends the list, as an empty one would.
            val next = runCatching {
                nori.library.songsListed(s.name, st, (y?.first ?: 0).toUInt(), (y?.last ?: 0).toUInt(), offset.toUInt())
            }.getOrNull()
            // runCatching also caught a cancellation: a reset while the page was being read drops it.
            ensureActive()
            _songs.value = _songs.value.plus(next?.songs.orEmpty())
            exhausted = next?.exhausted ?: true
        }
    }
}

class DecadesViewModel(app: Application) : NoriViewModel(app) {
    val decades: StateFlow<Load<List<dev.nori.music.ffi.library.Decade>>> = flow { emit(nori.library.decades()) }.asLoad()
}

class FoldersViewModel(app: Application) : NoriViewModel(app) {
    val roots: StateFlow<Load<List<Artist>>> = nori.library.folders().asLoad()
}

class FolderViewModel(app: Application) : DetailViewModel<dev.nori.music.ffi.model.Directory>(app) {
    override fun load(id: String) = nori.library.folder(id)
}
