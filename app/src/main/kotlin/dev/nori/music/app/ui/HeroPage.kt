package dev.nori.music.app.ui

import dev.nori.music.look.CoverLook
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.width
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.ui.layout.layout
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.font.FontWeight
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.PlayerViewModel
import dev.nori.music.app.vm.SettingsViewModel
import androidx.compose.foundation.layout.windowInsetsPadding

/**
 * An album, artist or playlist page, built the way Apple Music builds one: the artwork runs edge to
 * edge under the status bar and dissolves into the page, the page wears the colour the artwork ends
 * on, and the title, the artist and the two big buttons sit centred underneath it.
 *
 * The dissolve is the point. The wash starts from the average colour of the cover's own bottom rows,
 * so there is no line where the picture stops - it simply runs out of picture. Everything is static:
 * one gradient, no blur, no animation, so an open page costs nothing between frames. Scrolling moves
 * the artwork in the draw phase only, so it never recomposes.
 */
@Composable
fun HeroPage(
    coverUrl: String?,
    title: String,
    /** The same cover at the sizes lists ask for: drawn at once while [coverUrl] loads. */
    coverStandIns: List<String?> = emptyList(),
    /** The artist line under the title, in ink; tapping it opens [onSubtitle]. */
    subtitle: String? = null,
    /** A quiet line under that, in sentence case: year, song count, length, quality. */
    caption: String = "",
    /** The title's size: Literata 27 for a record, larger for an artist. */
    titleSize: TextUnit = 27.sp,
    onSubtitle: (() -> Unit)? = null,
    onPlay: (() -> Unit)? = null,
    onShuffle: (() -> Unit)? = null,
    /**
     * Keep the Shuffle + Play row even while [onPlay] / [onShuffle] are still null (hinted album /
     * artist / playlist pages waiting on the server). Without this the row jumps from heart-only to
     * transport when the detail lands - a one-frame pop.
     */
    awaitingPlay: Boolean = false,
    /**
     * This page's own queue (`pages::PageQueue`): the origin a queue started from the page carries. The
     * two big buttons answer for that queue rather than for the player in general
     * (`pages::hero_buttons`), and only for it: another page's queue playing a song this page also has
     * leaves them Play and Shuffle. Null: the page has no queue of its own.
     */
    queue: dev.nori.music.ffi.library.PageQueue? = null,
    /** The seal, bare, after the Play pill. */
    actions: @Composable RowScope.() -> Unit = {},
    /** The list head's quiet label at the left ("8 songs · 36:28"); with [headActions] it heads the songs. */
    headLabel: String = "",
    /** Bare glyphs at the right of the list head: download, add to queue, or a mix's refresh (`PageActionGlyphs`). */
    headActions: (@Composable RowScope.() -> Unit)? = null,
    /**
     * Artwork of the page's own making, for a page with no cover to bleed (a mix): drawn as a tile in the
     * middle, below the status bar, the way Apple shows a made-for-you mix. Used only when [coverUrl] is null.
     */
    art: (@Composable () -> Unit)? = null,
    /**
     * A page made of several records (a mix): up to four covers standing in for the one cover, drawn as a
     * 2x2 mosaic where a cover would be - full width, dissolving into the page upright, the soft band on its
     * side - and the page's colours taken from the first. Used only when [coverUrl] is null.
     */
    mosaic: List<String> = emptyList(),
    content: LazyListScope.() -> Unit,
) {
    val settings: SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    // Pages wear their cover's colours except in Midnight (the core's rule).
    val tinted = LocalPapers.current.coversTint
    // The picture that stands at the top of the page: the cover, or the mosaic of a page of several records.
    val lead = coverUrl ?: mosaic.firstOrNull()
    val picture: (@Composable (Modifier) -> Unit)? = when {
        coverUrl != null -> { m -> Cover(coverUrl, 0.dp, m, radius = 0.dp, standIns = coverStandIns) }
        mosaic.isNotEmpty() -> { m -> Mosaic(mosaic, m) }
        else -> null
    }
    // Measured from the smaller cover the tile that opened the page holds in memory, when it does (a
    // provider's too, whose file the core keeps in memory): the full one is a download away, and the
    // page wore the theme's colours until it came.
    val context = androidx.compose.ui.platform.LocalContext.current
    val measured = remember(lead, coverStandIns) {
        coverStandIns.firstOrNull { it != null && dev.nori.music.data.CoverLoader.get(context).kept(it) != null } ?: lead
    }
    val palette = if (tinted) rememberCoverPalette(measured) else null
    // Shuffle stays labelled Shuffle (never Pause); it lights while this page's queue is shuffling.
    val player: PlayerViewModel = viewModel()
    val playerState by player.state.collectAsStateWithLifecycle()
    // The queue this page started is what is on - whichever song of it happens to be sounding. Asked
    // when a new queue is set (the core's origin generation), not on every change of the player's state.
    val here = remember(queue, playerState.origin) { queue != null && player.playsFrom(queue) }
    // The core's answer (`pages::hero_buttons`) over JNI, one int, on every play and pause.
    val bits = remember(here, playerState.shuffle, playerState.playing, playerState.buffering, onPlay != null, onShuffle != null) {
        CoverLook.heroButtons(here, playerState.shuffle, playerState.playing, playerState.buffering, onPlay != null, onShuffle != null)
    }
    val pausing = bits and 4 != 0
    val buttons = remember(bits) {
        dev.nori.music.ffi.library.HeroButtons(
            shuffleLit = bits and 1 != 0, shuffleEnabled = bits and 2 != 0, shufflePress = heroPress(bits shr 4),
            pausing = pausing, playEnabled = bits and 8 != 0, playPress = heroPress(bits shr 6),
        )
    }

    TintedTheme(palette) {
        val scheme = MaterialTheme.colorScheme
        SystemBarIcons(LocalLook.current)
        PageTint(palette, waiting = palette == null && tinted && lead != null)
        val list = rememberLazyListState()
        // How far the hero has scrolled off, for its parallax: the list's own scroll upright; on its side the
        // hero stands still in its half and does not move with the songs (see below).
        val wide = LocalWide.current
        val heroScroll: (Float) -> Float = if (wide) { _ -> 0f } else { h -> if (list.firstVisibleItemIndex == 0) list.firstVisibleItemScrollOffset.toFloat() else h }
        // One block: artwork, then the wash it melts into, carrying the title and the buttons.
        // [coverSide]: on its side, the cover (or the page's own artwork) at this size, centred in the half,
        // so the half keeps the width its buttons need however short the screen is. Upright it is null and
        // the cover takes the full width, as ever.
        // [showArt] false: the name, caption and buttons alone, for laying over a cover drawn apart (below).
        val hero: @Composable (coverSide: androidx.compose.ui.unit.Dp?, showArt: Boolean) -> Unit = { coverSide, showArt ->
        Column(Modifier.fillMaxWidth()) {
            if (showArt) {
            // On its side the cover is a card standing in its half, as on a shelf, not a sleeve bleeding to the
            // screen's edges: nothing for it to dissolve into above or beside it.
            if (picture != null && coverSide != null) Box(
                Modifier.fillMaxWidth().windowInsetsPadding(statusBarRoom).padding(top = WIDE_TOP, bottom = 14.dp),
                Alignment.Center,
            ) { picture(Modifier.size(coverSide).clip(androidx.compose.foundation.shape.RoundedCornerShape(Radius.card))) }
            else if (picture != null) Box(
                Modifier.fillMaxWidth().layout { measurable, constraints ->
                    val side = constraints.maxWidth
                    val placeable = measurable.measure(androidx.compose.ui.unit.Constraints.fixed(side, side + HERO_DEEPER.roundToPx()))
                    layout(placeable.width, placeable.height) { placeable.place(0, 0) }
                }
                    // Parallax and fade, read in the draw phase: scrolling never recomposes the hero.
                    .graphicsLayer {
                        val scrolled = heroScroll(size.height)
                        translationY = scrolled * 0.4f
                        alpha = 1f - (scrolled / size.height).coerceIn(0f, 1f) * 0.5f
                    },
            ) {
                picture(Modifier.fillMaxSize())
                val look = LocalLook.current
                Box(
                    Modifier.fillMaxSize().drawWithCache {
                        // The whole dissolve happens inside the artwork, and finishes on the
                        // page colour rather than on the cover's edge colour: a second gradient
                        // below would be squeezed into a few dozen pixels as the parallax slides
                        // the picture down over it, and a colour ramp that steep across the full
                        // width is a line. The stops are the look's (nori_look::dress), made
                        // into brushes once per size.
                        val at = stage.heroStops
                        val dissolve = Brush.verticalGradient(
                            at[0] to Color.Transparent,
                            at[1] to look.color(CoverLook.HERO_EDGE),
                            at[2] to look.color(CoverLook.HERO_MID),
                            at[3] to look.color(CoverLook.BACKGROUND),
                        )
                        // Just enough shade under the status bar for white icons on a pale cover.
                        val shade = Brush.verticalGradient(0f to Color.Black.copy(alpha = stage.statusShade), stage.statusShadeTo to Color.Transparent)
                        onDrawWithContent {
                            drawContent()
                            drawRect(dissolve)
                            drawRect(shade)
                        }
                    },
                )
            } else if (art != null) Box(
                Modifier.fillMaxWidth().windowInsetsPadding(statusBarRoom).padding(top = if (coverSide != null) WIDE_TOP else 64.dp, bottom = if (coverSide != null) 14.dp else 18.dp)
                    // The page's own artwork is drawn at one size; on its side it is scaled to fit its place.
                    .then(if (coverSide != null) Modifier.height(coverSide) else Modifier),
                Alignment.Center,
            ) {
                if (coverSide == null) art()
                else Box(Modifier.size(coverSide).wrapContentSize(unbounded = true).graphicsLayer {
                    val k = coverSide.toPx() / MIX_ART.toPx()
                    scaleX = k; scaleY = k
                }) { art() }
            } else Spacer(Modifier.windowInsetsPadding(statusBarRoom).height(72.dp))
            }

            // Nothing is painted here: the artwork above has already dissolved onto the page
            // colour, and the page colour is what the root is painted with.
            Column(Modifier.fillMaxWidth()) {
            // Upright over a picture the title block rises into the melt, overlapping the picture's last rows.
            val rise = showArt && coverSide == null && picture != null
            Column(
                Modifier.fillMaxWidth().padding(horizontal = Space.gutter).then(if (rise) Modifier.raised(Space.titleRise + HERO_DEEPER) else Modifier.padding(vertical = 4.dp)),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    title, style = titled(MaterialTheme.typography.headlineSmall.copy(fontSize = titleSize)), textAlign = TextAlign.Center,
                    maxLines = 2, overflow = TextOverflow.Ellipsis,
                )
                if (!subtitle.isNullOrEmpty()) Text(
                    subtitle,
                    Modifier.padding(top = 3.dp).then(if (onSubtitle != null) Modifier.clickable(onClick = onSubtitle) else Modifier),
                    style = MaterialTheme.typography.bodyLarge.copy(fontSize = 17.sp, fontWeight = FontWeight.Medium),
                    textAlign = TextAlign.Center, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                if (caption.isNotEmpty()) {
                    val look = LocalLook.current
                    LookText(
                        caption, { look.color(CoverLook.ON_VARIANT) }, Modifier.padding(top = 5.dp),
                        style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp), textAlign = TextAlign.Center,
                        maxLines = 2, overflow = TextOverflow.Ellipsis,
                    )
                }
            }

            // Shuffle · Play · the page's seal: bare glyphs either side of an ink pill that fills the row. The
            // row is reserved while [awaitingPlay] so the layout does not jump when the taps land.
            if (awaitingPlay || onPlay != null || onShuffle != null) Row(
                Modifier.fillMaxWidth().padding(start = Space.tight, end = Space.tight, top = 12.dp),
                Arrangement.spacedBy(4.dp), Alignment.CenterVertically,
            ) {
                val press: (dev.nori.music.ffi.library.HeroPress, (() -> Unit)?) -> Unit = { p, start ->
                    when (p) {
                        dev.nori.music.ffi.library.HeroPress.START -> start?.invoke()
                        dev.nori.music.ffi.library.HeroPress.TOGGLE -> player.toggle()
                        dev.nori.music.ffi.library.HeroPress.SHUFFLE_OFF -> player.toggleShuffle()
                    }
                }
                GlyphButton(
                    Icons.Filled.Shuffle, say.shuffle,
                    enabled = buttons.shuffleEnabled, lit = buttons.shuffleLit,
                    onClick = { press(buttons.shufflePress, onShuffle) },
                )
                PillButton(
                    if (buttons.pausing) say.pause else say.play, if (buttons.pausing) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                    { press(buttons.playPress, onPlay) }, Modifier.weight(1f),
                    prominent = true, enabled = buttons.playEnabled,
                )
                actions()
            } else Row(
                Modifier.fillMaxWidth().padding(start = Space.tight, end = Space.tight, top = 2.dp),
                Arrangement.Center, Alignment.CenterVertically,
            ) { actions() }
            if (headLabel.isNotEmpty() || headActions != null) ListHead(headLabel, headActions)
            Spacer(Modifier.height(10.dp))
            }
        }
        }
        // On its side the page's colour runs out under the camera's strip beside it, which it
        // leaves as it goes, rather than the app's own colour there showing the page's edges.
        val outStart = LocalPageStart.current
        val outEnd = LocalPageEnd.current
        Box(Modifier.fillMaxSize().drawBehind {
            val l = outStart.toPx()
            drawRect(scheme.background, topLeft = Offset(-l, 0f), size = Size(size.width + l + outEnd.toPx(), size.height))
        }) {
            if (wide) androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxSize()) {
                // On its side the page stands in two halves, as Apple's does on a wide screen: the cover,
                // the name and the buttons beside it, and the songs scrolling down the right. Upright, the cover alone
                // was the whole screen and the songs began a screen further down.
                // Where the words begin: the player's controls' place, so the cover's soft edge runs on under them.
                val textAt = minOf(maxOf(maxWidth * 0.45f, 360.dp), maxWidth * 0.5f)
                // The cover at the end (a car's driver on the left, LocalCoverAtEnd): the same halves the other way round.
                val atEnd = LocalCoverAtEnd.current
                Row(Modifier.fillMaxSize()) {
                    // A cover fills its half as the player's sleeve fills its own: from the screen's very left
                    // edge - under the camera's punch hole, which the app otherwise keeps pages clear of - and top,
                    // going soft at its right edge, blurred as the sleeve does, into the page. The name, caption and
                    // buttons stand on the right above the songs, as the player's controls stand beside its sleeve,
                    // so nothing is written over the picture. A page with no picture at all - an artist without a
                    // photo, or one whose details have not come (offline) - keeps the same layout with the plain plate a
                    // missing cover shows, rather than falling back to another layout that appeared only then.
                    val band: @Composable (Modifier) -> Unit = picture ?: { m -> Cover(null, 0.dp, m, radius = 0.dp) }
                    val cover: @Composable () -> Unit = {
                        // The strip it runs out over: the camera's on the left, or on the right with the cover at the end.
                        val cutout = if (atEnd) LocalPageEnd.current else LocalPageStart.current
                        // The cover reaches [UNDER_TEXT] in under the name, the buttons and the songs, which keep their
                        // place: wider than tall, as the player's sleeve on its side.
                        val under = UNDER_TEXT
                        Box(
                            // Takes up the room up to the words and draws on past it, under them, to [half]; and out
                            // over the strip the page is kept off (the camera's), to the screen's edge.
                            Modifier.width(textAt).fillMaxHeight().layout { measurable, constraints ->
                                val extra = cutout.roundToPx()
                                val w = constraints.maxWidth + extra + under.roundToPx()
                                val placeable = measurable.measure(constraints.copy(minWidth = w, maxWidth = w))
                                layout(constraints.maxWidth, placeable.height) { placeable.place(if (atEnd) -under.roundToPx() else -extra, 0) }
                            },
                        ) {
                            // SoftSleeve turns itself round with the cover at the end; the shade below goes with it.
                            SoftSleeve(Modifier.fillMaxSize()) { band(Modifier.fillMaxSize()) }
                            // The shade under the status bar, faded out with the soft right edge so it does not end on a line.
                            Box(
                                Modifier.fillMaxSize().graphicsLayer { compositingStrategy = androidx.compose.ui.graphics.CompositingStrategy.Offscreen; if (atEnd) scaleX = -1f }.drawWithCache {
                                    val shade = Brush.verticalGradient(0f to Color.Black.copy(alpha = stage.statusShade), stage.statusShadeTo to Color.Transparent)
                                    val right = alphaGradient(stage.rubOut, Color.Black, size.width * (1f - MELT * ACROSS_MELT), size.width, across = true)
                                    onDrawBehind {
                                        drawRect(shade)
                                        drawRect(right, blendMode = androidx.compose.ui.graphics.BlendMode.DstOut)
                                    }
                                },
                            )
                        }
                    }
                    // Beside a cover, not the camera's strip: a shelf here (an artist's albums) has nothing to run out
                    // under on this side, and its fade would lay the page's colour over the picture.
                    val words: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit = {
                        androidx.compose.runtime.CompositionLocalProvider(if (atEnd) LocalPageEnd provides 0.dp else LocalPageStart provides 0.dp) {
                        LazyColumn(Modifier.weight(1f).fillMaxHeight(), state = list) {
                            // The first song level with the top of the cover beside it.
                            item(key = "hero-wide-top") { Spacer(Modifier.windowInsetsPadding(statusBarRoom).height(WIDE_TOP)) }
                            item(key = "hero-wide-head", contentType = "hero") { Column(Modifier.padding(bottom = 8.dp)) { hero(null, false) } }
                            content()
                            item(key = "tail") { Spacer(Modifier.height(Space.section + LocalChromeInset.current)) }
                        }
                        }
                    }
                    if (atEnd) { words(); cover() } else { cover(); words() }
                }
            } else LazyColumn(state = list) {
                item(key = "hero", contentType = "hero") {
                    // One block: artwork, then the wash it melts into, carrying the title and the buttons.
                    //
                    hero(null, true)
                }
                content()
                item(key = "tail") { Spacer(Modifier.height(Space.section + LocalChromeInset.current)) }
            }
            // No back button over the artwork: the back gesture is how these pages are left, and a dimmed disc
            // on the cover only covered part of it.
        }
    }
}

