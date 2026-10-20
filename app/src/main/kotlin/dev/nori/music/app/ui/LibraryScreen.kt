package dev.nori.music.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.size
import androidx.compose.ui.draw.clip
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material.icons.rounded.Shuffle
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
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Downloading
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
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
import dev.nori.music.ffi.settings.LibrarySection
import androidx.compose.material.icons.rounded.Album
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.CalendarMonth
import androidx.compose.material.icons.rounded.Category
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Radio
import androidx.compose.material.icons.automirrored.rounded.QueueMusic
import dev.nori.music.app.vm.LibraryHomeViewModel
import dev.nori.music.app.vm.SongSort
import dev.nori.music.app.vm.SongsViewModel
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.launch
import dev.nori.music.app.vm.GenresViewModel
import dev.nori.music.app.vm.PlaylistsViewModel
import dev.nori.music.app.vm.RadioViewModel
import dev.nori.music.app.vm.StarredViewModel

/**
 * One line that says how the list is ordered and opens the alternatives, instead of a second row of
 * chips under the first: two stacked chip strips made this screen read as a toolbar.
 */
@Composable
private fun <T> SortMenu(options: List<Pair<T, String>>, value: T, onChange: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box(Modifier.padding(start = Space.gutter - 4.dp, top = 2.dp)) {
        Row(
            Modifier.clip(PillShape).clickable { open = true }.padding(horizontal = 6.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Filled.SwapVert, null, Modifier.size(17.dp), tint = MaterialTheme.colorScheme.primary)
            Text(
                options.firstOrNull { it.first == value }?.second ?: say.sort,
                Modifier.padding(start = 4.dp), style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )
        }
        DropdownMenu(open, { open = false }) {
            options.forEach { (v, label) -> DropdownMenuItem({ Text(label) }, { onChange(v); open = false }) }
        }
    }
}

