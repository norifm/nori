package dev.nori.music.app.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.ui.text.withStyle
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.AlbumViewModel
import dev.nori.music.app.vm.ArtistUi
import dev.nori.music.app.vm.ArtistViewModel
import dev.nori.music.app.vm.FolderViewModel
import dev.nori.music.app.vm.GenreViewModel
import dev.nori.music.app.vm.Load
import dev.nori.music.app.vm.PlaylistViewModel
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.library.AlbumDetail
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.library.PlaylistDetail
import dev.nori.music.ffi.model.OriginKind
import dev.nori.music.ffi.model.PageOrigin
import dev.nori.music.ffi.model.Song

/**
 * A long playlist's way to narrow itself (play and shuffle live in the hero). An album's page has none:
 * nobody reaches for a search box to find a track among ten they can already see.
 */
@Composable
private fun FilterField(count: Int, filter: String, onFilter: (String) -> Unit) {
    val filtering = filter.isNotEmpty()
    if (!remember(count, filtering) { dev.nori.music.ffi.library.filterOffered(count.toUInt(), filtering) }) return
    SearchField(filter, onFilter, say.filter, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp))
}

/**
 * A hero page over a list of [songs] with no cover of its own (a genre, a folder, a smart playlist): its
 * name and [caption], Play and Shuffle while there is something to play, then [actions] and a ⋯ with the
 * list's queue and download entries, and [content] under it.
 */
@Composable
internal fun SongListPage(
    title: String, caption: String, songs: List<Song>, done: Set<String>, from: PageOrigin, actions: ActionsViewModel,
    pageActions: @Composable RowScope.() -> Unit = {}, content: LazyListScope.() -> Unit,
) {
    val any = songs.isNotEmpty()
    HeroPage(
        coverUrl = null,
        title = title,
        caption = caption,
        onPlay = if (any) { { actions.play(songs, from = from) } } else null,
        onShuffle = if (any) { { actions.shuffle(songs, from) } } else null,
        queue = remember(from) { dev.nori.music.ffi.library.PageQueue(from) },
        actions = {
            pageActions()
            if (any) MoreCircle(songsMore(songs, done, actions))
        },
        content = content,
    )
}

/** The ⋯ entries of a page of [songs]: add them to the queue, and download what is missing or give it back. */
@Composable
internal fun songsMore(songs: List<Song>, done: Set<String>, actions: ActionsViewModel): List<Pair<String, () -> Unit>> =
    listOf(say.addToQueue to { actions.enqueue(songs) }, downloadEntry(songs, done, actions))

/**
 * The places in [songs] a filter keeps, by the fields nori-core's `song_index` finds songs by; null
 * with no filter, for all of them. The index takes the list's text once, and only when a filter is first
 * typed; each keystroke then sends the filter.
 */
@Composable
private fun rememberMatching(songs: List<Song>, q: String): List<UInt>? {
    val index = remember(songs) { lazy { dev.nori.music.ffi.library.songIndex(songs) } }
    return remember(index, q) { if (q.isBlank()) null else index.value.view(q).rows }
}

/**
 * An artist's biography: four lines, and all of it on a tap when there is more, the way Apple's opens.
 * Cut, the fourth line ends "… More", the word in the accent: the text is shortened until that fits, found
 * by measuring (a dozen measures, once per text and width). Tapped again, it folds back.
 */
