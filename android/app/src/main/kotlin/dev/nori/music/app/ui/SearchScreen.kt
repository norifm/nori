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
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.graphicsLayer
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.SearchViewModel
import dev.nori.music.app.vm.results
import androidx.compose.runtime.remember

@Composable
fun SearchScreen(actions: ActionsViewModel, vm: SearchViewModel = viewModel()) {
    val ui by vm.ui.collectAsStateWithLifecycle()
    val downloads by actions.downloads.collectAsState()
    val nav = LocalNav.current
    val menu = LocalSongMenu.current
    val scopes = remember { dev.nori.music.ffi.library.searchScopes().map { it to say.searchScope(it) } }
    val focus = androidx.compose.ui.platform.LocalFocusManager.current
    // Opening the page shows what an empty search offers; Search tapped again, or the field, brings the keyboard.
    val taps = searchFocusKey()
    val opened = remember { taps }
    Column {
        LargeTitle(say.search)
        SearchField(
            ui.query, vm::setQuery, say.searchHint,
            Modifier.padding(start = Space.gutter, end = Space.gutter, top = 8.dp), testTag = "search", autofocus = taps != opened,
            focusKey = taps, onSearch = { vm.remember() },
        )
        // Its room is kept whether or not it shows, so the results do not move as the server answers.
        androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().height(2.dp)) {
            if (ui.answer is dev.nori.music.ffi.library.SearchShown.Asking) LinearProgressIndicator(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.onSurface, trackColor = MaterialTheme.colorScheme.outlineVariant)
        }
        (ui.answer as? dev.nori.music.ffi.library.SearchShown.Failed)?.let { f ->
            val said = remember(f) { say.searchFallback(f.fallback.reason) }
            Text(said, Modifier.padding(horizontal = Space.gutter), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        // The kind chips, All first, then the scope chips where providers' results are mixed in: picking one
        // shows it in place. As far below the field as the library's chips are below its title, the progress
        // line's room counted.
        if (ui.answer.results != null && ui.kinds.isNotEmpty()) LazyRow(
            Modifier.bleedsToEdges().padding(top = 8.dp).testTag("search-kinds"), contentPadding = edgePadding(horizontal = Space.gutter),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            item(key = "all") { Chip(say.searchAll, ui.kind == null) { vm.pick(null) } }
            items(ui.kinds, key = { it.name }) { k -> Chip(say.searchKind(k), ui.kind == k) { vm.pick(k) } }
            if (ui.scopesOffered) {
                item(key = "gap") { Box(Modifier.padding(horizontal = 4.dp).width(HAIRLINE).height(20.dp).background(MaterialTheme.colorScheme.outlineVariant)) }
                items(scopes, key = { it.first.name }) { (scope, label) -> Chip(label, ui.scope == scope) { vm.narrow(scope) } }
            }
        }
        // Back from a kind picked comes back to All; from a search, to this page's start, not to the tab before it.
        androidx.activity.compose.BackHandler(ui.kind != null) { vm.pick(null) }
        androidx.activity.compose.BackHandler(ui.query.isNotEmpty() && ui.kind == null) { vm.setQuery(""); focus.clearFocus() }
        // A first answer on its way: its results' places, the answer fading in over them.
        val reading = ui.answer is dev.nori.music.ffi.library.SearchShown.Asking && ui.answer.results == null
        LoadBox(if (reading) dev.nori.music.app.vm.Load.Loading else dev.nori.music.app.vm.Load.Ready(Unit), sketch = { PageSketch(dev.nori.music.ffi.library.Reading.SEARCH) }) {
        val r = ui.answer.results
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
            return@LoadBox
        }
        if (ui.answer is dev.nori.music.ffi.library.SearchShown.NothingFound) return@LoadBox EmptyNote(Note.NOTHING_FOUND)
        val play = { song: dev.nori.music.ffi.model.Song ->
            // One tap plays one song: a search result list is not an album, and with octo-fiesta queueing
            // the rest would make the server download every provider track in it.
            vm.remember(); actions.play(listOf(song), from = dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SEARCH, ui.query))
        }
        val (onRight, onLeft) = actions.swipes
        // The same two swipes every other list of songs has: a result is a song like any other.
        val songRow: @Composable (dev.nori.music.ffi.model.Song) -> Unit = { s ->
            SongRow(
                s, vm.cover(s.coverArt, CoverSize.ROW), onClick = { play(s) }, onMenu = { menu(s) },
                downloaded = s.id in downloads.doneIds,
                swipeRight = rowSwipe(onRight, s, actions), swipeLeft = rowSwipe(onLeft, s, actions),
            )
        }
        val picked = ui.kind
        if (picked != null && picked != dev.nori.music.ffi.library.SearchKind.SONGS) return@LoadBox WholeKind(picked, r, vm)
        val list = rememberLazyListState()
        androidx.compose.runtime.LaunchedEffect(ui.query, ui.scope, ui.kind) { list.requestScrollToItem(0) }
        LazyColumn(Modifier.pageToEdges().fadesUnderTitle(list), list, contentPadding = pagePadding(LocalChromeInset.current)) {
            if (picked == dev.nori.music.ffi.library.SearchKind.SONGS) {
                items(r.songs, key = { "s-" + it.id }, contentType = { "song" }) { s -> songRow(s) }
                return@LazyColumn
            }
            val layout = ui.layout
            fun items(kind: dev.nori.music.ffi.library.SearchKind) = layout.sections.firstOrNull { it.kind == kind }?.items.orEmpty().map { it.toInt() }
            layout.top?.let { t -> item(key = "top") { SectionTitle(say.topResult); TopResultCard(t, r, vm, actions, play) } }
            val songs = items(dev.nori.music.ffi.library.SearchKind.SONGS)
            if (songs.isNotEmpty()) item(key = "songs") { SectionTitle(say.songs) }
            items(songs.map { r.songs[it] }, key = { "s-" + it.id }, contentType = { "song" }) { s -> songRow(s) }
            val albums = items(dev.nori.music.ffi.library.SearchKind.ALBUMS).map { r.albums[it] }
            if (albums.isNotEmpty()) item(key = "albums") {
                SectionTitle(say.albums)
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(albums.first().id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(albums, key = { it.id }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 120.dp, { vm.remember(); nav.album(a.id, a) }, itemMotion()) }
                }
            }
            val artists = items(dev.nori.music.ffi.library.SearchKind.ARTISTS).map { r.artists[it] }
            if (artists.isNotEmpty()) item(key = "artists") {
                SectionTitle(say.artists)
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(artists.first().id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(artists, key = { it.id }) { a -> FoundArtist(a, vm, 96.dp, itemMotion()) }
                }
            }
            val playlists = items(dev.nori.music.ffi.library.SearchKind.PLAYLISTS).map { r.playlists[it] }
            if (playlists.isNotEmpty()) item(key = "playlists") {
                SectionTitle(say.playlists)
                LazyRow(Modifier.bleedsToEdges(), rememberShelfState(playlists.first().id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(playlists, key = { it.id }) { p -> FoundPlaylist(p, vm, itemMotion().width(120.dp)) }
                }
            }
        }
        }
    }
}

