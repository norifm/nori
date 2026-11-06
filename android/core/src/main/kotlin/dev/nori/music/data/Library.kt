package dev.nori.music.data

import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.library.AlbumDetail
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.library.ArtistDetail
import dev.nori.music.ffi.model.ArtistInfo
import dev.nori.music.ffi.Client
import dev.nori.music.ffi.Core
import dev.nori.music.ffi.model.Genre
import dev.nori.music.ffi.model.IngestStats
import dev.nori.music.ffi.lyrics.LyricsPick
import dev.nori.music.ffi.lyrics.LyricsShown
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.channelFlow
import dev.nori.music.ffi.Page
import dev.nori.music.ffi.PageShown
import dev.nori.music.ffi.StarsShown
import dev.nori.music.ffi.model.Playlist
import dev.nori.music.ffi.library.PlaylistDetail
import dev.nori.music.ffi.model.RadioStation
import dev.nori.music.ffi.Read
import dev.nori.music.ffi.model.SearchResult
import dev.nori.music.ffi.model.ServerInfo
import dev.nori.music.ffi.model.Song
import dev.nori.music.ffi.model.Starrable
import dev.nori.music.ffi.library.Starred
import dev.nori.music.ffi.library.StarMarks
import dev.nori.music.ffi.library.AlbumSort
import dev.nori.music.ffi.library.librarySizes
import dev.nori.music.ffi.net.Write
import dev.nori.music.net.lifted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext

/** What can be starred. The marks are kept per kind by the core (`stars.rs`), keyed by id alone. */
enum class StarKind(internal val target: Starrable) {
    SONG(Starrable.SONG), ALBUM(Starrable.ALBUM), ARTIST(Starrable.ARTIST),
}

/** The mark of [id] of [kind], if this session changed it. */
fun StarMarks.of(kind: StarKind, id: String): Boolean? = when (kind) {
    StarKind.SONG -> songs[id]
    StarKind.ALBUM -> albums[id]
    StarKind.ARTIST -> artists[id]
}

/** How the app sizes, names and keeps artwork (the core's `cover_rules`), read once. */
object Covers {
    val rules: dev.nori.music.ffi.CoverRules by lazy { dev.nori.music.ffi.coverRules() }

    /**
     * A cover of an octo-fiesta provider item (external song, album, artist or playlist), which is never
     * kept on disk: octo-fiesta draws a "not downloaded" badge on it and replaces the picture under the same id
     * once the item is in the library. The core's (`is_provider_cover`), through a `@FastNative` door:
     * 0.2 µs and nothing allocated, where the same test in Kotlin took 0.5 µs and 32 bytes.
     */
    fun isProvider(url: String): Boolean = dev.nori.music.look.CoverPixels.isProvider(url)
}

/**
 * Everything the app knows about the server. The core's [Client] does the work - which address, what is
 * stored and for how long, what is queued while offline, how LRCLIB is asked - and this turns its answers
 * into flows for the screens: the stored answer first, the network's only when it differs. The UI never
 * sees bytes or URLs.
 */
