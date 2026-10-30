package dev.nori.music.app.ui

import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.graphicsLayer
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.SearchViewModel
import androidx.compose.runtime.remember

@Composable
fun SearchScreen(actions: ActionsViewModel, vm: SearchViewModel = viewModel()) {
    val ui by vm.ui.collectAsStateWithLifecycle()
    val downloads by actions.downloads.collectAsState()
    val nav = LocalNav.current
    val menu = LocalSongMenu.current
    val scopes = remember { dev.nori.music.ffi.library.searchScopes().map { it to say.searchScope(it) } }
    // Opening the page shows what an empty search offers; Search tapped again, or the field, brings the keyboard.
    val taps = searchFocusKey()
    val opened = remember { taps }
    Column {
        LargeTitle(say.search)
        SearchField(
            ui.query, vm::setQuery, say.searchHint,
            Modifier.padding(start = Space.gutter, end = Space.gutter, top = 8.dp), testTag = "search", autofocus = taps != opened,
            focusKey = taps,
        )
        // Its room is kept whether or not it shows, so the results do not move as the server answers.
        androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().height(2.dp)) {
            if (ui.searching) LinearProgressIndicator(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.onSurface, trackColor = MaterialTheme.colorScheme.outlineVariant)
        }
        ui.error?.let { Text(it, Modifier.padding(horizontal = Space.gutter), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }

        if (ui.scopesOffered) LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(scopes) { (scope, label) -> Chip(label, ui.scope == scope) { vm.setScope(scope) } }
        }
        // Back from a search comes back to this page's start, not to the tab before it.
        val focus = androidx.compose.ui.platform.LocalFocusManager.current
        androidx.activity.compose.BackHandler(ui.query.isNotEmpty()) { vm.setQuery(""); focus.clearFocus() }
        val r = ui.shown
        // A new search starts with its best few songs again.
        var allSongs by androidx.compose.runtime.remember(ui.query) { androidx.compose.runtime.mutableStateOf(false) }
        if (r == null) {
            // Before a search: the recent searches, then ways in that need no words, the library's biggest
            // genres and its decades.
            val browse by vm.browse.collectAsStateWithLifecycle()
            val scroll = rememberLazyListState()
            // The last shelf stops a gutter short of the chrome rather than against it.
            LazyColumn(Modifier.pageToEdges().fadesUnderTitle(scroll), scroll, contentPadding = pagePadding(LocalChromeInset.current + Space.gutter)) {
                if (ui.history.isNotEmpty()) item(key = "recent") {
                    // Clear all is a word in the header's line, not a button as tall as a row.
                    SectionHeader(say.recentSearches, action = {
                        Text(say.clearAll, Modifier.clip(PillShape).clickable(onClick = vm::clearHistory).padding(horizontal = 12.dp, vertical = 4.dp), style = MaterialTheme.typography.labelLarge)
                    })
                }
                itemsIndexed(ui.history, key = { _, q -> "q-$q" }) { i, q -> RecentSearch(q, i == 0, { vm.setQuery(q) }, { vm.forgetSearch(q) }, Modifier.animateItem(fadeInSpec = null, fadeOutSpec = null)) }
                if (browse.genres.isNotEmpty()) {
                    item(key = "genres") { SectionTitle(say.librarySection(dev.nori.music.ffi.settings.LibrarySection.GENRES), onMore = { nav.librarySection(dev.nori.music.ffi.settings.LibrarySection.GENRES) }) }
                    items(browse.genres.chunked(2), key = { "g-" + it.first().name }) { pair ->
                        Row(Modifier.padding(start = Space.gutter, end = Space.gutter, bottom = GRID_GAP), horizontalArrangement = Arrangement.spacedBy(GRID_GAP)) {
                            pair.forEach { g -> GenreCard(g, vm.cover(g.coverArt, CoverSize.ROW), { nav.genre(g.name) }, Modifier.weight(1f)) }
                            if (pair.size == 1) androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
                        }
                    }
                }
                if (browse.decades.isNotEmpty()) item(key = "decades") {
                    SectionTitle(say.decades, onMore = { nav.librarySection(dev.nori.music.ffi.settings.LibrarySection.DECADES) })
                    LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        items(browse.decades, key = { it.start.toInt() }) { d -> Chip(remember(d.start) { say.decade(d.start.toInt()) }, selected = true) { nav.decade(d.start.toInt()) } }
                    }
                }
            }
            return@Column
        }
        // A fuller answer puts artists and albums above the songs: the list stays at its top for them, as a
        // shelf does, and a new search or scope starts there.
        val first = when {
            r.artists.isNotEmpty() -> "artists"
            r.albums.isNotEmpty() -> "albums"
            r.playlists.isNotEmpty() -> "playlists"
            else -> "songs"
        }
        // A shelf's › opens it whole, as a grid in its place, until Back; a new search or scope closes it.
        var whole by remember(ui.query, ui.scope) { androidx.compose.runtime.mutableStateOf<Whole?>(null) }
        androidx.activity.compose.BackHandler(whole != null) { whole = null }
        whole?.let { return@Column WholeShelf(it, r, vm) }
        val list = rememberShelfState(first)
        androidx.compose.runtime.LaunchedEffect(ui.query, ui.scope) { list.requestScrollToItem(0) }
        LazyColumn(Modifier.pageToEdges().fadesUnderTitle(list), list, contentPadding = pagePadding(LocalChromeInset.current)) {
            if (r.artists.isNotEmpty()) item(key = "artists") {
                SectionTitle(say.artists, onMore = { whole = Whole.ARTISTS }.takeIf { r.artists.size > SHELF_SEEN })
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(r.artists.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(r.artists, key = { it.id }) { a -> FoundArtist(a, vm, 96.dp) }
                }
            }
            if (r.albums.isNotEmpty()) item(key = "albums") {
                SectionTitle(say.albums, onMore = { whole = Whole.ALBUMS }.takeIf { r.albums.size > SHELF_SEEN })
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(r.albums.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(r.albums, key = { it.id }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 120.dp, { vm.remember(); nav.album(a.id, a) }) }
                }
            }
            if (r.playlists.isNotEmpty()) item(key = "playlists") {
                SectionTitle(say.playlists, onMore = { whole = Whole.PLAYLISTS }.takeIf { r.playlists.size > SHELF_SEEN })
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(r.playlists.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(r.playlists, key = { it.id }) { p -> FoundPlaylist(p, vm, Modifier.width(120.dp)) }
                }
            }
            if (r.songs.isNotEmpty()) item(key = "songs") { SectionTitle(say.songs) }
            val shown = dev.nori.music.ffi.library.songsShown(r.songs.size.toUInt(), allSongs).toInt()
            itemsIndexed(r.songs.take(shown), key = { _, s -> s.id }, contentType = { _, _ -> "song" }) { _, s ->
                // One tap plays one song: a search result list is not an album, and with octo-fiesta
                // queueing the rest would make the server download every provider track in it.
                // The same two swipes every other list of songs has: a result is a song like any other,
                // and reaching for the menu to queue one was the odd thing out here.
                val (onRight, onLeft) = actions.swipes
                SongRow(
                    s, vm.cover(s.coverArt, CoverSize.ROW), onClick = { vm.remember(); actions.play(listOf(s), from = dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SEARCH, ui.query)) }, onMenu = { menu(s) },
                    downloaded = s.id in downloads.doneIds,
                    swipeRight = rowSwipe(onRight, s, actions), swipeLeft = rowSwipe(onLeft, s, actions),
                )
            }
            if (shown < r.songs.size) item(key = "songs-all") { TextAction(say.showAll, { allSongs = true }, Modifier.padding(horizontal = 8.dp)) }
            if (ui.nothingFound) item { EmptyNote(Note.NOTHING_FOUND) }
        }
    }
}