@Composable
private fun Biography(text: String, modifier: Modifier) {
    var open by androidx.compose.runtime.saveable.rememberSaveable(text) { mutableStateOf(false) }
    val style = MaterialTheme.typography.bodyMedium
    val accent = MaterialTheme.colorScheme.primary
    val measurer = androidx.compose.ui.text.rememberTextMeasurer()
    androidx.compose.foundation.layout.BoxWithConstraints(modifier.fillMaxWidth().padding(horizontal = Space.gutter)) {
        val width = constraints.maxWidth
        val folded = remember(text, width, style, accent) { foldedBiography(text, width, style, accent, say.more, measurer) }
        Text(
            if (open || folded == null) androidx.compose.ui.text.AnnotatedString(text) else folded,
            Modifier.fillMaxWidth().clickable(enabled = folded != null) { open = !open }.animateContentSize(),
            style = style, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** [text] cut to four lines ending "… [more]" at [width], or null when all of it fits. */
private fun foldedBiography(
    text: String, width: Int, style: androidx.compose.ui.text.TextStyle, accent: androidx.compose.ui.graphics.Color, more: String,
    measurer: androidx.compose.ui.text.TextMeasurer,
): androidx.compose.ui.text.AnnotatedString? {
    val limit = androidx.compose.ui.unit.Constraints(maxWidth = width)
    val whole = measurer.measure(text, style, maxLines = 4, constraints = limit)
    if (!whole.hasVisualOverflow) return null
    fun cutAt(n: Int) = androidx.compose.ui.text.buildAnnotatedString {
        append(text.take(n).trimEnd())
        append("… ")
        withStyle(androidx.compose.ui.text.SpanStyle(color = accent)) { append(more) }
    }
    // The longest start of the text that still leaves room for "… More" on the fourth line.
    var lo = 0
    var hi = whole.getLineEnd(3, visibleEnd = true)
    while (lo < hi) {
        val mid = (lo + hi + 1) / 2
        if (measurer.measure(cutAt(mid), style, maxLines = 4, constraints = limit).hasVisualOverflow) hi = mid - 1 else lo = mid
    }
    return cutAt(lo)
}

/**
 * An item of a page's body that may join it after the page is drawn (an artist's biography and top songs
 * come after the page itself): it arrives with the body, and fades in where it joins if it is late.
 */
private fun LazyItemScope.late(arrival: State<Float>): Modifier =
    (if (AppMotion.reduce) Modifier.animateItem(null, null, null) else Modifier.animateItem()).arriving(arrival)

/** An artist's releases by kind, as the core grouped them when the artist was read (`pages::release_groups`). */
private fun groupsOf(d: dev.nori.music.ffi.library.ArtistDetail): List<Pair<String, List<Album>>> =
    d.groups.map { g -> say.releaseShelf(g) to g.albums.map { d.albums[it.toInt()] } }


/**
 * "Download" is the wrong word once the songs are already here, and so is offering all of them when
 * only a few are missing. This says what is actually left to do - and offers to give the space back
 * when there is nothing left.
 */
@Composable
internal fun downloadEntry(songs: List<Song>, done: Set<String>, actions: ActionsViewModel): Pair<String, () -> Unit> {
    val missing = remember(songs, done) { songs.filterNot { it.id in done } }
    // What it does is the core's (`menus::download_entry`); what it says, Say's.
    val act = remember(songs.size, missing.size) { dev.nori.music.ffi.library.downloadEntry(songs.size.toUInt(), missing.size.toUInt()) }
    val label = remember(act, missing.size) { say.downloadEntry(act, missing.size) }
    return label to when (act) {
        dev.nori.music.ffi.library.DownloadAct.ALL -> { { actions.download(songs) } }
        dev.nori.music.ffi.library.DownloadAct.MISSING -> { { actions.download(missing) } }
        dev.nori.music.ffi.library.DownloadAct.REMOVE -> { { actions.undownload(songs) } }
    }
}

/**
 * A page's own queue before its songs are read, from what it shows (a queue started from it carries
 * [from]), so a page opened on the queue playing answers for it at once. Once read, the page comes with
 * its queue (`AlbumDetail.queue`, and the like), the same one.
 */
@Composable
private fun rememberPageQueue(from: PageOrigin, read: dev.nori.music.ffi.library.PageQueue?): dev.nori.music.ffi.library.PageQueue =
    read ?: remember(from) { dev.nori.music.ffi.library.PageQueue(from) }

@Composable
private fun rememberOrigin(kind: OriginKind, id: String): PageOrigin = remember(kind, id) { PageOrigin(kind, id) }

/** Makes a playlist a favourite on this phone, or not (the core's `pins_toggled`). */
private fun SettingsViewModel.pin(id: String, on: Boolean) =
    update { it.copy(pinnedPlaylists = dev.nori.music.ffi.library.pinsToggled(it.pinnedPlaylists, id, on)) }

/** The last non-null value [LateMore] was given, kept for the menu's exit fade. */
private class LastValue<T : Any> { var value: T? = null }

/**
 * The ⋯ of a hero page, in a slot of its own 46 dp that is there from the first frame: it fades in when
 * the page's detail lands, so the Play pill beside it never shifts. The fade out keeps the last [value].
 */
@Composable
private fun <T : Any> LateMore(value: T?, entries: @Composable (T) -> List<Pair<String, () -> Unit>>) {
    val last = remember { LastValue<T>() }
    if (value != null) last.value = value
    Box(Modifier.size(46.dp), contentAlignment = Alignment.Center) {
        androidx.compose.animation.AnimatedVisibility(
            visible = value != null,
            enter = androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(if (AppMotion.reduce) 0 else 220)),
            exit = androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(if (AppMotion.reduce) 0 else 120)),
        ) { last.value?.let { MoreCircle(entries(it)) } }
    }
}

/** Why a hero page's detail did not come, under the hero. */
@Composable
private fun LoadFailed(message: String) = Text(
    message,
    Modifier.fillMaxWidth().padding(Space.gutter),
    color = MaterialTheme.colorScheme.onSurfaceVariant,
    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
)

/*
 * Album, artist and playlist pages each have one drawing path, from whatever is known of the page so far.
 * What the row that opened it already knew (Nav's hint: cover, title, counts) is enough for the hero to be
 * there from the first frame of the slide (PageMotion), with Play and ⋯ held in their places and the
 * songs rising in under it (Arrive) when the server answers - docs/motion.md items 16 and 17. A page
 * opened with no hint (a deep link, "Go to album" from a song) waits behind LoadBox, which fades the same
 * page in over its loader once the detail is there.
 */

@Composable
fun AlbumScreen(id: String, actions: ActionsViewModel, vm: AlbumViewModel = viewModel()) {
    LaunchedEffect(id) { vm.open(id) }
    val load by vm.ui.collectAsStateWithLifecycle()
    val hint = LocalNav.current.albumHint(id)
    if (hint == null) {
        LoadBox(load) { d -> AlbumPage(d.album, d, null, actions, vm) }
        return
    }
    val detail = (load as? Load.Ready)?.data
    AlbumPage(detail?.album ?: hint, detail, (load as? Load.Failed)?.message, actions, vm)
}

/** An album's page: [detail] null while the songs are on the wire, [failed] when they did not come. */
@Composable
private fun AlbumPage(album: Album, detail: AlbumDetail?, failed: String?, actions: ActionsViewModel, vm: AlbumViewModel) {
    val account = LocalRules.current.account
    val downloads = LocalRules.current.downloads
    val list = rememberSongListState(actions)
    val nav = LocalNav.current
    val arrival = rememberArrival(detail != null)
    // The album's own caption once it is read; until then what the row that opened it knew.
    val caption = remember(detail, album) { detail?.let(say::albumCaption) ?: say.albumHintCaption(album.year.toInt(), album.songCount.toInt(), album.duration.toLong()) }
    // A queue started here - its Play, Shuffle or a row - is the album's; its song played from anywhere else is not.
    val from = remember(detail?.queue, album.id) { detail?.queue?.origin() ?: PageOrigin(OriginKind.ALBUM, album.id) }
    val queue = rememberPageQueue(from, detail?.queue)
    HeroPage(
        coverUrl = vm.cover(album.coverArt, CoverSize.FULL),
        coverStandIns = listOf(vm.cover(album.coverArt, CoverSize.CARD), vm.cover(album.coverArt, CoverSize.ROW)),
        title = album.name,
        subtitle = album.artist,
        caption = caption,
        onSubtitle = album.artistId?.let { a ->
            { nav.artist(a, Artist(a, album.artist, album.coverArt, null, 0u, false, false)) }
        },
        // Play and shuffle wait for the songs: pressing them with an empty list would queue nothing.
        // The row itself is reserved ([awaitingPlay]) so the page does not reflow when they land.
        awaitingPlay = detail == null && failed == null,
        onPlay = detail?.let { d -> { actions.play(d.songs, from = from) } },
        onShuffle = detail?.let { d -> { actions.shuffle(d.songs, from) } },
        queue = queue,
        actions = {
            if (account) {
                val albumStarred = LocalStarMarks.current.effectiveStar(dev.nori.music.data.StarKind.ALBUM, album.id, album.starred)
                FavoriteCircle(albumStarred) { actions.starAlbum(album.id, !albumStarred); Unit }
            }
            LateMore(detail) { d ->
                listOfNotNull(
                    say.addToQueue to { actions.enqueue(d.songs) },
                    downloadEntry(d.songs, list.downloaded, actions).takeIf { downloads },
                    jamStartEntry { actions.play(d.songs, from = from) }.takeIf { account },
                )
            }
        },
    ) {
        when {
            detail != null -> {
                // An album is short enough to scroll and its running order is the point of it, so the songs
                // stay exactly as the record has them, grouped by disc (as the core laid them out,
                // `pages::album_discs`) and never narrowed. A row each: only the ones on screen are composed.
                detail.discs.forEachIndexed { n, disc ->
                    // Only an album of several discs heads them (the core leaves the one disc's empty).
                    if (disc.headed) item(key = "disc-$n", contentType = "disc") { SectionTitle(remember(disc) { say.discHeading(disc) }, Modifier.arriving(arrival)) }
                    songRows(
                        detail.songs, actions, list,
                        numbered = true, rows = disc.songs, lines = disc.lines, arrival = arrival, from = from,
                    )
                }
            }
            failed != null -> item(key = "fail") { LoadFailed(failed) }
            // The hero is already the page; a spinner under it would be a second thing to look at
            // while the songs are on a short wire. Empty until they land.
            else -> Unit
        }
    }
}

@Composable
fun ArtistScreen(id: String, actions: ActionsViewModel, vm: ArtistViewModel = viewModel()) {
    LaunchedEffect(id) { vm.open(id) }
    val load by vm.ui.collectAsStateWithLifecycle()
    val uri = LocalUriHandler.current
    var leaving by remember { mutableStateOf<String?>(null) }
    NoriDialog(leaving, { leaving = null }) { url ->
        AlertCard(title = { Text(say.openInBrowser) }, text = { Text(url) },
            confirmButton = { TextAction(say.open, { runCatching { uri.openUri(url) }; leaving = null }) }, dismissButton = { TextAction(say.cancel, { leaving = null }) })
    }
    val hint = LocalNav.current.artistHint(id)
    if (hint == null) {
        LoadBox(load) { ready -> ArtistPage(ready.detail.artist, ready, null, actions, vm) { leaving = it } }
        return
    }
    val ui = (load as? Load.Ready)?.data
    ArtistPage(ui?.detail?.artist ?: hint, ui, (load as? Load.Failed)?.message, actions, vm) { leaving = it }
}

/** An artist's page: [ui] null while the detail is on the wire, [failed] when it did not come. */
@Composable
private fun ArtistPage(artist: Artist, ui: ArtistUi?, failed: String?, actions: ActionsViewModel, vm: ArtistViewModel, leave: (String) -> Unit) {
    val account = LocalRules.current.account
    val downloads = LocalRules.current.downloads
    val list = rememberSongListState(actions)
    val nav = LocalNav.current
    val arrival = rememberArrival(ui != null)
    val groups = remember(ui?.detail) { ui?.detail?.let(::groupsOf).orEmpty() }
    val similar = remember(ui?.info) { ui?.info?.similar?.let { dev.nori.music.ffi.library.similarArtists(it) }.orEmpty() }
    val topFrom = rememberOrigin(OriginKind.ARTIST_TOP, artist.id)
    var allSongs by remember(artist.id) { mutableStateOf(false) }
    HeroPage(
        coverUrl = vm.cover(artist.coverArt, CoverSize.FULL),
        coverStandIns = listOf(vm.cover(artist.coverArt, CoverSize.CARD), vm.cover(artist.coverArt, CoverSize.ROW)),
        title = artist.name,
        caption = remember(ui?.detail, artist.albumCount) {
            if (ui != null) ui.detail.albums.size.let { n -> if (n == 0 && ui.detail.songs.isNotEmpty()) say.songs(ui.detail.songs.size) else say.releases(n) }
            else artist.albumCount?.takeIf { it > 0u }?.let { say.releases(it.toInt()) }.orEmpty()
        },
        onPlay = ui?.let { ready -> { actions.playArtist(ready.detail.artist.id) } },
        onShuffle = ui?.let { ready -> { actions.playArtist(ready.detail.artist.id, shuffle = true) } },
        awaitingPlay = ui == null && failed == null,
        // Only a queue this page's Play or Shuffle started (all the artist's songs), not any song by them.
        queue = rememberPageQueue(rememberOrigin(OriginKind.ARTIST, artist.id), ui?.detail?.queue),
        actions = {
            if (account) {
                val artistStarred = LocalStarMarks.current.effectiveStar(dev.nori.music.data.StarKind.ARTIST, artist.id, artist.starred)
                FavoriteCircle(artistStarred) { actions.starArtist(artist.id, !artistStarred); Unit }
            }
            LateMore(ui) { ready ->
                listOfNotNull(
                    say.addToQueue to { actions.queueArtist(ready.detail.artist.id); Unit },
                    (say.downloadEverything to { actions.downloadArtist(ready.detail.artist.id); Unit }).takeIf { downloads },
                )
            }
        },
    ) {
        when {
            ui != null -> {
                ui.info?.biography?.let { bio ->
                    item(key = "bio", contentType = "bio") { Biography(remember(bio) { dev.nori.music.ffi.library.biography(bio) }, late(arrival)) }
                }
                if (ui.info?.lastFmUrl != null || ui.info?.musicBrainzId != null) item(key = "links", contentType = "links") {
                    Row(late(arrival).padding(horizontal = 12.dp)) {
                        ui.info?.lastFmUrl?.let { u -> TextAction("last.fm", { leave(u) }) }
                        ui.info?.musicBrainzId?.let { m -> TextAction("MusicBrainz", { leave(dev.nori.music.ffi.library.musicbrainzArtistUrl(m)) }) }
                    }
                }
                groups.forEachIndexed { n, (group, albums) ->
                    item(key = "group-$n", contentType = "shelf") {
                        Column(late(arrival)) {
                            SectionTitle(group)
                            LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                items(albums, key = { it.id }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 120.dp, { nav.album(a.id, a) }) }
                            }
                        }
                    }
                }
                // The songs the artist's provider lists on the page (YouTube Music's); a server lists none.
                if (ui.detail.songs.isNotEmpty()) {
                    val total = ui.detail.songs.size.toUInt()
                    val shown = dev.nori.music.ffi.library.songsShown(total, allSongs)
                    item(key = "songs", contentType = "title") { SectionTitle(say.songs, late(arrival)) }
                    songRows(
                        ui.detail.songs, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, keyPrefix = "songs", appear = true,
                        arrival = arrival, from = topFrom, rows = if (shown < total) (0u until shown).toList() else null,
                    )
                    if (shown < total) item(key = "songs-all", contentType = "more") {
                        TextAction(say.showAll, { allSongs = true }, Modifier.padding(horizontal = 8.dp))
                    }
                }
                if (ui.top.isNotEmpty()) {
                    item(key = "top", contentType = "title") { SectionTitle(say.topSongs, late(arrival)) }
                    // The top songs are a list of their own: played from a row they are not the page's Play.
                    songRows(
                        ui.top, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, keyPrefix = "top", appear = true,
                        arrival = arrival, from = topFrom,
                    )
                }
                if (similar.isNotEmpty()) item(key = "similar", contentType = "similar") {
                    Column(late(arrival)) {
                        SectionTitle(say.similarArtists)
                        LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            items(similar, key = { it.id }) { a ->
                                Text(a.name, Modifier.clickable { nav.artist(a.id, a) }.padding(8.dp), color = MaterialTheme.colorScheme.primary)
                            }
                        }
                    }
                }
            }
            failed != null -> item(key = "fail") { LoadFailed(failed) }
            else -> Unit
        }
    }
}