/** A kind picked whole, as a grid under the chips: artists three across, the rest two. */
@Composable
private fun WholeKind(kind: dev.nori.music.ffi.library.SearchKind, r: dev.nori.music.ffi.model.SearchResult, vm: SearchViewModel) {
    val nav = LocalNav.current
    val grid = androidx.compose.foundation.lazy.grid.rememberLazyGridState()
    val artists = kind == dev.nori.music.ffi.library.SearchKind.ARTISTS
    androidx.compose.foundation.lazy.grid.LazyVerticalGrid(
        androidx.compose.foundation.lazy.grid.GridCells.Fixed(if (artists) 3 else 2), Modifier.fadesUnderTitle(grid), grid,
        contentPadding = androidx.compose.foundation.layout.PaddingValues(start = Space.gutter, end = Space.gutter, top = 12.dp, bottom = Space.gutter + LocalChromeInset.current),
        horizontalArrangement = Arrangement.spacedBy(GRID_GAP), verticalArrangement = Arrangement.spacedBy(GRID_GAP),
    ) {
        when (kind) {
            dev.nori.music.ffi.library.SearchKind.ARTISTS -> items(r.artists.size, key = { r.artists[it].id }) { i ->
                androidx.compose.foundation.layout.BoxWithConstraints { FoundArtist(r.artists[i], vm, maxWidth) }
            }
            dev.nori.music.ffi.library.SearchKind.ALBUMS -> items(r.albums.size, key = { r.albums[it].id }) { i ->
                val a = r.albums[i]
                AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 0.dp, { vm.remember(); nav.album(a.id, a) }, Modifier.fillMaxWidth(), fill = true)
            }
            else -> items(r.playlists.size, key = { r.playlists[it].id }) { i -> FoundPlaylist(r.playlists[i], vm, Modifier.fillMaxWidth(), fill = true) }
        }
    }
}

/**
 * The search's top result, as Home's Continue card: raised paper, the cover running into it, what it is in
 * small capitals, its name in Literata, its line and Play. A tap opens it (a song plays).
 */