/** How many cards of a shelf a phone shows across: a shelf longer than that can be opened whole. */
private const val SHELF_SEEN = 3

/** A shelf of the answer that can be opened whole. */
private enum class Whole { ARTISTS, ALBUMS, PLAYLISTS }

/** One of the answer's shelves whole, as a grid under its name: artists three across, the rest two. */
@Composable
private fun WholeShelf(whole: Whole, r: dev.nori.music.ffi.model.SearchResult, vm: SearchViewModel) {
    val nav = LocalNav.current
    val grid = androidx.compose.foundation.lazy.grid.rememberLazyGridState()
    val columns = if (whole == Whole.ARTISTS) 3 else 2
    SectionTitle(when (whole) { Whole.ARTISTS -> say.artists; Whole.ALBUMS -> say.albums; Whole.PLAYLISTS -> say.playlists })
    androidx.compose.foundation.lazy.grid.LazyVerticalGrid(
        androidx.compose.foundation.lazy.grid.GridCells.Fixed(columns), Modifier.fadesUnderTitle(grid), grid,
        contentPadding = androidx.compose.foundation.layout.PaddingValues(start = Space.gutter, end = Space.gutter, bottom = Space.gutter + LocalChromeInset.current),
        horizontalArrangement = Arrangement.spacedBy(GRID_GAP), verticalArrangement = Arrangement.spacedBy(GRID_GAP),
    ) {
        when (whole) {
            Whole.ARTISTS -> items(r.artists.size, key = { r.artists[it].id }) { i ->
                androidx.compose.foundation.layout.BoxWithConstraints { FoundArtist(r.artists[i], vm, maxWidth) }
            }
            Whole.ALBUMS -> items(r.albums.size, key = { r.albums[it].id }) { i ->
                val a = r.albums[i]
                AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 0.dp, { vm.remember(); nav.album(a.id, a) }, Modifier.fillMaxWidth(), fill = true)
            }
            Whole.PLAYLISTS -> items(r.playlists.size, key = { r.playlists[it].id }) { i -> FoundPlaylist(r.playlists[i], vm, Modifier.fillMaxWidth(), fill = true) }
        }
    }
}

