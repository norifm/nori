package dev.nori.music.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.AlbumsViewModel
import dev.nori.music.app.vm.ArtistsViewModel
import dev.nori.music.app.vm.DecadesViewModel
import dev.nori.music.app.vm.FoldersViewModel
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.settings.LibrarySection
import dev.nori.music.app.vm.LibraryHomeViewModel
import dev.nori.music.app.vm.SongSort
import dev.nori.music.app.vm.SongsViewModel
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.snapshotFlow
import dev.nori.music.app.vm.GenresViewModel
import dev.nori.music.app.vm.PlaylistsViewModel
import dev.nori.music.app.vm.RadioViewModel
import dev.nori.music.app.vm.StarredViewModel

/** The gap between the library's stacked parts: the pills' own (the strip of sections, the places, the filter, the order). */
private val LIST_GAP = 10.dp

/** The album orders as pills, as the songs have theirs: Favorites lit or not over them, one order lit, a tap chooses another. */
@Composable
private fun AlbumSortPills(vm: AlbumsViewModel) {
    val sort by vm.sort.collectAsStateWithLifecycle()
    val favorites by vm.favorites.collectAsStateWithLifecycle()
    val stars = LocalRules.current.stars
    val sorts = remember { dev.nori.music.ffi.library.albumSorts().map { it to say.albumSort(it) } }
    LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (stars) item(key = "favorites") { Chip(say.starredFavourites, favorites) { vm.setFavorites(!favorites) } }
        items(sorts, key = { it.first.name }) { (v, label) -> Chip(label, sort == v) { vm.setSort(v) } }
    }
}

/**
 * The library: its title, a strip of chips for the sections kept in Library sections and the one
 * chosen shown below it, with the places (Favorites, Downloads, History) at the top of each. The chosen
 * section is remembered; a deep link ([initial]) chooses one. Smart playlists are under Playlists.
 */
@Composable
fun LibraryScreen(actions: ActionsViewModel, initial: LibrarySection? = null, vm: LibraryHomeViewModel = viewModel()) {
    val rules = LocalRules.current
    val settings: dev.nori.music.app.vm.SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val sections = remember(prefs.librarySections, rules.sections) { dev.nori.music.ffi.library.librarySectionsShown(prefs.librarySections, rules.sections) }
    val chips = remember(sections) { sections.filter { it.onLibraryPage() } }
    var picked by rememberSaveable { mutableStateOf(initial?.name) }
    val shown = picked?.let(LibrarySection::valueOf)?.takeIf { it in chips } ?: chips.firstOrNull()
    // Only the shown section's view model is made, so only it loads anything.
    val albums = if (shown == LibrarySection.ALBUMS) viewModel<AlbumsViewModel>() else null
    val shuffle: (() -> Unit)? = when (shown) {
        LibrarySection.ALBUMS -> ({ actions.shuffleAlbums() })
        LibrarySection.SONGS -> ({ actions.shuffleAll() })
        else -> null
    }
    val places: @Composable () -> Unit = { Places(vm, favorites = LibrarySection.FAVOURITES in sections, shuffle = shuffle) }
    Column {
        LargeTitle(say.library)
        LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter, vertical = LIST_GAP), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            items(chips, key = { it.name }) { s -> Chip(say.librarySection(s), s == shown) { picked = s.name } }
        }
        // A profile with no library (a provider's, signed out) is found through search.
        if (rules.sections.isEmpty()) EmptyNote(say.searchIsTheWayIn, icon = Iconoir.Search)
        when (shown) {
            LibrarySection.ALBUMS -> Albums(albums!!, places)
            LibrarySection.ARTISTS -> Artists(places)
            LibrarySection.SONGS -> SongsScreen(actions, null, places)
            LibrarySection.PLAYLISTS -> Playlists(actions, places)
            LibrarySection.GENRES -> Genres(places)
            LibrarySection.DECADES -> Decades(places)
            LibrarySection.FOLDERS -> Folders(places)
            LibrarySection.RADIO -> Radio(places)
            null -> {
                val scroll = rememberLazyListState()
                LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) { item { places() } }
            }
            else -> Unit
        }
    }
}

/** Whether the section is a chip of the Library page, rather than a place or a page of its own. */
internal fun LibrarySection.onLibraryPage(): Boolean = when (this) {
    LibrarySection.FAVOURITES, LibrarySection.DOWNLOADS, LibrarySection.HISTORY, LibrarySection.SMART -> false
    else -> true
}