class Library(
    private val coreOf: () -> Core,
    private val clientOf: () -> Client,
) {
    // Resolved on every use, which is always on an IO thread: the core belongs to the active server profile
    // and building it costs ~100 ms the UI thread should not pay.
    private val core get() = coreOf()
    private val client get() = clientOf()
    /** How much each read asks for (the core's `library_sizes`). */
    private val sizes by lazy { librarySizes() }

    /**
     * Star changes made this session, one map per kind, as the core keeps them (`stars.rs`, which also
     * lays them over the lists it is asked to). Reads that paint once (a cached list, a queue snapshot)
     * would otherwise sit stale until the screen is reopened; the UI prefers these over the snapshot.
     * Bumped alongside [starsVersion].
     */
    private val _starMarks = MutableStateFlow(StarMarks(emptyMap(), emptyMap(), emptyMap()))
    val starMarks: StateFlow<StarMarks> = _starMarks.asStateFlow()
    /** Star state as it should be shown: this session's change wins over the [snapshot] a list or queue item was built with. */
    /** Applies the core's favorite marks to the visible library. */
    fun applyMarks(marks: StarMarks) {
        _starMarks.value = marks
        _starsVersion.update { it + 1 }
    }

    fun isStarred(kind: StarKind, id: String, snapshot: Boolean): Boolean = _starMarks.value.of(kind, id) ?: snapshot
    /** Bumped on every star change, so one-shot reads (the favourites list) can re-query. */
    private val _starsVersion = MutableStateFlow(0)
    val starsVersion: StateFlow<Int> = _starsVersion.asStateFlow()

    /** A read that always asks the server. */
    private suspend fun <T> call(read: Read, pick: (Page) -> T): T = withContext(Dispatchers.IO) { pick(lifted { client.readNow(read) }) }

    /**
     * The stored answer paints the screen at once; the server is asked unless that answer is fresh, and
     * its answer is emitted only when it differs. All of it, and when a failure is an error (offline with
     * something stored is not), is the core's (`Client::read_cached`); this only hands each page on.
     */
    private inline fun <T> cached(read: Read, crossinline pick: (Page) -> T): Flow<T> = channelFlow {
        val c = client
        val shown = object : PageShown {
            override fun show(page: Page) { trySend(page) }
        }
        lifted { c.readCached(read, shown) }
    }.buffer(Channel.UNLIMITED).flowOn(Dispatchers.IO).map { pick(it) }

    /**
     * Throws away the stored answers whose key starts with one of [prefixes], so the next read of them
     * has to ask the server (see the core's `drop_cached`).
     */
    suspend fun dropCached(vararg prefixes: String) = withContext(Dispatchers.IO) { lifted { client.dropCached(prefixes.toList()) } }

    // ---- session ----

    suspend fun musicFolders(): List<dev.nori.music.ffi.model.MusicFolder> = call(Read.MusicFolders) { (it as Page.Folders).v }


    // ---- search ----

    /**
     * Always asks the server, so provider results from octo-fiesta show up. One big
     * page: the proxy repeats its external results on every offset.
     */
    suspend fun search(query: String): SearchResult =
        call(Read.Search(query, sizes.searchSongs, sizes.searchAlbums, sizes.searchArtists)) { (it as Page.Found).v }

    /**
     * The server's answer to [query], taken into [session] by the core (`SearchSession::ask`) without
     * coming out here; null when the field has moved on.
     */
    suspend fun searchInto(session: dev.nori.music.ffi.SearchSession, query: String): dev.nori.music.ffi.library.SearchView? =
        withContext(Dispatchers.IO) { lifted { session.ask(client, query) } }

    suspend fun searchHistory(): List<String> = withContext(Dispatchers.IO) { core.searchHistory() }
    suspend fun forgetSearches() = withContext(Dispatchers.IO) { core.searchForget() }
    suspend fun forgetSearch(query: String) = withContext(Dispatchers.IO) { core.searchForgetOne(query) }

    // ---- browse ----

    /** "By year" is this year's albums; "random" is never stored (the core decides both). */
    fun albums(sort: AlbumSort, size: Int, offset: Int = 0, genre: String? = null): Flow<List<Album>> =
        cached(Read.AlbumList(sort, size, offset, genre)) { (it as Page.Albums).v }

    fun artists(): Flow<List<Artist>> = cached(Read.ArtistIndex) { (it as Page.Artists).v }
    fun album(id: String): Flow<AlbumDetail> = cached(Read.AlbumById(id)) { (it as Page.AlbumPage).v }

    /** Page [offset] of a home album shelf, read as the core says (`shelf_albums_read`). */
    fun shelfAlbums(shelf: dev.nori.music.ffi.library.HomeShelf.Albums, offset: Int = 0): Flow<List<Album>> =
        cached(dev.nori.music.ffi.shelfAlbumsRead(shelf.sort, shelf.size, shelf.followsStars, offset.toUInt())) { (it as Page.Albums).v }
    fun artist(id: String): Flow<ArtistDetail> = cached(Read.ArtistById(id)) { (it as Page.ArtistPage).v }
    fun artistInfo(id: String): Flow<ArtistInfo> = cached(Read.ArtistAbout(id)) { (it as Page.About).v }
    fun topSongs(artist: String): Flow<List<Song>> = cached(Read.TopSongs(artist)) { (it as Page.Songs).v }
    fun playlists(): Flow<List<Playlist>> = cached(Read.PlaylistList) { (it as Page.Playlists).v }
    /** The source's own home feed (`ProfileRules.feed`), shelf by shelf. */
    fun homeFeed(): Flow<List<dev.nori.music.ffi.model.FeedShelf>> = cached(Read.HomeFeed) { (it as Page.Feed).v }
    suspend fun featuredAlbums(limit: Int): List<Album> = withContext(Dispatchers.IO) { client.featuredAlbums(limit.coerceAtLeast(0).toUInt()) }
    suspend fun featuredPlaylists(): List<Playlist> = withContext(Dispatchers.IO) { client.featuredPlaylists() }
    fun playlist(id: String): Flow<PlaylistDetail> = cached(Read.PlaylistById(id)) { (it as Page.PlaylistPage).v }
    /** The favourites, with this session's marks laid over them by the core as they are read. */
    fun starred(): Flow<Starred> = cached(Read.StarredItems) { (it as Page.StarredPage).v }

    /**
     * The starred songs handed to the mixes (the core's `mix_favourites_stored` and `_refresh`): read and
     * handed inside the core, the stored answer first and the server's when it differs. Emits after each.
     */
    fun handFavourites(): Flow<Unit> = flow {
        val c = client
        val stored = lifted { c.mixFavouritesStored() }
        if (stored.handed) emit(Unit)
        if (stored.fresh) return@flow
        if (lifted { c.mixFavouritesRefresh(stored.digest) }) emit(Unit)
    }.flowOn(Dispatchers.IO)
    fun genres(): Flow<List<Genre>> = cached(Read.GenreList) { (it as Page.Genres).v }
    fun radio(): Flow<List<RadioStation>> = cached(Read.RadioList) { (it as Page.Stations).v }

    /**
     * The server's lyrics, and when it has no timed ones and the settings allow it, what the lyrics
     * services have: asked together, ranked and remembered by the core (`lyrics_lookup`), each better
     * answer emitted as it comes. Leaving the lyrics stops collecting this, which cancels the lookup and
     * every request in it.
     */
    fun lyricsFor(song: Song): Flow<FoundLyrics> = channelFlow {
        // The server's first, then the services', each only when it is new, in the core's order
        // (`Client::lyrics_for`).
        val shown = object : LyricsShown {
            override fun show(pick: LyricsPick) { trySend(FoundLyrics(pick.lyrics, pick.origin)) }
        }
        lifted { client.lyricsFor(song.id, shown) }
    }.buffer(Channel.UNLIMITED).flowOn(Dispatchers.IO)

    /**
     * [found], [songId]'s lyrics, a step sooner ([dir] > 0) or later (< 0), or back to the timing the core
     * checked them to (0); kept by the core for these words (`Core::lyrics_nudge`).
     */
    suspend fun lyricsNudge(songId: String, found: FoundLyrics, dir: Int): FoundLyrics = withContext(Dispatchers.IO) {
        core.lyricsNudge(songId, LyricsPick(found.lyrics, found.source), dir).let { FoundLyrics(it.lyrics, it.origin) }
    }

    // ---- local only: history, mixes, smart playlists (all computed in the Rust core from the index) ----

    suspend fun clearHistory() = withContext(Dispatchers.IO) { core.historyClear() }

    suspend fun excludeFromMixes(songId: String, excluded: Boolean) = withContext(Dispatchers.IO) { core.mixExcludedSet(songId, excluded) }

    // ---- what to play: each list is made in the core (actions.rs), requests included ----

    /** A radio from one song: it, then what the server finds similar, or random songs of its genre. */
    suspend fun radio(song: Song): List<Song> = withContext(Dispatchers.IO) { lifted { client.radio(song.id) } }

    /** An instant mix from the index, or the radio when the index has nothing near the song. */
    suspend fun instantMix(song: Song): List<Song> = withContext(Dispatchers.IO) { lifted { client.instantMix(song.id) } }

    /** Every album of an artist (a provider's left out), in order, as one list of songs. */
    suspend fun artistSongs(artistId: String): List<Song> = withContext(Dispatchers.IO) { lifted { client.artistSongsOf(artistId) } }
    suspend fun albumSongs(albumId: String): List<Song> = withContext(Dispatchers.IO) { lifted { client.albumSongsOf(albumId) } }
    suspend fun playlistSongs(playlistId: String): List<Song> = withContext(Dispatchers.IO) { lifted { client.playlistSongsOf(playlistId) } }

    suspend fun shuffleAll(): List<Song> = withContext(Dispatchers.IO) { lifted { client.shuffleAll() } }
    suspend fun shuffleAlbums(): List<Song> = withContext(Dispatchers.IO) { lifted { client.shuffleAlbums() } }

    suspend fun smartPlaylists() = withContext(Dispatchers.IO) { core.smartList() }
    fun smartDefaults() = dev.nori.music.ffi.library.smartDefaults()
    suspend fun smartSave(id: String, name: String, json: String): String = withContext(Dispatchers.IO) { core.smartSave(id, name, json) }
    suspend fun smartDelete(id: String) = withContext(Dispatchers.IO) { core.smartDelete(id) }
    suspend fun smartPage(json: String): dev.nori.music.ffi.library.SmartPage =
        withContext(Dispatchers.IO) { core.smartPage(json, sizes.smartSongs) }


    /** Sorted pages of the offline index: the "all songs" and "by decade" lists. */
    suspend fun browseSongs(sort: String, descending: Boolean, starredOnly: Boolean, years: IntRange?, offset: Int, limit: Int): List<Song> = withContext(Dispatchers.IO) {
        core.browseSongs(sort, descending, starredOnly, (years?.first ?: 0).toUInt(), (years?.last ?: 0).toUInt(), offset.toUInt(), limit.toUInt())
    }

    suspend fun decades(): List<dev.nori.music.ffi.library.Decade> = withContext(Dispatchers.IO) { core.browseDecades() }

    /** The [limit] genres with most indexed songs, each with a cover (the core's `genre_tiles`). */
    suspend fun genreTiles(limit: Int): List<dev.nori.music.ffi.library.GenreTile> = withContext(Dispatchers.IO) { core.genreTiles(limit.toUInt()) }
    suspend fun genreCover(genre: String): String? = withContext(Dispatchers.IO) { core.genreCover(genre) }


    suspend fun randomSongs(): List<Song> = call(Read.RandomSongs(sizes.randomSongs, null)) { (it as Page.Songs).v }

    suspend fun songsByGenre(genre: String): List<Song> = withContext(Dispatchers.IO) { lifted { client.genreSongs(genre) } }


    /** What carries the queue on past the song playing (see the core's autofill.rs); nothing on any failure. */
    suspend fun autofill(): dev.nori.music.ffi.Refill = withContext(Dispatchers.IO) { client.autofill() }

    /** What a plugin plays when the queue ended; empty while none decides. */
    suspend fun queueEnded(): dev.nori.music.ffi.Refill = withContext(Dispatchers.IO) { client.queueEnded() }

    /** Whether the fetched [count] songs go in; if so the core records what it picked. */
    suspend fun autofillArrived(fresh: dev.nori.music.ffi.Refill): Boolean = withContext(Dispatchers.IO) { client.autofillArrived(fresh) }

    /**
     * Draws mix [id] unless this period's draw is there already ([again]: a different one); the core
     * fetches the server's random songs when the index has nothing to draw from. True when it changed.
     */
    suspend fun mixEnsure(id: String, again: Boolean): Boolean = withContext(Dispatchers.IO) { client.mixEnsure(id, again) }

    /** Draws whichever mixes are missing or from the last period. True when any tile changed. */
    suspend fun mixWarm(): Boolean = withContext(Dispatchers.IO) { client.mixWarmAll() }

    /**
     * The queue another device or app left on the server, when it is worth offering to this one while it is
     * [playing] or not and shows the song [shown]; whether it is, is the core's (`queue_offer`).
     */
    suspend fun queueOffer(playing: Boolean, shown: String?): dev.nori.music.ffi.library.QueueOffer? =
        withContext(Dispatchers.IO) { lifted { client.queueOffer(playing, shown) } }

    /** [offer] was taken or turned down: the same queue is not offered again. */
    fun queueOfferDone(offer: dev.nori.music.ffi.library.QueueOffer) = client.queueOfferDone(offer)

    suspend fun song(id: String): Song? = call(Read.SongById(id)) { (it as Page.OneSong).v }

    // ---- writes: the core sends them, keeps them while offline and drops the reads they make stale ----

    private suspend fun write(w: Write) = withContext(Dispatchers.IO) { lifted { client.write(w) } }

    /** Replays queued writes. Stops at the first network failure; a write the server rejects is dropped. */
    suspend fun flushPending() = withContext(Dispatchers.IO) { lifted { client.flushPending() } }

    suspend fun star(kind: StarKind, id: String, on: Boolean) {
        // The core puts the mark up before it asks the server, so the heart fills under the finger, and
        // puts the one from before back if the server refuses (offline is not refusing: the core keeps
        // those and replays them), handing the marks over each time (`Client::star`).
        val shown = object : StarsShown {
            override fun marks(marks: StarMarks) {
                applyMarks(marks)
            }
        }
        withContext(Dispatchers.IO) { lifted { client.star(kind.target, id, on, shown) } }
        // Now the server has it, so the lists that come from it can be asked again.
        _starsVersion.update { it + 1 }
    }

    /** Bumped when a playlist is made, added to or deleted, so the lists of playlists read again (the core has dropped the stale answer). */
    private val _playlistsVersion = MutableStateFlow(0)
    val playlistsVersion: StateFlow<Int> = _playlistsVersion.asStateFlow()

    private suspend fun writePlaylist(w: Write) { write(w); _playlistsVersion.update { it + 1 } }

    suspend fun createPlaylist(name: String, songIds: List<String>) = writePlaylist(Write.CreatePlaylist(name, songIds))

    suspend fun addToPlaylist(id: String, songIds: List<String>) = writePlaylist(Write.AddToPlaylist(id, songIds))

    suspend fun deletePlaylist(id: String) = writePlaylist(Write.DeletePlaylist(id))

    /** Not worth queueing: by the time it could be replayed it is no longer true. */
    suspend fun nowPlaying(id: String) { call(Read.NowPlaying(id)) { it } }

    suspend fun createRadio(name: String, streamUrl: String) = write(Write.CreateRadio(name, streamUrl))
    suspend fun deleteRadio(id: String) = write(Write.DeleteRadio(id))

    /** A public link to a song or album; the server must have sharing enabled. */
    suspend fun share(id: String): String = call(Read.ShareLink(id)) { (it as Page.ShareUrl).v }


    /** [submission] false marks "now playing"; true counts the play. */
    suspend fun scrobble(id: String, submission: Boolean, timeMs: Long? = null) = write(Write.Scrobble(id, submission, timeMs))

    // ---- queue hand-off between devices ----

    /** The core's queue handed to the server (`Client::playlist_push`); nothing when there is nothing to hand. */
    suspend fun pushQueue(current: String?, positionMs: Long) = withContext(Dispatchers.IO) { lifted { client.playlistPush(current, positionMs) } }


    // ---- offline index ----

    /**
     * Walks the whole library into the local index and drops what the server no longer has. The pages go
     * from the socket into SQLite inside Rust; only the counts come back.
     */
    /** A page of the songs list (the core's `songs_listed`): the offline index's, or the server's where the profile keeps none. */
    suspend fun songsListed(sort: String, reversed: Boolean, starredOnly: Boolean, yearFrom: UInt, yearTo: UInt, offset: UInt) =
        withContext(Dispatchers.IO) { lifted { client.songsListed(sort, reversed, starredOnly, yearFrom, yearTo, offset) } }

    /** A page of the indexed albums in [sort]'s order turned around, which the server cannot list (the core's `albums_indexed`). */
    suspend fun albumsIndexed(sort: AlbumSort, reversed: Boolean, offset: Int, size: Int): List<Album> =
        withContext(Dispatchers.IO) { core.albumsIndexed(sort, reversed, offset.toUInt(), size.toUInt()) }

    suspend fun sync(): IngestStats = withContext(Dispatchers.IO) { lifted { client.syncLibrary() } }

    suspend fun indexSize(): IngestStats = withContext(Dispatchers.IO) { core.indexSize() }

    /** The signed prefix the profile's cover addresses start with; null where the core makes each one. */
    private class CoverForm(val prefix: String?)
    @Volatile private var coverForm: CoverForm? = null

    /**
     * Called for every row a list draws, on the UI thread: plain string work, no FFI. The core signs the
     * prefix once (per server and address) and says how the rest is put together (`cover_rules`); a row
     * only appends its id and size, which is cheaper than any crossing into the core would be. A provider
     * without a prefix (`cover_prefix`) has an address of its own for each cover, sized by the core
     * (`cover_address`).
     */
    fun coverUrl(id: String?, size: Int): String? {
        if (id == null) return null
        // A plugin's cover is its own address on any profile (`CoverRules::address_mark`).
        if (Covers.rules.addressMark in id) return id.replace(Covers.rules.sizeHole, size.toString())
        val form = coverForm ?: core.let { c ->
            CoverForm(c.coverPrefix()?.plus(Covers.rules.idParam))
        }.also { coverForm = it }
        val prefix = form.prefix ?: return core.coverAddress(id, size.toUInt())
        return prefix + android.net.Uri.encode(id) + Covers.rules.sizeParam + size
    }

    /** The signed prefix changes with the server, the credentials or the address in use. */
    fun onServerChanged() { coverForm = null }

    /** Another profile is active: its core keeps its own star marks, and this one's are not its hearts. */
    fun onProfileChanged() {
        _starMarks.value = StarMarks(emptyMap(), emptyMap(), emptyMap())
    }
}