@Composable
private fun TopResultCard(t: dev.nori.music.ffi.library.SearchItem, r: dev.nori.music.ffi.model.SearchResult, vm: SearchViewModel, actions: ActionsViewModel, play: (dev.nori.music.ffi.model.Song) -> Unit) {
    val nav = LocalNav.current
    val at = t.at.toInt()
    data class Top(val name: String, val line: String, val cover: String?, val remote: Boolean, val open: () -> Unit, val playIt: () -> Unit)
    val top = when (t.kind) {
        dev.nori.music.ffi.library.SearchKind.SONGS -> r.songs[at].let { s -> Top(say.title(s.title), s.artist, s.coverArt, s.isExternal, { play(s) }, { play(s) }) }
        dev.nori.music.ffi.library.SearchKind.ALBUMS -> r.albums[at].let { a -> Top(a.name, a.subtitle, a.coverArt, a.isExternal, { vm.remember(); nav.album(a.id, a) }, { vm.remember(); actions.play(dev.nori.music.app.vm.CardItem.OfAlbum(a)) }) }
        dev.nori.music.ffi.library.SearchKind.ARTISTS -> r.artists[at].let { a ->
            Top(a.name, a.albumCount?.takeIf { it > 0u }?.let { say.albums(it.toInt()) }.orEmpty(), a.coverArt, a.isExternal, { vm.remember(); nav.artist(a.id, a) }, { vm.remember(); actions.play(dev.nori.music.app.vm.CardItem.OfArtist(a)) })
        }
        dev.nori.music.ffi.library.SearchKind.PLAYLISTS -> r.playlists[at].let { p ->
            Top(p.name, if (p.songCount > 0u) say.songs(p.songCount.toInt()) else p.comment.orEmpty(), p.coverArt, false, { vm.remember(); nav.playlist(p.id, p) }, { vm.remember(); actions.play(dev.nori.music.app.vm.CardItem.OfPlaylist(p)) })
        }
    }
    val scheme = MaterialTheme.colorScheme
    val page = scheme.surfaceContainerHigh
    val ink = scheme.onSurface
    val url = vm.cover(top.cover, CoverSize.CARD)
    Box(
        Modifier.fillMaxWidth().padding(horizontal = Space.gutter).height(TOP_HEIGHT).clip(TileShape).background(page)
            .clickable(onClick = top.open).testTag("search-top"),
    ) {
        Row {
            Box(Modifier.size(TOP_HEIGHT)) {
                Cover(url, TOP_HEIGHT, radius = 0.dp, plate = false)
                Box(Modifier.matchParentSize().background(androidx.compose.ui.graphics.Brush.horizontalGradient(0.55f to androidx.compose.ui.graphics.Color.Transparent, 1f to page)))
            }
            Column(Modifier.weight(1f).fillMaxHeight().padding(start = 6.dp, top = 16.dp, end = 16.dp, bottom = 14.dp), Arrangement.SpaceBetween) {
                Column {
                    Text(
                        remember(t.kind) { say.topKind(t.kind).uppercase() }, style = MaterialTheme.typography.labelSmall.copy(fontSize = 11.5.sp, letterSpacing = 0.7.sp),
                        color = ink.copy(alpha = 0.65f),
                    )
                    Text(
                        top.name, Modifier.padding(top = 6.dp),
                        style = titled(MaterialTheme.typography.titleLarge.copy(fontSize = 21.sp, lineHeight = 24.sp, fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold)),
                        color = ink, maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                    )
                    RemoteLine(top.line, top.remote, MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp), Modifier.padding(top = 2.dp), color = ink.copy(alpha = 0.7f))
                }
                Row(
                    Modifier.height(38.dp).clip(PillShape).background(ink).clickable(onClick = top.playIt).padding(horizontal = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    androidx.compose.material3.Icon(Iconoir.PlaySolid, null, Modifier.size(18.dp), tint = page)
                    Text(say.play, Modifier.padding(start = 6.dp), style = MaterialTheme.typography.labelLarge.copy(fontSize = 14.sp), color = page)
                }
            }
        }
    }
}

private val TOP_HEIGHT = 178.dp

/** An artist found: how many albums, and the cloud for a provider's, so two of one name are told apart. */
@Composable
private fun FoundArtist(a: dev.nori.music.ffi.model.Artist, vm: SearchViewModel, size: androidx.compose.ui.unit.Dp, modifier: Modifier = Modifier) {
    val nav = LocalNav.current
    val cardMenu = LocalCardMenu.current
    ArtistCard(
        a.name, remember(a.albumCount) { a.albumCount?.takeIf { it > 0u }?.let { say.albums(it.toInt()) }.orEmpty() }, vm.cover(a.coverArt, CoverSize.ROW), size,
        onClick = { vm.remember(); nav.artist(a.id, a) }, modifier = modifier, remote = a.isExternal, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfArtist(a)) },
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