/**
 * The places the library keeps, not kinds of music: one grouped card of rows, ending in Shuffle
 * where the shown list has one ([shuffle], the account's own library only). [places] false leaves just Shuffle.
 */
@Composable
private fun Places(vm: LibraryHomeViewModel, favorites: Boolean, shuffle: (() -> Unit)?, places: Boolean = true) {
    val nav = LocalNav.current
    val rules = LocalRules.current
    val size by vm.downloadsSize.collectAsStateWithLifecycle()
    val shown = buildList {
        if (!places) return@buildList
        if (favorites) add(LibrarySection.FAVOURITES)
        for (s in listOf(LibrarySection.DOWNLOADS, LibrarySection.HISTORY)) if (s in rules.sections) add(s)
    }
    val shuffleRow = shuffle.takeIf { rules.account }
    if (shown.isEmpty() && shuffleRow == null) return
    Column(Modifier.padding(bottom = LIST_GAP)) {
        GroupCard {
            shown.forEachIndexed { i, s ->
                GroupRow(
                    say.librarySection(s), { nav.librarySection(s) }, icon = sectionIcon(s), chevron = true,
                    value = if (s == LibrarySection.DOWNLOADS) size else null, divider = shuffleRow != null || i < shown.lastIndex,
                )
            }
            if (shuffleRow != null) GroupRow(say.shuffle, shuffleRow, icon = Iconoir.Shuffle, divider = false)
        }
    }
}

/** The narrowest a card is on a wide screen, and the gap between cards: the grid of the Albums page and of a home row's full view. */
internal val GRID_MIN = 170.dp
internal val GRID_GAP = 12.dp

/** What each section is drawn with. */
@Composable
private fun sectionIcon(s: LibrarySection): androidx.compose.ui.graphics.vector.ImageVector = when (s) {
    LibrarySection.ALBUMS -> Iconoir.CompactDisc
    LibrarySection.FAVOURITES -> Kanji.Seal
    LibrarySection.ARTISTS -> Iconoir.User
    LibrarySection.SONGS -> Iconoir.MusicNote
    LibrarySection.PLAYLISTS -> Iconoir.Playlist
    LibrarySection.SMART -> Iconoir.MagicWand
    LibrarySection.HISTORY -> Iconoir.ClockRotateRight
    LibrarySection.GENRES -> Iconoir.ViewGrid
    LibrarySection.DECADES -> Iconoir.Calendar
    LibrarySection.FOLDERS -> Iconoir.Folder
    LibrarySection.RADIO -> Iconoir.AntennaSignal
    LibrarySection.DOWNLOADS -> Iconoir.Download
}

/** One section of the library, a page of its own. Only it is composed, so only its view model loads anything. */
@Composable
fun LibrarySectionScreen(section: LibrarySection, actions: ActionsViewModel) {
    val home: LibraryHomeViewModel = viewModel()
    Column {
        LargeTitle(say.librarySection(section))
        when (section) {
            LibrarySection.ALBUMS -> Albums(viewModel(), { Places(home, favorites = false, shuffle = { actions.shuffleAlbums() }, places = false) })
            LibrarySection.ARTISTS -> Artists()
            LibrarySection.SONGS -> SongsScreen(actions, null, { Places(home, favorites = false, shuffle = { actions.shuffleAll() }, places = false) })
            LibrarySection.PLAYLISTS -> Playlists(actions)
            LibrarySection.SMART -> SmartList()
            LibrarySection.HISTORY -> HistoryList(actions)
            LibrarySection.FAVOURITES -> Favourites(actions)
            LibrarySection.GENRES -> Genres()
            LibrarySection.DECADES -> Decades()
            LibrarySection.FOLDERS -> Folders()
            LibrarySection.RADIO -> Radio()
            LibrarySection.DOWNLOADS -> Downloads(actions)
        }
    }
}