/** Plays the whole of a list in random order, worded and drawn like [SortMenu] across from it. */
@Composable
private fun ShuffleLink(onClick: () -> Unit) {
    Row(
        Modifier.padding(end = Space.gutter - 4.dp, top = 2.dp).clip(PillShape).clickable(onClick = onClick).padding(horizontal = 6.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Shuffle, null, Modifier.size(17.dp), tint = MaterialTheme.colorScheme.primary)
        Text(say.shuffle, Modifier.padding(start = 4.dp), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
    }
}

/**
 * The library as Apple's: a list of its sections, each a page of its own, and the albums added last under
 * it. Which sections, and their order, are the user's (Settings, Appearance, Library sections) among those
 * the profile offers (`library_sections_shown`); smart playlists are under Playlists, and History is the
 * page's own button. A list rather than a strip of pills: a dozen pills ran off the screen sideways.
 */
@Composable
fun LibraryScreen(actions: ActionsViewModel, vm: LibraryHomeViewModel = viewModel()) {
    val rules = LocalRules.current
    val settings: dev.nori.music.app.vm.SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val sections = remember(prefs.librarySections, rules.sections) { dev.nori.music.ffi.library.librarySectionsShown(prefs.librarySections, rules.sections) }
    val recent by vm.recent.collectAsStateWithLifecycle()
    val playlists by vm.playlists.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    androidx.compose.foundation.layout.BoxWithConstraints {
    // As many across as the Albums page has (its grid's own count, [albumColumns]): two upright.
    val across = albumColumns(LocalWide.current, maxWidth)
    LazyColumn(Modifier.underStatusBar(), contentPadding = PaddingValues(top = LocalStatusRoom.current, bottom = Space.gutter + LocalChromeInset.current)) {
        item(key = "title") {
            // What was played and what is kept on the phone: places rather than kinds of music, so buttons
            // in the corner rather than rows of the list.
            LargeTitle(say.library) {
                Row(Modifier.padding(end = Space.gutter - Space.tight), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    for (s in listOf(LibrarySection.DOWNLOADS, LibrarySection.HISTORY)) {
                        if (s in rules.sections) OutlineCircleButton(sectionIcon(s), say.librarySection(s)) { nav.librarySection(s) }
                    }
                }
            }
        }
        // A profile with no library (a provider's, signed out) is found through search.
        if (rules.sections.isEmpty()) item(key = "search") {
            EmptyNote(say.searchIsTheWayIn, icon = Icons.Rounded.Search)
        }
        items(sections, key = { it.name }) { s ->
            NavRow(
                say.librarySection(s), { nav.librarySection(s) }, chevron = true,
                leading = { Icon(sectionIcon(s), null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary) },
            )
        }
        // Without an offline index the albums are the account's saved ones, in the source's order.
        if (recent.isNotEmpty()) {
            item(key = "recent") { SectionTitle(if (rules.indexed) say.recentlyAdded else say.yourAlbums) }
            items(recent.chunked(across), key = { row -> row.first().id }) { row ->
                Row(Modifier.padding(horizontal = Space.gutter, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(ALBUM_GAP)) {
                    row.forEach { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 132.dp, { nav.album(a.id, a) }, Modifier.weight(1f), fill = true) }
                    repeat(across - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
        // A source's own playlists, where its library is not indexed and the page would otherwise end here.
        if (!rules.indexed && playlists.isNotEmpty()) item(key = "playlists") {
            Column {
                SectionTitle(say.yourPlaylists)
                LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(playlists, key = { it.id }) { p ->
                        val line = remember(p.songCount, p.comment) { if (p.songCount > 0u) say.songs(p.songCount.toInt()) else p.comment.orEmpty() }
                        CoverCard(p.name, line, vm.cover(p.coverArt, CoverSize.CARD), 132.dp, { nav.playlist(p.id, p) })
                    }
                }
            }
        }
    }
    }
}

/** The narrowest an album's card is on its side, and the gap between cards: the Albums page's grid and Library's recent albums. */
private val ALBUM_MIN = 170.dp
private val ALBUM_GAP = 12.dp

/**
 * How many album cards stand across [width]: two upright; on its side as many as fit at [ALBUM_MIN] or wider,
 * counted as an adaptive grid counts them, the page's gutters taken off.
 */
private fun albumColumns(wide: Boolean, width: androidx.compose.ui.unit.Dp): Int =
    if (!wide) 2 else (((width - Space.gutter * 2 + ALBUM_GAP) / (ALBUM_MIN + ALBUM_GAP)).toInt()).coerceAtLeast(1)

/** A round button drawn as its outline alone: the page's own places, quieter than the plates on a record's page. */
@Composable
private fun OutlineCircleButton(icon: androidx.compose.ui.graphics.vector.ImageVector, description: String, onClick: () -> Unit) =
    androidx.compose.material3.Surface(
        onClick = onClick, shape = androidx.compose.foundation.shape.CircleShape, color = androidx.compose.ui.graphics.Color.Transparent,
        border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant), modifier = Modifier.size(38.dp),
    ) { Box(Modifier.fillMaxSize(), Alignment.Center) { Icon(icon, description, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurface) } }

/** What each section is drawn with. */
@Composable
private fun sectionIcon(s: LibrarySection): androidx.compose.ui.graphics.vector.ImageVector = when (s) {
    LibrarySection.ALBUMS -> Icons.Rounded.Album
    LibrarySection.FAVOURITES -> Kanji.Seal
    LibrarySection.ARTISTS -> Icons.Rounded.Person
    LibrarySection.SONGS -> Icons.Rounded.MusicNote
    LibrarySection.PLAYLISTS -> Icons.AutoMirrored.Rounded.QueueMusic
    LibrarySection.SMART -> Icons.Rounded.AutoAwesome
    LibrarySection.HISTORY -> Icons.Rounded.History
    LibrarySection.GENRES -> Icons.Rounded.Category
    LibrarySection.DECADES -> Icons.Rounded.CalendarMonth
    LibrarySection.FOLDERS -> Icons.Rounded.Folder
    LibrarySection.RADIO -> Icons.Rounded.Radio
    LibrarySection.DOWNLOADS -> Icons.Rounded.Download
}

/** One section of the library, a page of its own. Only it is composed, so only its view model loads anything. */
@Composable
fun LibrarySectionScreen(section: LibrarySection, actions: ActionsViewModel) {
    Column {
        BackTitle(say.librarySection(section))
        when (section) {
            LibrarySection.ALBUMS -> Albums(actions)
            LibrarySection.ARTISTS -> Artists()
            LibrarySection.SONGS -> SongsScreen(actions, null)
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

@Composable
private fun Albums(actions: ActionsViewModel, vm: AlbumsViewModel = viewModel()) {
    val albums by vm.albums.collectAsStateWithLifecycle()
    val sort by vm.sort.collectAsStateWithLifecycle()
    val none by vm.none.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    Column {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            SortMenu(remember { dev.nori.music.ffi.library.albumSorts().map { it to say.albumSort(it) } }, sort, vm::setSort)
            Spacer(Modifier.weight(1f))
            // Random albums, each whole and in its own order; the account's own library only.
            if (LocalRules.current.account) ShuffleLink(actions::shuffleAlbums)
        }
        if (sort == dev.nori.music.ffi.library.AlbumSort.ARTIST_ROWS) {
            ArtistRows(vm)
            return@Column
        }
        // Ask for the covers just past the fold while the ones on screen are still arriving. The first row
        // is read by a snapshot observer, not here: read in composition, it recomposed the whole grid on
        // every row scrolled.
        val list = rememberLazyGridState()
        val context = androidx.compose.ui.platform.LocalContext.current
        androidx.compose.runtime.LaunchedEffect(list, albums) {
            androidx.compose.runtime.snapshotFlow { list.firstVisibleItemIndex }.collect { first ->
                prefetchCovers(context, albums.drop(first + 6).take(12).map { vm.cover(it.coverArt, CoverSize.CARD) })
            }
        }
        // Two columns, counted rather than measured: an adaptive grid gave a wide phone a third column,
        // and a cover a third of the way across the screen is too small to recognise a sleeve by, which
        // is the only reason to show covers instead of a list of names.
        // On its side the same two would each be half a wide screen; there the cards keep about the size
        // they have upright and the row takes as many as fit.
        // On its side the cards keep about the size they have upright (a little larger than Home's shelves,
        // which is the point of a grid of covers) and the row takes as many as fit.
        val wide = LocalWide.current
        if (none) EmptyNote(Note.NO_ALBUMS)
        LazyVerticalGrid(if (wide) GridCells.Adaptive(ALBUM_MIN) else GridCells.Fixed(2), state = list, contentPadding = PaddingValues(start = Space.gutter, end = Space.gutter, top = Space.gutter, bottom = Space.gutter + LocalChromeInset.current), horizontalArrangement = Arrangement.spacedBy(ALBUM_GAP), verticalArrangement = Arrangement.spacedBy(ALBUM_GAP)) {
            itemsIndexed(albums, key = { _, a -> a.id }, contentType = { _, _ -> "album" }) { i, a ->
                if (i >= albums.size - 12) vm.loadMore()
                AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 132.dp, { nav.album(a.id, a) }, Modifier.fillMaxWidth(), fill = true)
            }
        }
    }
}

/** Thumbnails in an artist's row: about five across a phone, so a sixth shows the row scrolls. */
private val ROW_COVER = 64.dp

/** The albums a row per artist: the artist's name, then their covers alone. */
@Composable
private fun ArtistRows(vm: AlbumsViewModel) {
    val rows by vm.rows.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LazyColumn(contentPadding = PaddingValues(top = 4.dp, bottom = Space.gutter + LocalChromeInset.current)) {
        itemsIndexed(rows, key = { _, r -> r.first().id }, contentType = { _, _ -> "artist" }) { i, row ->
            if (i >= rows.size - 8) vm.loadMore()
            val first = row.first()
            val artist = first.artistId
            Column(Modifier.padding(bottom = 12.dp)) {
                Text(
                    first.artist,
                    Modifier.fillMaxWidth().then(if (artist != null) Modifier.clickable { nav.artist(artist) } else Modifier).padding(horizontal = Space.gutter, vertical = 6.dp),
                    style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
                LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(row, key = { it.id }, contentType = { "cover" }) { a ->
                        Cover(
                            vm.cover(a.coverArt, CoverSize.ROW), ROW_COVER,
                            Modifier.clickable { nav.album(a.id, a) }.semantics { contentDescription = a.name },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun Artists(vm: ArtistsViewModel = viewModel()) {
    val load by vm.artists.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    var filter by remember { mutableStateOf("") }
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    LoadBox(load) { all ->
        // What the filter keeps and the first row of each initial, for the index on the right edge
        // (nori-core's `TextIndex`): the names go over once per list, each keystroke sends the filter.
        val index = remember(all) { dev.nori.music.ffi.library.TextIndex(all.map { listOf(it.name) }) }
        val view = remember(index, filter) { index.view(filter) }
        val artists = remember(view) { view.rows.map { all[it.toInt()] } }
        val letters = view.letters
        if (all.isEmpty()) return@LoadBox EmptyNote(Note.NO_ARTISTS)
        Column {
            SearchField(filter, { filter = it }, say.filterArtists, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp))
            Row {
                LazyColumn(Modifier.weight(1f), state = list, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
                    items(artists, key = { it.id }, contentType = { "artist" }) { a ->
                        Column {
                            Row(Modifier.fillMaxWidth().clickable { nav.artist(a.id, a) }.padding(horizontal = Space.gutter, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
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
                if (view.showLetters) Column(Modifier.padding(end = 2.dp).verticalScroll(rememberScrollState())) {
                    letters.forEach { l -> Text(l.letter, Modifier.clickable { scope.launch { list.scrollToItem(l.row.toInt()) } }.padding(horizontal = 8.dp, vertical = 1.dp), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary) }
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
fun SongsScreen(actions: ActionsViewModel, decade: Int?, vm: SongsViewModel = viewModel(key = "songs-$decade")) {
    LaunchedEffect(decade) { vm.setYears(decade?.let { dev.nori.music.ffi.library.decadeYears(it.toUInt()) }?.let { it.from.toInt()..it.to.toInt() }) }
    val songs by vm.songs.collectAsStateWithLifecycle()
    val sort by vm.sort.collectAsStateWithLifecycle()
    val starred by vm.starredOnly.collectAsStateWithLifecycle()
    val rows = rememberSongListState(actions)
    val list = rememberLazyListState()
    // Ask for the next page a screenful before the end, from a snapshot observer rather than from inside item composition.
    LaunchedEffect(list, songs.size) { snapshotFlow { (list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= songs.size - 40 }.collect { if (it) vm.loadMore() } }
    Column {
        if (decade != null) SectionTitle(remember(decade) { say.decade(decade) })
        // The whole library's songs in random order: above the list, where Apple puts its Shuffle.
        else if (LocalRules.current.account) Row(Modifier.fillMaxWidth()) { Spacer(Modifier.weight(1f)); ShuffleLink(actions::shuffleAll) }
        val indexed = LocalRules.current.indexed
        if (indexed) LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            item { Chip(say.starredFavourites, starred) { vm.setStarredOnly(!starred) } }
            items(SongSort.entries) { s -> Chip(say.songSort(s.name), sort == s) { vm.setSort(s) } }
        }
        if (songs.isEmpty() && indexed) EmptyNote(Note.NO_INDEX)
        LazyColumn(state = list, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) { songRows(songs, actions, rows, cover = { vm.cover(it.coverArt, CoverSize.ROW) }, from = LIBRARY_SONGS) }
    }
}

@Composable
private fun Decades(vm: DecadesViewModel = viewModel()) {
    val load by vm.decades.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { decades ->
        LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            if (decades.isEmpty()) item { EmptyNote(Note.NO_INDEX) }
            items(decades, key = { it.start.toInt() }) { d ->
                NavRow(remember(d.start) { say.decade(d.start.toInt()) }, { nav.decade(d.start.toInt()) }, trailing = "${d.songCount}", chevron = true)
            }
        }
    }
}

@Composable
private fun Folders(vm: FoldersViewModel = viewModel()) {
    val load by vm.roots.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { roots ->
        LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            items(roots, key = { it.id }) { f ->
                NavRow(
                    f.name, { nav.folder(f.id) }, chevron = true,
                    leading = { Icon(Icons.Filled.Folder, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary) },
                )
            }
        }
    }
}

@Composable
private fun Playlists(actions: ActionsViewModel, vm: PlaylistsViewModel = viewModel()) {
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
        LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            // Smart playlists are playlists: theirs is a row here, as Apple keeps them among the others.
            if (smart) item {
                NavRow(say.smartPlaylists, { nav.librarySection(LibrarySection.SMART) }, chevron = true, leading = { Icon(Icons.Rounded.AutoAwesome, null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary) })
            }
            item { ActionRow(say.newPlaylist, Icons.Filled.Add, { name = ""; creating = true }) }
            item { ActionRow(say.importPlaylistFile, Icons.Filled.FileDownload, { pickM3u.launch(arrayOf("*/*")) }) }
            if (playlists.isEmpty()) item { EmptyNote(Note.NO_PLAYLISTS) }
            items(playlists, key = { it.id }) { p ->
                NavRow(
                    p.name, { nav.playlist(p.id, p) },
                    subtitle = remember(p.songCount, p.duration) { say.listCaption(p.songCount.toInt(), p.duration.toLong(), false) },
                    leading = { Cover(vm.cover(p.coverArt, CoverSize.ROW), 48.dp) },
                    action = { IconButton({ vm.delete(p.id) }, Modifier.size(40.dp)) { Icon(Icons.Filled.Delete, say.delete, Modifier.size(19.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) } },
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
        LazyColumn(Modifier.pageToEdges(), contentPadding = pagePadding(LocalChromeInset.current)) {
            // The songs as one list to play, the page the "Favourites" tile on Home opens too.
            item(key = "all") {
                NavRow(
                    say.favouriteSongs, { nav.mix(dev.nori.music.app.vm.FAVOURITES_MIX) }, chevron = true,
                    subtitle = remember(s.librarySongs) { say.songs(s.librarySongs.toInt()) },
                    leading = { Icon(Kanji.Sealed, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary) },
                )
            }
            if (s.albums.isNotEmpty()) item(key = "albums") {
                LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(s.albums, key = { it.id }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 120.dp, { nav.album(a.id, a) }) }
                }
            }
            items(s.artists, key = { "ar" + it.id }) { a ->
                NavRow(
                    a.name, { nav.artist(a.id, a) }, chevron = true,
                    leading = { Cover(vm.cover(a.coverArt, CoverSize.ROW), 44.dp, radius = 22.dp) },
                )
            }
            songRows(s.songs, actions, list, cover = { vm.cover(it.coverArt, CoverSize.ROW) })
        }
    }
}

@Composable
private fun Genres(vm: GenresViewModel = viewModel()) {
    val load by vm.genres.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LoadBox(load) { genres ->
        LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            items(genres, key = { it.name }) { g ->
                NavRow(g.name, { nav.genre(g.name) }, trailing = "${g.songCount}", chevron = true)
            }
        }
    }
}

@Composable
private fun Radio(vm: RadioViewModel = viewModel()) {
    val load by vm.stations.collectAsStateWithLifecycle()
    var adding by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var url by remember { mutableStateOf("") }
    NoriDialog(adding, { adding = false }) {
        AlertCard(
            title = { Text(say.newStation) },
            text = { Column { FormField(name, { name = it }, label = { Text(say.name) }, singleLine = true); Spacer(Modifier.height(10.dp)); FormField(url, { url = it }, label = { Text(say.streamUrl) }, singleLine = true) } },
            confirmButton = { TextAction(say.add, { vm.add(name.trim(), url.trim()); adding = false }, enabled = dev.nori.music.ffi.library.radioCanAdd(name, url)) },
        )
    }
    LoadBox(load) { stations ->
        LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            item { ActionRow(say.newStation, Icons.Filled.Add, { name = ""; url = ""; adding = true }) }
            if (stations.isEmpty()) item { EmptyNote(Note.NO_STATIONS) }
            items(stations, key = { it.id }) { s ->
                NavRow(
                    s.name, { vm.play(s) },
                    leading = { Icon(Icons.Filled.Radio, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary) },
                    action = { IconButton({ vm.delete(s.id) }, Modifier.size(40.dp)) { Icon(Icons.Filled.Delete, say.delete, Modifier.size(19.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) } },
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
    val d by actions.downloads.collectAsState()
    val done by actions.downloadedLibrary.collectAsStateWithLifecycle()
    val list = rememberSongListState(actions)
    val counts by actions.downloadQueueCounts.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
        // The way to the queue, always there: what is on its way now, or where it went.
        item(key = "queue") {
            // Counted by the core as it splits the queue for the downloads screen: waiting is what is
            // downloading or queued, the failed are counted apart.
            val c = counts
            NavRow(
                say.downloadQueue, nav::downloads, chevron = true,
                subtitle = remember(c) { say.downloadQueue((c?.waiting ?: 0u).toInt(), (c?.failed ?: 0u).toInt()) },
                leading = { Icon(Icons.Filled.Downloading, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary) },
            )
        }
        // Until the songs are read there is nothing to say, not "nothing downloaded" for a frame.
        val library = done
        if (library != null && library.albums.isEmpty() && library.songs.isEmpty()) item { EmptyNote(Note.NOTHING_DOWNLOADED) }
        items(library?.albums.orEmpty().chunked(2), key = { it.first().id }, contentType = { "albums" }) { row ->
            Row(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                row.forEach { album ->
                    AlbumCard(album, actions.cover(album.coverArt, CoverSize.CARD), 132.dp, { nav.downloadedAlbum(album.id, album) }, Modifier.weight(1f), fill = true)
                }
                if (row.size == 1) Spacer(Modifier.weight(1f))
            }
        }
        songRows(library?.songs.orEmpty(), actions, list, cover = { actions.cover(it.coverArt, CoverSize.ROW) }, from = DOWNLOADED_SONGS)
    }
}