@Composable
fun PlaylistScreen(id: String, actions: ActionsViewModel, vm: PlaylistViewModel = viewModel()) {
    LaunchedEffect(id) { vm.open(id) }
    val load by vm.ui.collectAsStateWithLifecycle()
    var filter by remember { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val detail = (load as? Load.Ready)?.data
    val exportM3u = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.CreateDocument("audio/x-mpegurl")) { uri ->
        if (uri != null && detail != null) runCatching { context.contentResolver.openOutputStream(uri)?.use { it.write(actions.exportM3u(detail.playlist.name, detail.songs).toByteArray()) } }
    }
    val export = { name: String -> exportM3u.launch(dev.nori.music.ffi.library.m3uFileName(name)) }
    val hint = LocalNav.current.playlistHint(id)
    if (hint == null) {
        LoadBox(load) { d -> PlaylistPage(id, d.playlist, d, null, actions, vm, filter, { filter = it }, export) }
        return
    }
    PlaylistPage(id, detail?.playlist ?: hint, detail, (load as? Load.Failed)?.message, actions, vm, filter, { filter = it }, export)
}

/** A playlist's page: [detail] null while the songs are on the wire, [failed] when they did not come. */
@Composable
private fun PlaylistPage(
    id: String,
    playlist: dev.nori.music.ffi.model.Playlist,
    detail: PlaylistDetail?,
    failed: String?,
    actions: ActionsViewModel,
    vm: PlaylistViewModel,
    filter: String,
    onFilter: (String) -> Unit,
    export: (playlistName: String) -> Unit,
) {
    val settings: SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val list = rememberSongListState(actions)
    val arrival = rememberArrival(detail != null)
    val shown = rememberMatching(detail?.songs.orEmpty(), filter)
    val from = rememberOrigin(OriginKind.PLAYLIST, id)
    HeroPage(
        coverUrl = vm.cover(playlist.coverArt, CoverSize.FULL),
        coverStandIns = listOf(vm.cover(playlist.coverArt, CoverSize.CARD), vm.cover(playlist.coverArt, CoverSize.ROW)),
        title = playlist.name,
        subtitle = remember(playlist.comment, prefs.playlistNotes) { dev.nori.music.ffi.library.playlistDescription(playlist.comment, prefs.playlistNotes) },
        caption = remember(detail, playlist) { detail?.let { say.listCaption(it.songs.size, it.seconds.toLong(), true) } ?: say.albumHintCaption(0, playlist.songCount.toInt(), playlist.duration.toLong()) },
        onPlay = detail?.let { d -> { actions.play(d.songs, from = from) } },
        onShuffle = detail?.let { d -> { actions.shuffle(d.songs, from) } },
        awaitingPlay = detail == null && failed == null,
        // Its own queue only: a song it shares with another playlist playing from there leaves it alone.
        queue = rememberPageQueue(from, detail?.queue),
        actions = {
            val pinned = id in prefs.pinnedPlaylists
            // A favourite, drawn and named as every other favourite in the app is: a heart, filled
            // when it is one. The server has no way to star a playlist, so it is kept on this phone,
            // and the home page's shelf of them reads it.
            // The home page that shows them is the account's.
            val account = LocalRules.current.account
    val downloads = LocalRules.current.downloads
            if (account) FavoriteCircle(pinned) { settings.pin(id, !pinned); Unit }
            LateMore(detail) { d ->
                listOfNotNull(
                    say.addToQueue to { actions.enqueue(d.songs) },
                    downloadEntry(d.songs, list.downloaded, actions).takeIf { downloads },
                    jamStartEntry { actions.play(d.songs, from = from) },
                    say.exportPlaylistFile to { export(playlist.name) },
                )
            }
        },
    ) {
        when {
            detail != null -> {
                item(key = "filter", contentType = "filter") { Box(Modifier.arriving(arrival)) { FilterField(detail.songs.size, filter, onFilter) } }
                // A row each: a playlist of a thousand songs composes the dozen on screen, not all of them.
                songRows(
                    detail.songs, actions, list,
                    cover = { vm.cover(it.coverArt, CoverSize.ROW) }, rows = shown, arrival = arrival, from = from,
                )
            }
            failed != null -> item(key = "fail") { LoadFailed(failed) }
            else -> Unit
        }
    }
}