/** The albums, as a grid of prints or, sorted by Artist, a shelf per artist; the orders are pills under [header]. */
@Composable
private fun Albums(vm: AlbumsViewModel, places: @Composable () -> Unit = {}) {
    val albums by vm.albums.collectAsStateWithLifecycle()
    val sort by vm.sort.collectAsStateWithLifecycle()
    val none by vm.none.collectAsStateWithLifecycle()
    val favorites by vm.favorites.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    Column {
        if (none) {
            places()
            // The pills stay, so the filter that emptied the grid can be turned off.
            if (favorites) AlbumSortPills(vm)
            return@Column EmptyNote(if (favorites) Note.NO_FAVOURITE_ALBUMS else Note.NO_ALBUMS)
        }
        val header: @Composable () -> Unit = { places(); AlbumSortPills(vm) }
        if (sort == dev.nori.music.ffi.library.AlbumSort.ARTIST_ROWS) {
            ArtistRows(vm, header)
            return@Column
        }
        // Ask for the covers just past the fold while the ones on screen are still arriving. The first row
        // is read by a snapshot observer, not here: read in composition, it recomposed the whole grid on
        // every row scrolled.
        val list = rememberLazyGridState()
        val context = androidx.compose.ui.platform.LocalContext.current
        androidx.compose.runtime.LaunchedEffect(list, albums) {
            androidx.compose.runtime.snapshotFlow { list.firstVisibleItemIndex }.collect { first ->
                val from = minOf(first + 6, albums.size)
                prefetchCovers(context, albums.subList(from, minOf(from + 12, albums.size)).map { vm.cover(it.coverArt, CoverSize.CARD) })
            }
        }
        // The next page a screenful before the end, from a snapshot observer rather than from item composition.
        androidx.compose.runtime.LaunchedEffect(list, albums.size) {
            androidx.compose.runtime.snapshotFlow { (list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= albums.size - 12 }.collect { if (it) vm.loadMore() }
        }
        // Two columns, counted rather than measured: an adaptive grid gave a wide phone a third column,
        // and a cover a third of the way across the screen is too small to recognise a sleeve by, which
        // is the only reason to show covers instead of a list of names.
        // On its side the cards keep about the size they have upright (a little larger than Home's shelves,
        // which is the point of a grid of covers) and the row takes as many as fit.
        val wide = LocalWide.current
        LazyVerticalGrid(if (wide) GridCells.Adaptive(GRID_MIN) else GridCells.Fixed(2), Modifier.fadesUnderTitle(list), state = list, contentPadding = PaddingValues(start = Space.gutter, end = Space.gutter, bottom = Space.gutter + LocalChromeInset.current), horizontalArrangement = Arrangement.spacedBy(GRID_GAP), verticalArrangement = Arrangement.spacedBy(GRID_GAP)) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "header") { Column(Modifier.insideGutters()) { header() } }
            items(albums, key = { it.id }, contentType = { "album" }) { a ->
                AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 132.dp, { nav.album(a.id, a) }, Modifier.fillMaxWidth(), fill = true)
            }
        }
    }
}

/** A grid item that spans the page: run out over the gutters the grid keeps. */
private fun Modifier.insideGutters(): Modifier = layout { measurable, constraints ->
    val gutter = Space.gutter.roundToPx()
    val placeable = measurable.measure(constraints.copy(minWidth = constraints.maxWidth + gutter * 2, maxWidth = constraints.maxWidth + gutter * 2))
    layout(constraints.maxWidth, placeable.height) { placeable.place(-gutter, 0) }
}

/** The size of an album's print on an artist's shelf: a little under Home's, so the row reads as one that scrolls. */
private val ARTIST_COVER = 112.dp

/** The albums a shelf per artist: their name and how many, then their albums. */
@Composable
private fun ArtistRows(vm: AlbumsViewModel, header: @Composable () -> Unit) {
    val rows by vm.rows.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    val list = rememberLazyListState()
    LaunchedEffect(list, rows.size) { snapshotFlow { (list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= rows.size - 8 }.collect { if (it) vm.loadMore() } }
    LazyColumn(Modifier.fadesUnderTitle(list), state = list, contentPadding = PaddingValues(bottom = Space.gutter + LocalChromeInset.current)) {
        item(key = "header") { Column(Modifier.padding(bottom = LIST_GAP)) { header() } }
        items(rows, key = { it.first().id }, contentType = { "artist" }) { row ->
            val first = row.first()
            val artist = first.artistId
            val cardMenu = LocalCardMenu.current
            Column(Modifier.padding(bottom = 22.dp)) {
                Row(
                    Modifier.fillMaxWidth().then(
                        if (artist != null) Modifier.combinedClickable(
                            onClick = { nav.artist(artist) },
                            onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfArtist(Artist(artist, first.artist, first.coverArt, null, row.size.toUInt(), false, first.isExternal))) },
                        ) else Modifier,
                    )
                        .padding(horizontal = Space.gutter).padding(bottom = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        first.artist, Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.bodyLarge.copy(fontSize = 17.sp, fontWeight = FontWeight.SemiBold),
                        maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                    )
                    Text(
                        remember(row.size) { say.albums(row.size) }, Modifier.padding(start = 8.dp),
                        style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.5.sp), color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1,
                    )
                    Spacer(Modifier.weight(1f))
                    if (artist != null) Icon(
                        Iconoir.NavArrowRight, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(row, key = { it.id }, contentType = { "cover" }) { a ->
                        val cardMenu = LocalCardMenu.current
                        CoverCard(a.name, if (a.year > 0u) "${a.year}" else "", vm.cover(a.coverArt, CoverSize.CARD), ARTIST_COVER, { nav.album(a.id, a) }, remote = a.isExternal, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfAlbum(a)) })
                    }
                }
            }
        }
    }
}