/**
 * Several covers as one picture: two by two, each cell a cover cropped square, filling whatever box it is
 * given (a cover's place on a page). Fewer than four: the first alone. The seams are left as they are - the
 * page's dissolve and its soft edge run over all four at once, which is what makes them read as one picture.
 */
@Composable
private fun Mosaic(urls: List<String>, modifier: Modifier) {
    if (urls.size < 4) { Cover(urls.first(), 0.dp, modifier, radius = 0.dp); return }
    Column(modifier) {
        for (row in 0..1) Row(Modifier.weight(1f).fillMaxWidth()) {
            for (col in 0..1) Cover(urls[row * 2 + col], 0.dp, Modifier.weight(1f).fillMaxHeight(), radius = 0.dp)
        }
    }
}

/** The line over a page's songs: [label] quiet at the left, [actions] at the right, a hairline under. */
@Composable
private fun ListHead(label: String, actions: (@Composable RowScope.() -> Unit)?) {
    val look = LocalLook.current
    Column(Modifier.fillMaxWidth().padding(top = 6.dp)) {
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = Space.gutter, end = Space.tight), verticalAlignment = Alignment.CenterVertically) {
            LookText(
                label, { look.color(CoverLook.ON_VARIANT) }, Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp), maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            actions?.invoke(this)
        }
        Box(Modifier.padding(end = Space.gutter)) { Hairline() }
    }
}

/** Placed [by] higher than it is laid out, so what follows moves up with it and the two overlap. */
private fun Modifier.raised(by: androidx.compose.ui.unit.Dp): Modifier = layout { measurable, constraints ->
    val placeable = measurable.measure(constraints)
    val up = by.roundToPx()
    layout(placeable.width, placeable.height - up) { placeable.place(0, -up) }
}

/** On its side: the space above the cover and above the first song, under the status bar. */
private val WIDE_TOP = 12.dp

/** The size a page's own artwork (a mix's) is drawn at: MixScreen hands [HeroPage] its art at this size. */
private val MIX_ART = 236.dp

/** Two bits of `HeroButtons::pack`: what a button presses. */
private fun heroPress(bits: Int) = when (bits and 3) {
    1 -> dev.nori.music.ffi.library.HeroPress.TOGGLE
    2 -> dev.nori.music.ffi.library.HeroPress.SHUFFLE_OFF
    else -> dev.nori.music.ffi.library.HeroPress.START
}

/** How much taller than square the picture is, with the title block rising by as much: the cover reaches further behind the title. */
private val HERO_DEEPER = 44.dp