/** An artist found: how many albums, and the cloud for a provider's, so two of one name are told apart. */
@Composable
private fun FoundArtist(a: dev.nori.music.ffi.model.Artist, vm: SearchViewModel, size: androidx.compose.ui.unit.Dp) {
    val nav = LocalNav.current
    val cardMenu = LocalCardMenu.current
    ArtistCard(
        a.name, remember(a.albumCount) { a.albumCount?.takeIf { it > 0u }?.let { say.albums(it.toInt()) }.orEmpty() }, vm.cover(a.coverArt, CoverSize.ROW), size,
        onClick = { vm.remember(); nav.artist(a.id, a) }, remote = a.isExternal, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfArtist(a)) },
    )
}

/** A playlist found. A provider's is not counted until opened: its own line (who made it) instead. */
@Composable
private fun FoundPlaylist(p: dev.nori.music.ffi.model.Playlist, vm: SearchViewModel, modifier: Modifier, fill: Boolean = false) {
    val nav = LocalNav.current
    val cardMenu = LocalCardMenu.current
    val line = remember(p.songCount, p.comment) { if (p.songCount > 0u) say.songs(p.songCount.toInt()) else p.comment.orEmpty() }
    CoverCard(p.name, line, vm.cover(p.coverArt, CoverSize.CARD), 120.dp, { vm.remember(); nav.playlist(p.id, p) }, modifier, fill = fill, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfPlaylist(p)) })
}

/** The tile a genre is browsed from: its name in Literata on the surface, a cover of its songs running into the paper at the end. */
@Composable
private fun GenreCard(g: dev.nori.music.ffi.library.GenreTile, cover: String?, onClick: () -> Unit, modifier: Modifier) {
    val scheme = MaterialTheme.colorScheme
    androidx.compose.material3.Surface(onClick, modifier.height(GENRE_TILE), shape = TileShape, color = scheme.surfaceContainer, contentColor = scheme.onSurface, border = hairlineBorder()) {
        androidx.compose.foundation.layout.Box {
            androidx.compose.foundation.layout.Box(
                Modifier.align(Alignment.CenterEnd).size(GENRE_TILE)
                    .graphicsLayer { compositingStrategy = androidx.compose.ui.graphics.CompositingStrategy.Offscreen }
                    .drawWithContent { drawContent(); drawRect(COVER_FADE, blendMode = androidx.compose.ui.graphics.BlendMode.DstIn) },
            ) { Cover(cover, GENRE_TILE, radius = 0.dp) }
            Column(Modifier.align(Alignment.CenterStart).padding(start = 14.dp, end = GENRE_TILE / 2)) {
                Text(
                    g.name, style = titled(MaterialTheme.typography.titleMedium.copy(fontSize = 17.sp, fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold)),
                    maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
                Text(remember(g.songCount) { say.songs(g.songCount.toInt()) }, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1)
            }
        }
    }
}

private val GENRE_TILE = 72.dp
private val COVER_FADE = androidx.compose.ui.graphics.Brush.horizontalGradient(0f to androidx.compose.ui.graphics.Color.Transparent, 0.75f to androidx.compose.ui.graphics.Color.Black)

/** A recent search: tapped, it searches again; swiped either way, it is forgotten. */
/** [first]: right under its header, as a shelf's cards sit under theirs, without the room between rows above it. */
@Composable
private fun RecentSearch(query: String, first: Boolean, onClick: () -> Unit, onForget: () -> Unit, modifier: Modifier) {
    val forget = RowSwipe(Iconoir.Xmark, say.remove, onForget)
    val swipe = remember { SwipeState() }
    Box(modifier.fillMaxWidth()) {
        SwipeBackdrop(swipe, forget, forget, Modifier.matchParentSize())
        Row(
            Modifier.fillMaxWidth().swipeable(swipe, forget, forget, MaterialTheme.colorScheme.background, gone = true)
                .clickable(onClick = onClick).padding(start = Space.gutter, end = Space.gutter, top = if (first) 0.dp else 9.dp, bottom = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            androidx.compose.material3.Icon(Iconoir.ClockRotateRight, null, Modifier.padding(end = 14.dp).size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(query, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyLarge)
        }
    }
}