@Composable
private fun Artists(header: @Composable () -> Unit = {}, vm: ArtistsViewModel = viewModel()) {
    val load by vm.artists.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    var filter by remember { mutableStateOf("") }
    LoadBox(load) { all ->
        // The names go over to nori-core's `TextIndex` once per list, each keystroke sends the filter.
        val index = remember(all) { dev.nori.music.ffi.library.TextIndex(all.map { listOf(it.name) }) }
        val artists = remember(index, filter) { index.view(filter).rows.map { all[it.toInt()] } }
        if (all.isEmpty()) return@LoadBox EmptyNote(Note.NO_ARTISTS)
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            item { SearchField(filter, { filter = it }, say.filterArtists, Modifier.padding(start = Space.gutter, end = Space.gutter, bottom = LIST_GAP)) }
            items(artists, key = { it.id }, contentType = { "artist" }) { a ->
                val cardMenu = LocalCardMenu.current
                Column {
                    Row(Modifier.fillMaxWidth().combinedClickable(onClick = { nav.artist(a.id, a) }, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfArtist(a)) }).padding(horizontal = Space.gutter, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(vm.cover(a.coverArt, CoverSize.ROW), 48.dp, radius = 24.dp)
                        Column(Modifier.padding(start = 12.dp)) {
                            Text(a.name, style = MaterialTheme.typography.bodyLarge)
                            a.albumCount?.let { n -> Text(remember(n) { say.albums(n.toInt()) }, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        }
                    }
                    Hairline(startIndent = Space.gutter + 60.dp)
                }
            }
        }
    }
}

/**
 * Every indexed song, sorted; with [decade] set, only that decade. Reads the offline index, or the server's
 * list in its own order where the profile keeps none (a jam guest's).
 */
@Composable
fun SongsScreen(actions: ActionsViewModel, decade: Int?, header: @Composable () -> Unit = {}, vm: SongsViewModel = viewModel(key = "songs-$decade")) {
    LaunchedEffect(decade) { vm.setYears(decade?.let { dev.nori.music.ffi.library.decadeYears(it.toUInt()) }?.let { it.from.toInt()..it.to.toInt() }) }
    val songs by vm.songs.collectAsStateWithLifecycle()
    val sort by vm.sort.collectAsStateWithLifecycle()
    val starred by vm.starredOnly.collectAsStateWithLifecycle()
    val rows = rememberSongListState(actions)
    val list = rememberLazyListState()
    // Ask for the next page a screenful before the end, from a snapshot observer rather than from inside item composition.
    LaunchedEffect(list, songs.size) { snapshotFlow { (list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= songs.size - 40 }.collect { if (it) vm.loadMore() } }
    val indexed = LocalRules.current.indexed
    LazyColumn(Modifier.fadesUnderTitle(list), state = list, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
        item(key = "header") { header() }
        if (decade != null) item(key = "decade") { SectionTitle(remember(decade) { say.decade(decade) }) }
        if (indexed) item(key = "sorts") {
            LazyRow(Modifier.bleedsToEdges().padding(bottom = LIST_GAP), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                item { Chip(say.starredFavourites, starred) { vm.setStarredOnly(!starred) } }
                items(SongSort.entries) { s -> Chip(say.songSort(s.name), sort == s) { vm.setSort(s) } }
            }
        }
        if (songs.isEmpty() && indexed) item(key = "none") { EmptyNote(Note.NO_INDEX) }
        songRows(songs, actions, rows, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, from = LIBRARY_SONGS)
    }
}

@Composable
private fun Decades(header: @Composable () -> Unit = {}, vm: DecadesViewModel = viewModel()) {
    val load by vm.decades.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { decades ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            if (decades.isEmpty()) item { EmptyNote(Note.NO_INDEX) }
            item {
                GroupCard {
                    decades.forEachIndexed { i, d ->
                        GroupRow(remember(d.start) { say.decade(d.start.toInt()) }, { nav.decade(d.start.toInt()) }, value = "${d.songCount}", chevron = true, divider = i < decades.lastIndex)
                    }
                }
            }
        }
    }
}