@Composable
fun GenreScreen(name: String, actions: ActionsViewModel, vm: GenreViewModel = viewModel()) {
    LaunchedEffect(name) { vm.open(name) }
    val load by vm.ui.collectAsStateWithLifecycle()
    val list = rememberSongListState(actions)
    val from = rememberOrigin(OriginKind.GENRE, name)
    LoadBox(load) { songs ->
        SongListPage(name, remember(songs.size) { say.songs(songs.size) }, songs, list.downloaded, from, actions) {
            songRows(songs, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, from = from)
        }
    }
}

/** One level of the server's folder tree: subfolders first, then the files in it. */
@Composable
fun FolderScreen(id: String, actions: ActionsViewModel, vm: FolderViewModel = viewModel()) {
    LaunchedEffect(id) { vm.open(id) }
    val load by vm.ui.collectAsStateWithLifecycle()
    val list = rememberSongListState(actions)
    val nav = LocalNav.current
    val from = rememberOrigin(OriginKind.FOLDER, id)
    LoadBox(load) { d ->
        SongListPage(remember(d.name) { say.folderTitle(d.name) }, remember(d) { say.folderCaption(d.folders.size, d.songs.size) }, d.songs, list.downloaded, from, actions) {
            items(d.folders, key = { "f" + it.id }) { f -> NavRow(remember(f.name) { say.folderRow(f.name) }, { nav.folder(f.id) }, chevron = true) }
            songRows(d.songs, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, from = from)
        }
    }
}
