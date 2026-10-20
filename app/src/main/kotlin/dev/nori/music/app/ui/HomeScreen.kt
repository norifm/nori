package dev.nori.music.app.ui

import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.material.icons.filled.DragHandle
import androidx.compose.ui.zIndex
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshState
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material3.Text
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.material3.IconButton
import androidx.compose.material3.Icon
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.HomeViewModel
import dev.nori.music.ffi.settings.HomeRow
import dev.nori.music.ffi.model.Album

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(actions: ActionsViewModel, vm: HomeViewModel = viewModel()) {
    val load by vm.ui.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    // "For you" is the profile's where it has one (`ProfileRules.mixes`): a jam guest's Home is the host's shelves.
    val mixes = LocalRules.current.mixes
    val noLibrary = LocalRules.current.sections.isEmpty()
    val closed by vm.closedToGuests.collectAsStateWithLifecycle()
    val offer by vm.offer.collectAsStateWithLifecycle()
    // The last offer shown, drawn while its card leaves after the offer itself is gone.
    val shownOffer = remember { arrayOfNulls<dev.nori.music.ffi.library.QueueOffer>(1) }
    // Each time the page comes into sight: back from another app, or the phone unlocked.
    androidx.lifecycle.compose.LifecycleStartEffect(Unit) { vm.lookForOffer(); onStopOrDispose {} }
    LoadBox(load) { ui ->
        val arrival = rememberArrival()
        val rise = with(LocalDensity.current) { Arrival.RISE.toPx() }
        val refreshing by vm.refreshing.collectAsStateWithLifecycle()
        val pull = rememberPullToRefreshState()
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = vm::refresh,
            state = pull,
            indicator = { RefreshMark(pull, refreshing) },
        ) {
        // The page follows the pull down, and stays down while the refresh runs, so the ring has a gap of its
        // own above the title rather than landing on whatever comes first.
        LazyColumn(Modifier.pageToEdges().underStatusBar().graphicsLayer { translationY = pullGap(pull).toPx() }, contentPadding = pagePadding(LocalChromeInset.current, top = LocalStatusRoom.current)) {
            // Only the title, as Apple's: shuffling the library lives in Library, and which rows this page has
            // in Settings, Appearance.
            item(key = "title") { LargeTitle(say.listenNow, Modifier.arriving(arrival, 0, rise)) }
            // The card stays in the list while it goes: taken or turned down, it fades and settles back as its
            // room folds away and the page below rises into it. Arriving, it is simply there. Taken, the queue
            // starts once the card has gone: starting playback holds the main thread for a few frames, which
            // would cut the card's leaving short.
            if (offer != null) shownOffer[0] = offer
            shownOffer[0]?.let { o ->
                item(key = "offer") {
                    val ms = if (reduceMotion()) 0 else 320
                    var taking by remember(o) { mutableStateOf(false) }
                    val shown = remember { androidx.compose.animation.core.MutableTransitionState(true) }
                    shown.targetState = offer != null && !taking
                    if (taking && shown.isIdle && !shown.currentState) LaunchedEffect(o) { vm.takeOffer() }
                    androidx.compose.animation.AnimatedVisibility(
                        shown,
                        enter = androidx.compose.animation.EnterTransition.None,
                        exit = androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(ms * 2 / 3)) +
                            androidx.compose.animation.scaleOut(androidx.compose.animation.core.tween(ms), targetScale = 0.94f) +
                            androidx.compose.animation.shrinkVertically(androidx.compose.animation.core.tween(ms, easing = androidx.compose.animation.core.FastOutSlowInEasing)),
                    ) {
                        // Flicked away to either side, it is turned down, as its cross does: the room it leaves folds away after.
                        androidx.compose.runtime.key(o) {
                            val swipe = androidx.compose.material3.rememberSwipeToDismissBoxState(
                                confirmValueChange = { it != androidx.compose.material3.SwipeToDismissBoxValue.Settled && run { vm.dismissOffer(); true } },
                            )
                            androidx.compose.material3.SwipeToDismissBox(swipe, backgroundContent = {}) {
                                QueueOfferCard(o, actions, { taking = true }, vm::dismissOffer)
                            }
                        }
                    }
                }
            }
            if (closed && ui.rows.all { it.isEmpty }) item(key = "closed") {
                Text(say.homeClosedToGuests, Modifier.padding(vertical = 12.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            // A profile with no library and nothing in its feed (a provider's, offline) is found through search.
            if (noLibrary && ui.feed.isEmpty()) item(key = "search") {
                Text(say.searchIsTheWayIn, Modifier.padding(horizontal = Space.gutter, vertical = 12.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            // Favourites are always here; the mixes join them when the taste model is on (MixesViewModel).
            if (mixes) item(key = "mixes") { Column(Modifier.arriving(arrival, 1, rise)) { SectionTitle(say.forYou); MixTiles() } }
            // The shelves carry on the count the sections above started, so each one arrives a moment
            // after the one over it; an empty shelf is not drawn and does not take a place in the order.
            var place = 2
            // A source's own shelves (`ProfileRules.feed`), titled as it titles them; each kind drawn as the
            // library's shelves of that kind are.
            ui.feed.forEachIndexed { i, f ->
                val key = "feed-$i"
                if (f.songs.isNotEmpty()) songShelf(f.title, f.songs, vm, actions, arrival, place++, rise, dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.SHELF, f.title), "$key-songs")
                if (f.albums.isNotEmpty()) shelf(f.title, f.albums, vm, arrival, place++, rise, "$key-albums")
                if (f.playlists.isNotEmpty()) playlistShelf(f.title, f.playlists, vm, arrival, place++, rise, "$key-playlists")
            }
            ui.rows.forEach { shelf ->
                // The favourite playlists are a selection of something the page fetches once for all of
                // it, so their shelf is filled from there - but it stands where the user put it in the
                // order.
                val s = if (shelf.row == dev.nori.music.ffi.settings.HomeRow.PINNED) dev.nori.music.app.vm.Shelf.Playlists(shelf.row, ui.pinned) else shelf
                if (s.isEmpty) return@forEach
                when (s) {
                    is dev.nori.music.app.vm.Shelf.Albums -> shelf(say.homeRow(s.row), s.albums, vm, arrival, place++, rise)
                    is dev.nori.music.app.vm.Shelf.Playlists -> playlistShelf(say.homeRow(s.row), s.playlists, vm, arrival, place++, rise)
                    is dev.nori.music.app.vm.Shelf.Songs -> songShelf(say.homeRow(s.row), s.songs, vm, actions, arrival, place++, rise, s.origin)
                }
            }
        }
        }
    }
}

/** How far the page has come down under a pull: the gap the ring sits in. */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
private fun pullGap(state: PullToRefreshState) = PULL_GAP * state.distanceFraction.coerceIn(0f, 1.3f)

private val PULL_GAP = 56.dp
private val RING = 18.dp

/**
 * What a pull at the top of the page looks like: a thin ring, no container and nothing behind it, in the
 * middle of the gap the page leaves as it comes down ([pullGap]). The Material indicator is a filled
 * circle on a raised plate, which on this page reads as a button that has landed in the wrong place.
 * While the finger is down the ring fills as far as the pull has come, which is a static value read in
 * the draw phase; only once the refresh is really running does it turn, and it is gone the moment that
 * is over. At rest it is drawn at zero opacity, so an idle page has nothing at its top and nothing
 * animating there either.
 */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun androidx.compose.foundation.layout.BoxScope.RefreshMark(state: PullToRefreshState, refreshing: Boolean) {
    val tint = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.5f)
    Box(
        Modifier.align(androidx.compose.ui.Alignment.TopCenter)
            .graphicsLayer {
                val d = state.distanceFraction
                alpha = if (refreshing) 1f else (d * 1.6f - 0.15f).coerceIn(0f, 1f)
                translationY = ((pullGap(state) - RING) / 2).coerceAtLeast(0.dp).toPx()
            },
    ) {
        if (refreshing) androidx.compose.material3.CircularProgressIndicator(Modifier.size(RING), color = tint, strokeWidth = 2.dp)
        else androidx.compose.material3.CircularProgressIndicator(
            progress = { state.distanceFraction.coerceIn(0f, 1f) },
            modifier = Modifier.size(RING), color = tint, strokeWidth = 2.dp,
            trackColor = androidx.compose.ui.graphics.Color.Transparent,
            gapSize = 0.dp,
        )
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.shelf(
    title: String,
    albums: List<Album>,
    vm: HomeViewModel,
    arrival: State<Float>,
    place: Int,
    rise: Float,
    key: String = title,
) {
    item(key = key, contentType = "shelf") {
        val nav = LocalNav.current
        Column(Modifier.arriving(arrival, place, rise)) {
            SectionTitle(title)
            LazyRow(Modifier.bleedsToEdges(), rememberShelfState(albums.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                items(albums, key = { it.id }, contentType = { "album" }) { a -> AlbumCard(a, vm.cover(a.coverArt, CoverSize.CARD), 150.dp, { nav.album(a.id, a) }) }
            }
        }
    }
}

/** Playlists laid out as the albums are, so a shelf of them reads as the same kind of thing. */
private fun androidx.compose.foundation.lazy.LazyListScope.playlistShelf(
    title: String,
    playlists: List<dev.nori.music.ffi.model.Playlist>,
    vm: HomeViewModel,
    arrival: State<Float>,
    place: Int,
    rise: Float,
    key: String = title,
) {
    item(key = key, contentType = "shelf") {
        val nav = LocalNav.current
        Column(Modifier.arriving(arrival, place, rise)) {
            SectionTitle(title)
            LazyRow(Modifier.bleedsToEdges(), rememberShelfState(playlists.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                items(playlists, key = { it.id }, contentType = { "playlist" }) { p ->
                    // A feed's playlist is not counted until opened: its own line (who is in it) instead.
                    val line = remember(p.songCount, p.comment) { if (p.songCount > 0u) say.songs(p.songCount.toInt()) else p.comment.orEmpty() }
                    CoverCard(p.name, line, vm.cover(p.coverArt, CoverSize.CARD), 150.dp, { nav.playlist(p.id, p) })
                }
            }
        }
    }
}

/**
 * A shelf's scroll state that stays at its start when cards come in ahead of its first (an album just
 * played joins Recently played): by key, the row would keep the old first card in view, one card in.
 * A shelf scrolled by hand keeps its place.
 */
@Composable
internal fun rememberShelfState(first: Any?): LazyListState {
    val state = rememberLazyListState()
    val shown = remember { arrayOf(first) }
    if (shown[0] != first) {
        if (state.firstVisibleItemIndex == 0 && state.firstVisibleItemScrollOffset == 0) state.requestScrollToItem(0)
        shown[0] = first
    }
    return state
}

/** A shelf of songs plays from where it is tapped, the rest of the shelf behind it, as a list would. */
private fun androidx.compose.foundation.lazy.LazyListScope.songShelf(
    title: String,
    songs: List<dev.nori.music.ffi.model.Song>,
    vm: HomeViewModel,
    actions: dev.nori.music.app.vm.ActionsViewModel,
    arrival: State<Float>,
    place: Int,
    rise: Float,
    /** The shelf: the songs played from it are its queue. */
    from: dev.nori.music.ffi.model.PageOrigin,
    key: String = title,
) {
    item(key = key, contentType = "shelf") {
        // A long press opens the song's menu, as it does on a row of a list.
        val menu = LocalSongMenu.current
        Column(Modifier.arriving(arrival, place, rise)) {
            SectionTitle(title)
            LazyRow(Modifier.bleedsToEdges(), rememberShelfState(songs.firstOrNull()?.id), contentPadding = edgePadding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                itemsIndexed(songs, key = { _, s -> s.id }, contentType = { _, _ -> "song" }) { i, s ->
                    CoverCard(s.title, s.artist, vm.cover(s.coverArt, CoverSize.CARD), 150.dp, { actions.play(songs, i, from) }, onLongClick = { menu(s) })
                }
            }
        }
    }
}

/**
 * The home page's sections do not appear all at once: each one fades up and rises the last few pixels
 * into place a moment after the one above it, so the page reads top to bottom the way the pages it is
 * part of now arrive (see PageMotion in App.kt). It is one run, started when the page's content first
 * exists and over inside four tenths of a second, and the fourth section is the last to wait - with a
 * dozen shelves the page would otherwise still be assembling itself after the user had begun to scroll.
 */
private object Arrival {
    /** How long the whole run lasts, and how far apart two sections start within it. */
    const val RUN = 380
    private const val STEP = 40
    /** Each section's own move, and the last place that still waits its turn. */
    private const val MOVE = 220
    private const val LAST = 4

    /** The same curve the pages use: quick to appear, unhurried about coming to rest. */
    private val Settle = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1f)

    val RISE = 14.dp

    /** Where one section is in its own move, given how far the single run driving all of them has got. */
    fun at(run: Float, place: Int): Float {
        val ms = run * RUN - minOf(place, LAST) * STEP
        return Settle.transform((ms / MOVE).coerceIn(0f, 1f))
    }
}

/**
 * The one animation behind the whole page. It runs once, the first time the page is on screen after
 * the app starts: a section scrolled away and back reads a run that finished long ago, and fresh data
 * for a page already showing does not start it again, so nothing here moves while the page is sitting
 * still.
 *
 * Once per process, not once per visit. Coming back to the page - a pop from an album, a tab tap -
 * composes it again from nothing; run again, the page would slide back into place (PageMotion) with
 * every section at nought, a black card whose sections appear only once the slide is over. The page
 * is what is coming back; it should look as it did.
 */
@Composable
private fun rememberArrival(): State<Float> {
    val plain = reduceMotion()
    val progress = remember { Animatable(if (arrived) 1f else 0f) }
    // Linear, because the curve belongs to each section's own move and not to the clock they share.
    LaunchedEffect(plain) {
        arrived = true
        if (plain) progress.snapTo(1f)
        else if (progress.value < 1f) progress.animateTo(1f, tween(Arrival.RUN, easing = LinearEasing))
    }
    return progress.asState()
}

/** Whether the home page has arrived once already since the app started; see rememberArrival. */
private var arrived = false

/** Read in the draw phase: the run redraws the sections on screen and recomposes none of them. */
private fun Modifier.arriving(arrival: State<Float>, place: Int, rise: Float) = graphicsLayer {
    val t = Arrival.at(arrival.value, place)
    alpha = t
    translationY = rise * (1f - t)
}


/**
 * The queue another device or app left on the server, offered at the top of Home the way Spotify offers
 * to carry on here: who left it, the song it stands on, play and dismiss.
 */
@Composable
private fun QueueOfferCard(offer: dev.nori.music.ffi.library.QueueOffer, actions: ActionsViewModel, onPlay: () -> Unit, onDismiss: () -> Unit) {
    val song = offer.songs.getOrNull(offer.index.toInt()) ?: return
    androidx.compose.material3.Surface(
        onClick = onPlay, shape = androidx.compose.foundation.shape.RoundedCornerShape(16.dp),
        // Halfway between the large title's letters and the next heading's: with 4 dp of room above it and only what
        // the next heading keeps above itself below, the letters stood 15 and 21 dp off it, so it is drawn 3 dp
        // lower than its place; the page under it does not move.
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth().padding(start = Space.gutter, end = Space.gutter, top = 4.dp).offset(y = 3.dp),
    ) {
        Row(Modifier.padding(start = 12.dp, top = 12.dp, bottom = 12.dp, end = 4.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            Cover(actions.cover(song.coverArt, CoverSize.ROW), 48.dp, radius = 8.dp)
            Column(Modifier.weight(1f).padding(start = 12.dp)) {
                Text(say.continueFrom(offer.by), style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                Text(
                    remember(song) { say.songLine(song.title, song.artist) }, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
            }
            IconButton(onPlay) { Icon(androidx.compose.material.icons.Icons.Rounded.PlayArrow, say.play, Modifier.size(28.dp)) }
            IconButton(onDismiss) { Icon(androidx.compose.material.icons.Icons.Rounded.Close, say.close, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
    }
}