@Composable
private fun Folders(header: @Composable () -> Unit = {}, vm: FoldersViewModel = viewModel()) {
    val load by vm.roots.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { roots ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            item {
                GroupCard {
                    roots.forEachIndexed { i, f -> GroupRow(f.name, { nav.folder(f.id) }, icon = Iconoir.Folder, chevron = true, divider = i < roots.lastIndex) }
                }
            }
        }
    }
}

@Composable
private fun Playlists(actions: ActionsViewModel, header: @Composable () -> Unit = {}, vm: PlaylistsViewModel = viewModel()) {
    val load by vm.playlists.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    var creating by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val pickM3u = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) runCatching { context.contentResolver.openInputStream(uri)?.use { it.readBytes().decodeToString() } }.getOrNull()?.let { text ->
            actions.importM3u(dev.nori.music.ffi.library.m3uPlaylistName(uri.lastPathSegment), text)
        }
    }
    NoriDialog(creating, { creating = false }) {
        AlertCard(
            title = { Text(say.newPlaylist) },
            text = { FormField(name, { name = it }, label = { Text(say.name) }, singleLine = true) },
            confirmButton = { TextAction(say.create, { vm.create(name.trim()); creating = false }, enabled = name.isNotBlank()) },
        )
    }
    val smart = LibrarySection.SMART in LocalRules.current.sections
    LoadBox(load) { playlists ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            // Smart playlists are playlists: theirs is a row here, as Apple keeps them among the others.
            item {
                GroupCard(Modifier.padding(bottom = LIST_GAP)) {
                    if (smart) GroupRow(say.smartPlaylists, { nav.librarySection(LibrarySection.SMART) }, icon = Iconoir.MagicWand, chevron = true)
                    GroupRow(say.newPlaylist, { name = ""; creating = true }, icon = Iconoir.Plus)
                    GroupRow(say.importPlaylistFile, { pickM3u.launch(arrayOf("*/*")) }, icon = Iconoir.Download, divider = false)
                }
            }
            if (playlists.isEmpty()) item { EmptyNote(Note.NO_PLAYLISTS) }
            items(playlists, key = { it.id }) { p ->
                val cardMenu = LocalCardMenu.current
                val menu = { cardMenu(dev.nori.music.app.vm.CardItem.OfPlaylist(p)) }
                NavRow(
                    p.name, { nav.playlist(p.id, p) }, onLongClick = menu,
                    subtitle = remember(p.songCount, p.duration) { say.listCaption(p.songCount.toInt(), p.duration.toLong(), false) },
                    leading = { Cover(vm.cover(p.coverArt, CoverSize.ROW), 48.dp) },
                    action = { IconButton(menu, Modifier.size(40.dp)) { Icon(Iconoir.MoreHoriz, say.more, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) } },
                )
            }
        }
    }
}

@Composable
private fun Favourites(actions: ActionsViewModel, vm: StarredViewModel = viewModel()) {
    val load by vm.starred.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    val list = rememberSongListState(actions)
    LoadBox(load) { s ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.pageToEdges().fadesUnderTitle(scroll), scroll, contentPadding = pagePadding(LocalChromeInset.current)) {
            // The songs as one list to play, the page the "Favourites" tile on Home opens too.
            item(key = "all") {
                NavRow(
                    say.favouriteSongs, { nav.mix(dev.nori.music.app.vm.FAVOURITES_MIX) }, chevron = true,
                    subtitle = remember(s.librarySongs) { say.songs(s.librarySongs.toInt()) },
                    leading = { Icon(Kanji.Sealed, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurface) },
                )
            }
            if (s.albums.isNotEmpty()) item(key = "albums") {
                LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(s.albums, key = { it.id }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 120.dp, { nav.album(a.id, a) }) }
                }
            }
            items(s.artists, key = { "ar" + it.id }) { a ->
                val cardMenu = LocalCardMenu.current
                NavRow(
                    a.name, { nav.artist(a.id, a) }, chevron = true, onLongClick = { cardMenu(dev.nori.music.app.vm.CardItem.OfArtist(a)) },
                    leading = { Cover(vm.cover(a.coverArt, CoverSize.ROW), 44.dp, radius = 22.dp) },
                )
            }
            songRows(s.songs, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) })
        }
    }
}

@Composable
private fun Genres(header: @Composable () -> Unit = {}, vm: GenresViewModel = viewModel()) {
    val load by vm.genres.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { genres ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            items(genres, key = { it.name }) { g ->
                NavRow(g.name, { nav.genre(g.name) }, trailing = "${g.songCount}", chevron = true)
            }
        }
    }
}

@Composable
private fun Radio(header: @Composable () -> Unit = {}, vm: RadioViewModel = viewModel()) {
    val load by vm.stations.collectAsStateWithLifecycle()
    var adding by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var url by remember { mutableStateOf("") }
    var deleting by remember { mutableStateOf<dev.nori.music.ffi.model.RadioStation?>(null) }
    DeleteQuestion(deleting, { it.name }, { vm.delete(it.id) }, { deleting = null })
    NoriDialog(adding, { adding = false }) {
        AlertCard(
            title = { Text(say.newStation) },
            text = { Column { FormField(name, { name = it }, label = { Text(say.name) }, singleLine = true); Spacer(Modifier.height(10.dp)); FormField(url, { url = it }, label = { Text(say.streamUrl) }, singleLine = true) } },
            confirmButton = { TextAction(say.add, { vm.add(name.trim(), url.trim()); adding = false }, enabled = dev.nori.music.ffi.library.radioCanAdd(name, url)) },
        )
    }
    LoadBox(load) { stations ->
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { header() }
            item { ActionRow(say.newStation, Iconoir.Plus, { name = ""; url = ""; adding = true }) }
            if (stations.isEmpty()) item { EmptyNote(Note.NO_STATIONS) }
            items(stations, key = { it.id }) { s ->
                NavRow(
                    s.name, { vm.play(s) },
                    leading = { Icon(Iconoir.AntennaSignal, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurface) },
                    action = { MoreButton(listOf(say.delete to { deleting = s })) },
                )
            }
        }
    }
}

/** What a queue played from the library's list of songs carries. */
private val LIBRARY_SONGS = dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SONGS, "")

/** What a queue played from the downloaded songs carries. */
internal val DOWNLOADED_SONGS = dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.DOWNLOADS, "")

@Composable
private fun Downloads(actions: ActionsViewModel) {
    val done by actions.downloadedLibrary.collectAsStateWithLifecycle()
    val list = rememberSongListState(actions)
    val counts by actions.downloadQueueCounts.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    val scroll = rememberLazyListState()
    LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
        // The way to the queue, always there: what is on its way now, or where it went.
        item(key = "queue") {
            // Counted by the core as it splits the queue for the downloads screen: waiting is what is
            // downloading or queued, the failed are counted apart.
            val c = counts
            NavRow(
                say.downloadQueue, nav::downloads, chevron = true,
                subtitle = remember(c) { say.downloadQueue((c?.waiting ?: 0u).toInt(), (c?.failed ?: 0u).toInt()) },
                leading = { Icon(Iconoir.DataTransferDown, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurface) },
            )
        }
        // Until the songs are read there is nothing to say, not "nothing downloaded" for a frame.
        val library = done
        if (library != null && library.albums.isEmpty() && library.songs.isEmpty()) item { EmptyNote(Note.NOTHING_DOWNLOADED) }
        items(library?.albums.orEmpty().chunked(2), key = { it.first().id }, contentType = { "albums" }) { row ->
            Row(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                row.forEach { album ->
                    AlbumCard(album, actions.cover(album.coverArt, CoverSize.CARD), 132.dp, { nav.downloadedAlbum(album.id, album) }, Modifier.weight(1f), fill = true, menu = false)
                }
                if (row.size == 1) Spacer(Modifier.weight(1f))
            }
        }
        songRows(library?.songs.orEmpty(), actions, list, cover = { actions.cover(it.coverArt, CoverSize.ROW) }, from = DOWNLOADED_SONGS)
    }
}
