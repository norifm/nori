package dev.nori.music.app.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onPlaced
import androidx.compose.runtime.setValue
import androidx.compose.material3.IconButton
import androidx.compose.runtime.mutableStateOf
import androidx.compose.animation.togetherWith
import androidx.compose.animation.core.animateFloat
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.composed
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.ColorProducer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.LocalMinimumInteractiveComponentSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import kotlinx.coroutines.launch
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.em
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.role
import dev.nori.music.app.R
import dev.nori.music.look.CoverLook
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import dev.nori.music.data.of

/**
 * The look of the app in one file: radii, spacing, type and the handful of shapes every screen is
 * built from. Apple Music is the reference for the feel - artwork that bleeds into the page, round
 * corners everywhere, hairlines instead of boxes, floating chrome - but the widgets underneath stay
 * ordinary Material 3, because a phone that looks like iOS on purpose ends up looking wrong.
 *
 * Everything here is static: colours and brushes are values, not animations, so a page that sits on
 * screen costs nothing between frames.
 */
object Radius {
    val cover = 8.dp
    val card = 12.dp
    val tile = 18.dp
    val sheet = 26.dp
    val pill = 100.dp
}

object Space {
    /** The side margin every screen shares. Text, rows and section titles all start here. */
    val gutter = 20.dp
    val tight = 8.dp
    val row = 12.dp
    val section = 26.dp
    /** How far the title block of the player, album and artist pages rises into the sleeve's melt. */
    val titleRise = 34.dp
}

val CardShape = RoundedCornerShape(Radius.card)
val TileShape = RoundedCornerShape(Radius.tile)
val PillShape = RoundedCornerShape(Radius.pill)

/** How thick the edge of a raised surface is: a chip, a card, a sheet, a popover, a dialog. */
val HAIRLINE = 0.5.dp

/** The edge of a raised surface, in the page's hairline color. */
@Composable
fun hairlineBorder(): androidx.compose.foundation.BorderStroke {
    val color = MaterialTheme.colorScheme.outlineVariant
    return remember(color) { androidx.compose.foundation.BorderStroke(HAIRLINE, color) }
}

/**
 * Inter, as a variable font, standing in for the system face an iPhone would use. It is the single
 * biggest reason a screenshot reads as "a modern music app" rather than "an Android app": Roboto's
 * wide, loose letterforms are what makes stock Compose look like a settings screen. One 880 kB file
 * carries every weight, and each weight is a real instance of the variable font, never a synthetic
 * bold - synthesised weights are what make text look smeared.
 */
private fun inter(weight: FontWeight) = Font(
    R.font.inter, weight,
    variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)),
)

val Inter = FontFamily(inter(FontWeight.Normal), inter(FontWeight.Medium), inter(FontWeight.SemiBold), inter(FontWeight.Bold))

/**
 * The second and last typeface, for what a page is called and nothing else: Literata at its display size,
 * a book face whose sharp serifs sit with the mincho of the seal and the seasons. Cut to that one optical
 * size and to weights 500-800 (440 kB), with the whole of Latin, Greek, Cyrillic and Vietnamese.
 */
private fun literata(weight: FontWeight) = Font(
    R.font.literata, weight,
    variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)),
)

private val Literata = FontFamily(literata(FontWeight.SemiBold), literata(FontWeight.Bold))

/** [style] as a page's name: in Literata, tracked a little tighter. */
fun titled(style: TextStyle): TextStyle = style.copy(fontFamily = Literata, letterSpacing = (-0.3).sp)

/**
 * Tighter and heavier than stock Material, which is what makes a music app read as a music app:
 * screen and album titles are display-weight with negative tracking, list rows are plain text, and
 * captions are small grey capitals.
 */
val NoriTypography = Typography().run {
    fun TextStyle.f(weight: FontWeight? = null, tracking: Float? = null) = copy(
        fontFamily = Inter,
        fontWeight = weight ?: fontWeight,
        letterSpacing = tracking?.sp ?: letterSpacing,
    )
    copy(
        displayLarge = displayLarge.f(FontWeight.Bold, -1.5f),
        displayMedium = displayMedium.f(FontWeight.Bold, -1.2f),
        displaySmall = displaySmall.f(FontWeight.Bold, -1f),
        headlineLarge = headlineLarge.f(FontWeight.Bold, -0.9f),
        headlineMedium = headlineMedium.f(FontWeight.Bold, -0.7f),
        headlineSmall = headlineSmall.f(FontWeight.Bold, -0.5f),
        titleLarge = titleLarge.f(FontWeight.Bold, -0.4f),
        titleMedium = titleMedium.f(FontWeight.SemiBold, -0.2f),
        titleSmall = titleSmall.f(FontWeight.SemiBold, -0.1f),
        bodyLarge = bodyLarge.f(FontWeight.Normal, -0.1f).copy(fontSize = 16.sp, lineHeight = 21.sp),
        bodyMedium = bodyMedium.f().copy(fontSize = 15.sp, lineHeight = 20.sp),
        bodySmall = bodySmall.f().copy(fontSize = 13.sp, lineHeight = 17.sp),
        labelLarge = labelLarge.f(FontWeight.SemiBold),
        labelMedium = labelMedium.f(FontWeight.Medium),
        labelSmall = labelSmall.f(FontWeight.SemiBold, 0.5f),
    )
}

/**
 * The colours a page wears, taken from its artwork: its whole [look] as nori-look dressed it (see
 * [Look]), and the wash - the cover blurred and pulled towards the page, drawn stretched over the page so
 * the colours vary the way the artwork's do instead of settling into one average.
 *
 * Two palettes are the same when their looks are and they share a wash.
 */
@androidx.compose.runtime.Immutable
class PagePalette(val look: IntArray, val wash: androidx.compose.ui.graphics.ImageBitmap? = null) {
    val fixed = FixedLook(look)
    /** What the bottom of the cover is, so the picture can dissolve into the page without a seam. */
    val edge: Color get() = Color(look[CoverLook.EDGE])
    /** The page under it. */
    val background: Color get() = Color(look[CoverLook.BACKGROUND])
    /** This page on the season's [papers] dyed with its cover (nori_look::dress::dyed), wash included. Not for the main thread. */
    fun dyed(papers: Papers): PagePalette =
        CoverLook.dyed(look, papers.table, papers.darkTheme, wash?.asAndroidBitmap())
            .let { PagePalette(it.look, it.wash?.asImageBitmap()) }
    override fun equals(other: Any?): Boolean = other is PagePalette && other.wash == wash && other.look.contentEquals(look)
    override fun hashCode(): Int = look.contentHashCode() * 31 + (wash?.hashCode() ?: 0)
}

val LocalPalette = staticCompositionLocalOf<PagePalette?> { null }

/** Star changes made this session, per kind (see Library.starMarks). */
val LocalStarMarks = staticCompositionLocalOf { dev.nori.music.ffi.library.StarMarks(emptyMap(), emptyMap(), emptyMap()) }

/**
 * Star state as the screen should show it: this session's change wins over the snapshot the list was
 * painted with. Every toggle must also act on this, not on the snapshot, or the second tap undoes
 * the first one's server call instead of flipping what is on screen.
 */
fun dev.nori.music.ffi.library.StarMarks.effectiveStar(kind: dev.nori.music.data.StarKind, id: String, snapshot: Boolean): Boolean =
    of(kind, id) ?: snapshot

/**
 * The player's page, aligned to its sleeve. The cover is drawn at the sleeve's own scale behind it, so
 * the blurred copy and the sharp one are the same picture at the same size; above and below, the
 * texture's first and last rows carry on. That last part is the whole point: the sleeve's bottom edge
 * then meets a wash made of the sleeve's own bottom rows, in the same colours, and the picture runs out
 * into the page instead of ending on one. Stretched over the screen instead, the wash showed the middle
 * of the cover where the sleeve ended, and the hue jumped across a line.
 */
fun androidx.compose.ui.draw.CacheDrawScope.sleeveWash(
    palette: PagePalette, sleeveBottom: Float, sleeveHeight: Float,
): androidx.compose.ui.draw.DrawResult {
    val endY = size.height
    val wash = palette.wash
    val page = palette.background
    if (wash == null) return onDrawBehind { drawRect(page, size = Size(size.width, endY)) }
    val w = size.width.toInt().coerceAtLeast(1)
    val bottom = sleeveBottom.coerceIn(1f, endY)
    val top = (bottom - sleeveHeight).coerceAtLeast(0f).toInt()
    // The last row, carried down, is the right colour where it meets the sleeve and wrong everywhere
    // below it: the same stripes at the same strength all the way to the bottom edge. Apple's page
    // darkens and calms as it goes down - at y 2000 of `w4` it is still their red, but deeper and more
    // even than under the artwork. So the stripes give way, slowly at first, to a deeper page colour,
    // and arrive at it exactly at the bottom edge of the screen, where there is nothing to meet.
    //
    // The page ends on its own colour, not on a darkened version of it: a bright record gets a bright
    // page, and darkening its bottom would split the screen in two. The gradient below calms the
    // stripes without changing how light the page is. Made once per page and sleeve, not per frame.
    val look = palette.look
    val floor = if (endY - bottom > 1f) Brush.verticalGradient(
        // The colours stay through the controls and settle into one only towards the bottom:
        // the owner liked them under the transport and did not want them gone, just ended.
        stage.floorStops[0] to Color(look[CoverLook.FLOOR_0]),
        stage.floorStops[1] to Color(look[CoverLook.FLOOR_22]),
        stage.floorStops[2] to Color(look[CoverLook.FLOOR_75]),
        stage.floorStops[3] to page,
        startY = bottom, endY = endY,
    ) else null
    val floorTop = Offset(0f, bottom)
    val floorSize = Size(size.width, endY - bottom)
    return onDrawBehind {
        // Rounded edges rather than rounded heights, so the three bands abut exactly with no row of page
        // colour showing between them.
        //
        // And the two plain bands each run one row on under the sleeve's, which is drawn last. Abutting is
        // exact only on whole pixels: while the sheet rises it sits at a fraction of a pixel, each band's
        // edge is anti-aliased, and two part-covered edges in the same row let the page under the sheet
        // show through - a hairline across the screen at the sleeve's bottom that came and went with the
        // fraction, half way up. The extra rows are one source row stretched, so they cost the picture
        // nothing, and on whole pixels the sleeve's band covers them.
        fun band(srcY: Int, srcH: Int, y0: Int, y1: Int) {
            if (y1 <= y0) return
            drawImage(
                wash,
                srcOffset = IntOffset(0, srcY), srcSize = IntSize(WASH_ROWS, srcH),
                dstOffset = IntOffset(0, y0), dstSize = IntSize(w, y1 - y0),
                filterQuality = FilterQuality.Low,
            )
        }
        val bottomRow = bottom.toInt()
        band(0, 1, 0, if (top > 0) top + 1 else 0)
        band(WASH_ROWS - 1, 1, if (bottomRow > top) bottomRow - 1 else bottomRow, endY.toInt())
        band(0, WASH_ROWS, top, bottomRow)
        if (floor != null) drawRect(floor, topLeft = floorTop, size = floorSize)
    }
}

/**
 * [sleeveWash] on its side: the sleeve is the left part of the screen, up to x [sleeveRight], [sleeveWidth]
 * wide and the screen's height. The blurred cover is drawn there at the sleeve's own size, and its last
 * column carries on to the right edge under the controls, calming into the page as it goes - the bottom
 * rows' job, turned, so the sleeve's soft right edge runs out into its own colours there too.
 */
fun androidx.compose.ui.draw.CacheDrawScope.sleeveWashAcross(
    palette: PagePalette, sleeveRight: Float, sleeveWidth: Float,
): androidx.compose.ui.draw.DrawResult {
    val endX = size.width
    val wash = palette.wash
    val page = palette.background
    if (wash == null) return onDrawBehind { drawRect(page) }
    val h = size.height.toInt().coerceAtLeast(1)
    val right = sleeveRight.coerceIn(1f, endX)
    val left = (right - sleeveWidth).coerceAtLeast(0f).toInt()
    val look = palette.look
    val floor = if (endX - right > 1f) Brush.horizontalGradient(
        stage.floorStops[0] to Color(look[CoverLook.FLOOR_0]),
        stage.floorStops[1] to Color(look[CoverLook.FLOOR_22]),
        stage.floorStops[2] to Color(look[CoverLook.FLOOR_75]),
        stage.floorStops[3] to page,
        startX = right, endX = endX,
    ) else null
    val floorLeft = Offset(right, 0f)
    val floorSize = Size(endX - right, size.height)
    return onDrawBehind {
        fun band(srcX: Int, srcW: Int, x0: Int, x1: Int) {
            if (x1 <= x0) return
            drawImage(
                wash,
                srcOffset = IntOffset(srcX, 0), srcSize = IntSize(srcW, WASH_ROWS),
                dstOffset = IntOffset(x0, 0), dstSize = IntSize(x1 - x0, h),
                filterQuality = FilterQuality.Low,
            )
        }
        val rightCol = right.toInt()
        band(0, 1, 0, if (left > 0) left + 1 else 0)
        band(WASH_ROWS - 1, 1, if (rightCol > left) rightCol - 1 else rightCol, endX.toInt())
        band(0, WASH_ROWS, left, rightCol)
        if (floor != null) drawRect(floor, topLeft = floorLeft, size = floorSize)
    }
}

/**
 * A row that scrolls sideways (a shelf of covers, a row of chips), run out past the page's sides on a phone on
 * its side, under the strips the page is kept off there - the camera's punch hole at one end, the tab rail
 * at the other - where its items fade out towards the screen's edge instead of stopping on a line, as a
 * list upright fades out under the floating bar. Its first and last items still start and end where the
 * page does: the row's padding takes the strips back ([edgePadding]). Upright there are no strips and this
 * is nothing.
 */
@Composable
fun Modifier.bleedsToEdges(): Modifier {
    val start = LocalPageStart.current
    val end = LocalPageEnd.current
    if (start == 0.dp && end == 0.dp) return this
    // The page's own colour laid over the row's ends, not a mask on a layer of the row's own: it reads the same
    // on the plain page the rows stand on, and there is no offscreen layer per row to flicker as it scrolls.
    val page = MaterialTheme.colorScheme.background
    return layout { measurable, constraints ->
        val w = constraints.maxWidth + start.roundToPx() + end.roundToPx()
        val placeable = measurable.measure(constraints.copy(minWidth = w, maxWidth = w))
        layout(constraints.maxWidth, placeable.height) { placeable.place(-start.roundToPx(), 0) }
    }.drawWithCache {
        val reach = EDGE_REACH.toPx()
        val l = start.toPx()
        val r = end.toPx()
        val inStart = if (l > 0f) l + reach else 0f
        val inFade = Brush.horizontalGradient(*edgeFade(page, l, reach), startX = inStart, endX = 0f)
        val outStart = if (r > 0f) size.width - r - reach else size.width
        val outFade = Brush.horizontalGradient(*edgeFade(page, r, reach), startX = outStart, endX = size.width)
        onDrawWithContent {
            drawContent()
            if (l > 0f) drawRect(inFade, size = Size(inStart, size.height))
            if (r > 0f) drawRect(outFade, topLeft = Offset(outStart, 0f), size = Size(size.width - outStart, size.height))
        }
    }
}

/**
 * A page's scrolling list that holds sideways rows, run out to the screen's edge over the strip the page is
 * kept off on its side (the camera's): a scrolling list lets its content spill only a little past its sides,
 * so rows inside one that ran out under the strip were cut where the list began. Its content is kept off the
 * strip by [pagePadding] instead. Upright there is no strip and this is nothing.
 */
@Composable
fun Modifier.pageToEdges(): Modifier {
    val start = LocalPageStart.current
    val end = LocalPageEnd.current
    if (start == 0.dp && end == 0.dp) return this
    return layout { measurable, constraints ->
        val w = constraints.maxWidth + start.roundToPx() + end.roundToPx()
        val placeable = measurable.measure(constraints.copy(minWidth = w, maxWidth = w))
        layout(constraints.maxWidth, placeable.height) { placeable.place(-start.roundToPx(), 0) }
    }
}

/**
 * A page's scroller that starts at the page's top, run on up under the status bar the page is kept below
 * ([LocalStatusRoom]), where what scrolls up fades out into the page's colour instead of stopping on a line.
 * Its content still starts where the page does: the scroller's top padding takes the room back. Nothing
 * where a page drawn under the status bar (an album's) is.
 */
@Composable
fun Modifier.underStatusBar(): Modifier {
    val room = LocalStatusRoom.current
    if (room == 0.dp) return this
    return layout { measurable, constraints ->
        val top = room.roundToPx()
        val tall = if (constraints.hasBoundedHeight) constraints.maxHeight + top else constraints.maxHeight
        val placeable = measurable.measure(constraints.copy(minHeight = (constraints.minHeight + top).coerceAtMost(tall), maxHeight = tall))
        layout(placeable.width, placeable.height - top) { placeable.place(0, -top) }
    }
}

/** The room a page is kept below the status bar by, which an [underStatusBar] scroller pads its content by. */
val LocalStatusRoom = androidx.compose.runtime.compositionLocalOf { 0.dp }

/** A [pageToEdges] list's padding: the strips it runs over at its sides (the camera's, the rail's), and [bottom]. */
@Composable
fun pagePadding(bottom: Dp, top: Dp = 0.dp): PaddingValues = PaddingValues(start = LocalPageStart.current, end = LocalPageEnd.current, top = top, bottom = bottom)

/** How far into the page an edge fade reaches past the strip beside it ([bleedsToEdges], the bar's upright). */
val EDGE_REACH = 24.dp

/**
 * The fade of what runs out under a strip at the screen's edge (the camera's, the chrome's) into [page]'s colour:
 * colour stops from [reach] px before the strip (0) to the screen's edge (1), across a strip [strip] px wide.
 * Untouched before the strip, still mostly there where it begins and thinning out across it, a trace left at
 * the edge: the chrome's glass over it has something to show. Gone by the strip's start, the pill was a pane
 * over plain colour.
 */
fun edgeFade(page: Color, strip: Float, reach: Float): Array<Pair<Float, Color>> {
    val at = (reach / (strip + reach)).coerceIn(0f, 1f)
    fun c(visible: Float) = page.copy(alpha = 1f - visible)
    val across = listOf(0.3f to 0.62f, 0.6f to 0.36f, 0.85f to 0.16f)
    return (listOf(0f to c(1f), at to c(0.86f)) + across.map { (x, v) -> at + (1f - at) * x to c(v) } + listOf(1f to c(0.06f))).toTypedArray()
}

/** A sideways row's padding: [horizontal] and [vertical], and at each end the strip it runs out under ([bleedsToEdges]). */
@Composable
fun edgePadding(horizontal: Dp = 0.dp, vertical: Dp = 0.dp): PaddingValues =
    PaddingValues(start = horizontal + LocalPageStart.current, end = horizontal + LocalPageEnd.current, top = vertical, bottom = vertical)

/** [edgePadding] with the same padding on every side. */
@Composable
fun edgePadding(all: Dp): PaddingValues = edgePadding(horizontal = all, vertical = all)

/**
 * How the app draws and times its pages - every gradient's stops, the waits, the fades and the meter's
 * pace - as nori-core says (`stage.rs`, `nori_look::sleeve`). Read once, the first time a page draws.
 */
val stage: dev.nori.music.ffi.Stage by lazy { dev.nori.music.ffi.stage() }

/**
 * Every word the screens say, from the app's string resources ([Say]): the fixed ones read once per
 * locale, the ones with a number or a name in them made when asked.
 */
val say: Say get() = Say.current

/**
 * How much of the sleeve's height goes soft at the bottom: the same share the colour of those rows is
 * averaged out of the wash at (`nori_look::cover::MELT`).
 */
val MELT: Float get() = stage.melt

/**
 * A gradient of [color] at the core's [stops], from [startY] to [endY] - or, [across], from x [startY] to
 * [endY]. Made once per size, never per frame.
 */
fun alphaGradient(stops: List<dev.nori.music.ffi.look.Stop>, color: Color, startY: Float, endY: Float, across: Boolean = false): Brush {
    val at = Array(stops.size) { stops[it].at to color.copy(alpha = stops[it].alpha) }
    return if (across) Brush.horizontalGradient(*at, startX = startY, endX = endY) else Brush.verticalGradient(*at, startY = startY, endY = endY)
}

/** The wash texture is this many pixels a side (nori_look's `WASH_OUT`). */
private const val WASH_ROWS = CoverLook.WASH

/**
 * A hairline in the Apple sense: a dim line that starts where the text starts and never reaches the
 * right edge of the screen. Drawn, not laid out, so a long list does not pay for a divider composable.
 */
@Composable
fun Hairline(startIndent: Dp = Space.gutter) {
    val color = LocalContentColor.current.copy(alpha = 0.10f)
    Box(Modifier.fillMaxWidth().padding(start = startIndent).height(1.dp).background(color))
}

/**
 * A section heading sitting on the gutter, with an optional [action] on the right. With [onMore] the whole
 * header opens the section's full view, and a quiet chevron ends it.
 */
@Composable
fun SectionHeader(
    title: String, modifier: Modifier = Modifier, onMore: (() -> Unit)? = null,
    action: @Composable (RowScope.() -> Unit)? = null,
    /** Closer to the rows it heads: a disc's name inside one list. */
    compact: Boolean = false,
) {
    Row(
        modifier.fillMaxWidth()
            .then(if (onMore != null) Modifier.clickable(onClick = onMore) else Modifier)
            .padding(start = Space.gutter, end = if (action != null) Space.tight else Space.gutter, top = if (compact) 12.dp else Space.section, bottom = if (compact) 2.dp else 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title, Modifier.weight(1f),
            style = MaterialTheme.typography.titleLarge.copy(fontSize = 19.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.2).sp),
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        action?.invoke(this)
        if (onMore != null) Icon(
            Iconoir.NavArrowRight, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** The small capitals over a [GroupCard]: "PLAYBACK". */
@Composable
fun GroupLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        remember(text) { text.uppercase() },
        modifier.padding(start = 32.dp, end = 32.dp, top = 22.dp, bottom = 8.dp),
        style = MaterialTheme.typography.labelSmall.copy(fontSize = 11.5.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.06.em),
        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
    )
}

/**
 * Rows grouped on one raised plate: the surface, a hairline edge, the tile's corners, inset from the page's
 * sides. Rows inside are [GroupRow]s, or anything that ends in a [Hairline] starting where its text does,
 * the last without one.
 */
@Composable
fun GroupCard(modifier: Modifier = Modifier, content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) {
    val scheme = MaterialTheme.colorScheme
    // The content color spelled out: Material resolves it to Unspecified for a color that is not one of
    // its roles, and text inside then drew almost black.
    Surface(
        modifier.fillMaxWidth().padding(horizontal = GROUP_MARGIN),
        shape = TileShape, color = scheme.surfaceContainer, contentColor = scheme.onSurface, border = hairlineBorder(),
    ) { Column(content = content) }
}

/** How far a [GroupCard] stands in from the page's sides. */
private val GROUP_MARGIN = 16.dp
/** How far a [GroupRow]'s content stands in from the card's sides, and the gap after its icon. */
internal val GROUP_INSET = 14.dp
private val GROUP_ICON = 20.dp

/**
 * A row of a [GroupCard]: an optional [icon] in ink, a title and maybe a quiet line under it, a quiet [value],
 * then [trailing] (a switch) or a [chevron]. Its [divider] is the hairline under it, starting where the text
 * does; the last row of a card goes without.
 */
@Composable
fun GroupRow(
    title: String,
    onClick: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    value: String? = null,
    icon: ImageVector? = null,
    chevron: Boolean = false,
    divider: Boolean = true,
    minHeight: Dp = 52.dp,
    trailing: (@Composable RowScope.() -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = minHeight)
                .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                .padding(horizontal = GROUP_INSET, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (icon != null) {
                Icon(icon, null, Modifier.size(GROUP_ICON), tint = scheme.onSurface)
                Spacer(Modifier.width(GROUP_INSET))
            }
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (!subtitle.isNullOrEmpty()) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
            }
            if (!value.isNullOrEmpty()) Text(
                value, Modifier.padding(start = 10.dp), style = MaterialTheme.typography.bodyMedium.copy(fontSize = 13.5.sp),
                color = scheme.onSurfaceVariant, maxLines = 1,
            )
            trailing?.invoke(this)
            if (chevron) Icon(
                Iconoir.NavArrowRight, null, Modifier.padding(start = 4.dp).size(20.dp), tint = scheme.onSurfaceVariant,
            )
        }
        if (divider) Hairline(startIndent = if (icon != null) GROUP_INSET * 2 + GROUP_ICON else GROUP_INSET)
    }
}

/** The name of a whole screen, the size Apple sets it: big, bold, sitting on the gutter. */
@Composable
fun LargeTitle(text: String, modifier: Modifier = Modifier, trailing: @Composable (RowScope.() -> Unit)? = null) {
    Row(
        modifier.fillMaxWidth().padding(start = Space.gutter, end = Space.tight, top = 8.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text, Modifier.weight(1f),
            style = titled(MaterialTheme.typography.headlineMedium.copy(fontSize = 31.sp)),
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        trailing?.invoke(this)
    }
}

/** A page opened from another: back, and its name. */
@Composable
fun BackTitle(text: String) {
    val nav = LocalNav.current
    Row(Modifier.padding(start = 4.dp, end = Space.gutter), verticalAlignment = Alignment.CenterVertically) {
        androidx.compose.material3.IconButton(nav::back) { Icon(Iconoir.ArrowLeft, say.back, Modifier.size(22.dp)) }
        Text(text, Modifier.weight(1f), style = titled(MaterialTheme.typography.headlineSmall), maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Small grey capitals under a title: "2019 · ALTERNATIVE · LOSSLESS". */
@Composable
fun Caption(text: String, modifier: Modifier = Modifier, align: TextAlign = TextAlign.Start, caps: Boolean = true) {
    if (text.isEmpty()) return
    // The page's quieter text colour, read while drawing (the player's page moves under it).
    val look = LocalLook.current
    LookText(
        if (caps) remember(text) { text.uppercase() } else text, { look.color(CoverLook.ON_VARIANT) }, modifier,
        style = MaterialTheme.typography.labelSmall, textAlign = align, maxLines = 2, overflow = TextOverflow.Ellipsis,
    )
}

/** How tall a page's [PillButton] stands. */
private val ACTION_SIDE = 46.dp

/**
 * The button the page is built around: a full pill, tinted with the page's accent. [prominent] is the
 * one the eye should land on (Play); the others sit on a translucent version of the same colour.
 * On a paper-white page the plates go darker so they do not wash out into the background - and the
 * darkening is continuous in the page's lightness, so a cross-fade onto white does not flip the
 * Play button black in one frame.
 */
@Composable
fun PillButton(
    text: String, icon: ImageVector?, onClick: () -> Unit, modifier: Modifier = Modifier,
    prominent: Boolean = false, enabled: Boolean = true,
) {
    // The plates lean darker as the page gets paler, continuously (nori_look::dress): looked up.
    val look = LocalLook.current
    val container = look.color(if (prominent) CoverLook.PILL else CoverLook.PILL_PLATE)
    val content = look.color(if (prominent) CoverLook.PILL_INK else CoverLook.TINT_INK)
    Surface(
        onClick = onClick, enabled = enabled, shape = PillShape, color = container, contentColor = content,
        modifier = modifier.heightIn(min = ACTION_SIDE),
    ) {
        Row(Modifier.padding(horizontal = 16.dp), Arrangement.Center, Alignment.CenterVertically) {
            if (icon != null) Icon(icon, null, Modifier.size(18.dp))
            Text(
                text, Modifier.padding(start = if (icon != null) 7.dp else 0.dp),
                style = MaterialTheme.typography.titleSmall.copy(fontSize = 15.sp), maxLines = 1,
            )
        }
    }
}

/**
 * A short message on a pill lifted off the page (the chrome's slab, edge and shadow: it floats over
 * whatever is under it), its action in ink and its optional × quiet: the app's messages at the top and
 * the queue's undo at its foot. Colors are read while drawing, so a look cross-fading under it redraws it
 * without recomposing. [behind] sits under the words.
 */
@Composable
fun MessagePill(
    text: String, look: Look, modifier: Modifier = Modifier,
    maxLines: Int = 1, action: String? = null, actionEnabled: Boolean = true, onAction: () -> Unit = {},
    onDismiss: (() -> Unit)? = null, behind: @Composable androidx.compose.foundation.layout.BoxScope.() -> Unit = {},
) {
    val ink = ColorProducer { look.color(CoverLook.CHROME_CONTENT) }
    Box(
        modifier.drawBehind {
            val round = androidx.compose.ui.geometry.CornerRadius(size.height / 2)
            drawRoundRect(look.color(CoverLook.SURFACE_CONTAINER), cornerRadius = round)
            drawRoundRect(look.color(CoverLook.OUTLINE_VARIANT), cornerRadius = round, style = androidx.compose.ui.graphics.drawscope.Stroke(0.5.dp.toPx()))
        },
    ) {
        behind()
        Row(Modifier.padding(start = 18.dp, end = 4.dp).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
            LookText(
                text, ink, Modifier.weight(1f, fill = false).padding(vertical = 10.dp),
                style = MaterialTheme.typography.bodyMedium, maxLines = maxLines, overflow = TextOverflow.Ellipsis,
            )
            if (action != null) {
                androidx.compose.material3.TextButton(onAction, enabled = actionEnabled) {
                    LookText(action, ink, style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.SemiBold), maxLines = 1)
                }
            }
            if (onDismiss != null) {
                IconButton(onDismiss) { LookIcon(Iconoir.Xmark, words(R.string.close), Modifier.size(20.dp)) { look.color(CoverLook.CHROME_CONTENT_75) } }
            } else if (action == null) {
                Spacer(Modifier.width(14.dp))
            }
        }
    }
}

/** The app's own scheme under a page's [TintedTheme] (null outside one): sheets wear it wherever they open. */
val LocalAppScheme = androidx.compose.runtime.staticCompositionLocalOf<androidx.compose.material3.ColorScheme?> { null }

/**
 * Dresses everything inside in the colours of one cover: the page colour becomes the surface, the
 * cover's accent becomes the primary, and text colours are chosen to read on it. Screens keep using
 * `MaterialTheme.colorScheme`, so nothing below needs to know where the colours came from.
 */
@Composable
fun TintedTheme(palette: PagePalette?, content: @Composable () -> Unit) {
    val base = MaterialTheme.colorScheme
    val scheme = androidx.compose.runtime.remember(palette, base) { palette?.let { base.dressedIn(it.fixed) } ?: base }
    MaterialTheme(colorScheme = scheme) {
        androidx.compose.runtime.CompositionLocalProvider(
            LocalAppScheme provides (LocalAppScheme.current ?: base),
            LocalContentColor provides scheme.onSurface,
            LocalPalette provides palette,
            LocalLook provides (palette?.fixed ?: LocalLook.current),
        ) { content() }
    }
}

/** This scheme with the page's roles put in from [l]. */
private fun androidx.compose.material3.ColorScheme.dressedIn(l: Look): androidx.compose.material3.ColorScheme {
    val on = l.color(CoverLook.ON)
    return copy(
        background = l.color(CoverLook.BACKGROUND), surface = l.color(CoverLook.BACKGROUND),
        onBackground = on, onSurface = on, onSurfaceVariant = l.color(CoverLook.ON_VARIANT),
        surfaceVariant = l.color(CoverLook.SURFACE_VARIANT),
        surfaceContainer = l.color(CoverLook.SURFACE_CONTAINER),
        surfaceContainerHigh = l.color(CoverLook.SURFACE_CONTAINER_HIGH),
        primary = l.color(CoverLook.ACCENT), onPrimary = l.color(CoverLook.ON_PRIMARY),
        secondaryContainer = l.color(CoverLook.SECONDARY_CONTAINER), onSecondaryContainer = on,
        outlineVariant = l.color(CoverLook.OUTLINE_VARIANT),
    )
}

/**
 * Status bar icons follow the colour of the page they sit on, and go back to the app's when it leaves.
 * The look says which (a light page gets dark icons); a page that is changing colour is followed as it
 * goes, without recomposing anything.
 */
@Composable
fun SystemBarIcons(look: Look) {
    val view = androidx.compose.ui.platform.LocalView.current
    val controller = remember(view) {
        (view.context as? android.app.Activity)?.window?.let { androidx.core.view.WindowCompat.getInsetsController(it, view) }
    }
    androidx.compose.runtime.DisposableEffect(controller) {
        val before = controller?.isAppearanceLightStatusBars
        onDispose { if (before != null) controller.isAppearanceLightStatusBars = before }
    }
    LaunchedEffect(controller, look) {
        if (controller == null) return@LaunchedEffect
        androidx.compose.runtime.snapshotFlow { look.argb(CoverLook.STATUS_LIGHT) != 0 }
            .collect { controller.isAppearanceLightStatusBars = it }
    }
}

/**
 * A search or filter field as a capsule raised on the surface with a hairline edge, outlined in the accent
 * while it has the focus: an icon, and a clear button that only exists when there is something to clear.
 */
@Composable
fun SearchField(
    value: String, onValue: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier,
    testTag: String? = null, autofocus: Boolean = false, focusKey: Any = Unit,
) {
    val scheme = MaterialTheme.colorScheme
    // Asked for by hand: opening search focuses the field so the keyboard is already there.
    val focus = remember(autofocus) { if (autofocus) FocusRequester() else null }
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    // A focus request needs the field's node attached, which it is once the field is placed.
    var placed by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    LaunchedEffect(focus, focusKey, placed) {
        if (focus == null || !placed) return@LaunchedEffect
        focus.requestFocus()
        // Focus alone does not raise the keyboard when it lands before the window is ready.
        keyboard?.show()
    }
    Surface(
        shape = PillShape, color = scheme.surfaceContainer, contentColor = scheme.onSurface, modifier = modifier.fillMaxWidth(),
        border = if (focused) androidx.compose.foundation.BorderStroke(HAIRLINE * 2, scheme.primary) else hairlineBorder(),
    ) {
        Row(Modifier.padding(horizontal = 14.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Iconoir.Search, null, Modifier.size(19.dp), tint = scheme.onSurfaceVariant)
            // The padding belongs to the box, not to the field: with it on the field the placeholder sat
            // at the top of the capsule while the typed text sat in the middle of it.
            Box(Modifier.weight(1f).padding(horizontal = 8.dp, vertical = 11.dp)) {
                if (value.isEmpty()) Text(placeholder, style = MaterialTheme.typography.bodyLarge, color = scheme.onSurfaceVariant)
                androidx.compose.foundation.text.BasicTextField(
                    value, onValue, singleLine = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = scheme.onSurface),
                    cursorBrush = androidx.compose.ui.graphics.SolidColor(scheme.primary),
                    modifier = Modifier.fillMaxWidth().onFocusChanged { focused = it.isFocused }
                        .then(if (testTag != null) Modifier.testTag(testTag) else Modifier)
                        .then(if (focus != null) Modifier.focusRequester(focus).onPlaced { placed = true } else Modifier),
                )
            }
            if (value.isNotEmpty()) androidx.compose.material3.IconButton({ onValue("") }, Modifier.size(28.dp)) {
                Icon(Iconoir.Xmark, say.clear, Modifier.size(17.dp), tint = scheme.onSurfaceVariant)
            }
        }
    }
}

/** A choice chip: the selected one raised on the surface with a hairline edge and in ink, the others quiet words. */
@Composable
fun Chip(label: String, selected: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    // The pill is 34 dp: Material's 48 dp touch floor would add 7 dp of air above and below it.
    CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides Dp.Unspecified) { Surface(
        onClick = onClick, shape = PillShape,
        color = if (selected) scheme.surfaceContainer else Color.Transparent,
        contentColor = if (selected) scheme.onSurface else scheme.onSurfaceVariant,
        border = if (selected) hairlineBorder() else null,
        modifier = modifier,
    ) {
        Box(Modifier.height(34.dp).padding(horizontal = 14.dp), Alignment.Center) {
            Text(label, style = MaterialTheme.typography.labelLarge.copy(fontSize = 14.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        }
    } }
}

/**
 * A slider drawn rather than assembled: a rounded track, a fill and a dot. Material's Slider brings a
 * ripple, a state layer and a value label, which on a screen of ten equalizer bands reads as ten
 * widgets instead of one curve - and costs a layer each. [centred] fills outwards from zero, which is
 * what a gain control should look like.
 */
@Composable
fun NoriSlider(
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    onChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    centred: Boolean = false,
) {
    val scheme = MaterialTheme.colorScheme
    val track = scheme.onSurface.copy(alpha = if (enabled) 0.16f else 0.07f)
    val fill = if (enabled) scheme.onSurface else scheme.onSurface.copy(alpha = 0.25f)
    val knob = if (enabled) Color.White else Color(0xFFBDBDBD)
    val span = (range.endInclusive - range.start).takeIf { it > 0f } ?: 1f
    val fraction = ((value - range.start) / span).coerceIn(0f, 1f)
    val pick: (Float, Float) -> Unit = { x, w -> onChange(range.start + (x / w).coerceIn(0f, 1f) * span) }
    Box(
        modifier.fillMaxWidth().height(40.dp)
            .pointerInput(enabled, range) {
                if (!enabled) return@pointerInput
                detectHorizontalDragGestures(
                    onDragStart = { pick(it.x, size.width.toFloat()) },
                ) { change, _ -> pick(change.position.x, size.width.toFloat()) }
            }
            .pointerInput(enabled, range) {
                if (!enabled) return@pointerInput
                detectTapGestures { pick(it.x, size.width.toFloat()) }
            }
            .drawBehind {
                // UISlider's proportions: a 4 pt track and a 28 pt white knob sitting on a soft shadow.
                // The old 6 dp track with a 17 dp grey knob was Material's shape in Apple's colours.
                val h = 4.dp.toPx()
                val y = (size.height - h) / 2f
                val radius = androidx.compose.ui.geometry.CornerRadius(h / 2f, h / 2f)
                drawRoundRect(track, androidx.compose.ui.geometry.Offset(0f, y), androidx.compose.ui.geometry.Size(size.width, h), radius)
                val from = if (centred) size.width * ((0f - range.start) / span).coerceIn(0f, 1f) else 0f
                val to = size.width * fraction
                drawRoundRect(
                    fill, androidx.compose.ui.geometry.Offset(minOf(from, to), y),
                    androidx.compose.ui.geometry.Size(kotlin.math.abs(to - from), h), radius,
                )
                val r = 14.dp.toPx()
                val cx = to.coerceIn(r, size.width - r)
                drawCircle(Color.Black.copy(alpha = if (enabled) 0.18f else 0.08f), r + 1.dp.toPx(), androidx.compose.ui.geometry.Offset(cx, size.height / 2f + 1.5f.dp.toPx()))
                drawCircle(knob, r, androidx.compose.ui.geometry.Offset(cx, size.height / 2f))
            },
    )
}

/**
 * A form field with the app's corners and no hard outline: a soft filled capsule-ish box, the way a
 * settings form looks on iOS. It keeps Material's text field underneath, so labels, password masking
 * and keyboard options all behave exactly as before - only the frame changes.
 */
@Composable
fun FormField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    label: @Composable (() -> Unit)? = null,
    placeholder: @Composable (() -> Unit)? = null,
    supportingText: @Composable (() -> Unit)? = null,
    singleLine: Boolean = false,
    minLines: Int = 1,
    visualTransformation: androidx.compose.ui.text.input.VisualTransformation = androidx.compose.ui.text.input.VisualTransformation.None,
    keyboardOptions: androidx.compose.foundation.text.KeyboardOptions = androidx.compose.foundation.text.KeyboardOptions.Default,
) {
    val scheme = MaterialTheme.colorScheme
    val filled = LocalLook.current.color(CoverLook.FORM)
    androidx.compose.material3.OutlinedTextField(
        value, onValueChange, modifier, label = label, placeholder = placeholder, supportingText = supportingText,
        singleLine = singleLine, minLines = minLines,
        visualTransformation = visualTransformation, keyboardOptions = keyboardOptions,
        shape = CardShape,
        colors = androidx.compose.material3.OutlinedTextFieldDefaults.colors(
            focusedContainerColor = filled, unfocusedContainerColor = filled,
            focusedBorderColor = scheme.primary.copy(alpha = 0.6f),
            unfocusedBorderColor = Color.Transparent,
        ),
    )
}

/**
 * The list row every screen shares: something on the left (a picture, or a few characters such as a
 * rank), a title and maybe a second line, a value or a chevron on the right, and a hairline that starts
 * where the text does. Without [onClick] it only says something.
 */
@Composable
fun NavRow(
    title: String,
    onClick: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    trailing: String? = null,
    leading: (@Composable () -> Unit)? = null,
    leadingText: String? = null,
    chevron: Boolean = false,
    divider: Boolean = true,
    /** How many lines the subtitle may take. */
    subtitleLines: Int = 1,
    action: (@Composable RowScope.() -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                .padding(start = Space.gutter, end = if (action != null) 4.dp else Space.gutter, top = 11.dp, bottom = 11.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (leading != null) { leading(); Spacer(Modifier.size(12.dp)) }
            if (leadingText != null) Text(leadingText, Modifier.width(LEADING_TEXT), style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant)
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (!subtitle.isNullOrEmpty()) Text(
                    subtitle, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant,
                    maxLines = subtitleLines, overflow = TextOverflow.Ellipsis,
                )
            }
            if (!trailing.isNullOrEmpty()) Text(
                trailing, Modifier.padding(start = 10.dp), style = MaterialTheme.typography.bodyMedium,
                color = scheme.onSurfaceVariant,
            )
            action?.invoke(this)
            if (chevron) Icon(
                Iconoir.NavArrowRight, null,
                Modifier.padding(start = 6.dp).size(19.dp), tint = scheme.onSurfaceVariant.copy(alpha = 0.7f),
            )
        }
        if (divider) Hairline(startIndent = Space.gutter + when { leading != null -> 60.dp; leadingText != null -> LEADING_TEXT; else -> 0.dp })
    }
}

/** The width a row's leading characters (a rank) stand in. */
private val LEADING_TEXT = 28.dp

/** A word in ink that does something when tapped: a dialog's buttons, a row's "Edit". [destructive] is in the error color. */
@Composable
fun TextAction(label: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, destructive: Boolean = false) {
    val scheme = MaterialTheme.colorScheme
    androidx.compose.material3.TextButton(
        onClick, modifier, enabled = enabled,
        colors = androidx.compose.material3.ButtonDefaults.textButtonColors(contentColor = if (destructive) scheme.error else scheme.onSurface),
    ) { Text(label) }
}

/** A row that does something rather than going somewhere: "New playlist", "Import M3U…". */
@Composable
fun ActionRow(title: String, icon: ImageVector, onClick: () -> Unit, divider: Boolean = true) {
    Column {
        Row(
            Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = Space.gutter, vertical = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurface)
            Text(title, Modifier.padding(start = 12.dp), style = MaterialTheme.typography.bodyLarge)
        }
        if (divider) Hairline(startIndent = Space.gutter + 32.dp)
    }
}

/**
 * A bare glyph that does something: the small actions either side of a page's Play pill. No plate: the
 * glyph is in the page's ink, and in its accent while [lit] (Shuffle while shuffle is on, a stamped seal).
 * [quiet] draws it in the page's quieter ink. It keeps its full strength while disabled: a washed-out Shuffle looked absent while Play was not yet
 * tappable. [iconModifier] reaches the glyph alone, for an animation of it.
 */
@Composable
fun GlyphButton(
    icon: ImageVector, description: String, modifier: Modifier = Modifier,
    enabled: Boolean = true, lit: Boolean = false, quiet: Boolean = false, iconModifier: Modifier = Modifier, onClick: () -> Unit,
) {
    val look = LocalLook.current
    Box(
        modifier.size(GLYPH_TARGET).clickable(
            enabled = enabled, role = androidx.compose.ui.semantics.Role.Button, interactionSource = null,
            indication = androidx.compose.material3.ripple(bounded = false, radius = GLYPH_TARGET / 2), onClick = onClick,
        ),
        Alignment.Center,
    ) {
        LookIcon(icon, description, Modifier.size(24.dp).then(iconModifier)) { look.color(if (lit) CoverLook.ACCENT else if (quiet) CoverLook.ON_VARIANT else CoverLook.ON) }
    }
}

/**
 * A favorite's seal being stamped down by a tap ([down]): it lands from half as big again and settles,
 * with a firm tap under the finger. Taken off, it goes at once, and a seal that comes or goes by itself
 * (another song, another device) is simply drawn.
 */
@androidx.compose.runtime.Stable
class Stamp(private val scope: kotlinx.coroutines.CoroutineScope, private val haptics: androidx.compose.ui.hapticfeedback.HapticFeedback, private val plain: Boolean) {
    val scale = androidx.compose.animation.core.Animatable(1f)

    fun down() {
        haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.Confirm)
        if (plain) return
        scope.launch {
            scale.snapTo(1.5f)
            scale.animateTo(1f, androidx.compose.animation.core.spring(dampingRatio = 0.5f, stiffness = 700f))
        }
    }
}

@Composable
fun rememberStamp(): Stamp {
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val haptics = androidx.compose.ui.platform.LocalHapticFeedback.current
    val plain = reduceMotion()
    return remember(scope, haptics, plain) { Stamp(scope, haptics, plain) }
}

/** A favorite's seal ([Kanji.Sealed]), a tap away from being stamped or taken off; [tint] when it is one, [muted] its outline when not. */
@Composable
fun FavoriteSeal(
    starred: Boolean,
    modifier: Modifier = Modifier,
    size: Dp = 22.dp,
    tint: Color = MaterialTheme.colorScheme.primary,
    muted: Color = LocalContentColor.current.copy(alpha = 0.75f),
    onClick: () -> Unit,
) {
    val stamp = rememberStamp()
    IconButton({ if (!starred) stamp.down(); onClick() }, modifier) {
        Icon(
            if (starred) Kanji.Sealed else Kanji.Seal,
            if (starred) say.removeFromFavourites else say.addToFavourites,
            Modifier.size(size).graphicsLayer { scaleX = stamp.scale.value; scaleY = stamp.scale.value },
            tint = if (starred) tint else muted,
        )
    }
}

/** Favorite as a [GlyphButton], the seal in the page's accent when it is one, stamped as [FavoriteSeal] is. */
@Composable
fun FavoriteButton(starred: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val stamp = rememberStamp()
    GlyphButton(
        if (starred) Kanji.Sealed else Kanji.Seal,
        if (starred) say.removeFromFavourites else say.favourite,
        modifier, lit = starred,
        iconModifier = Modifier.graphicsLayer { scaleX = stamp.scale.value; scaleY = stamp.scale.value },
        onClick = { if (!starred) stamp.down(); onClick() },
    )
}

/** The ⋯ that holds whatever did not fit beside a page's Play button. */
@Composable
fun MoreButton(items: List<Pair<String, () -> Unit>>, modifier: Modifier = Modifier) {
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        GlyphButton(Iconoir.MoreHoriz, say.more) { open = true }
        NoriPopover(open, { open = false }) {
            items.forEach { (label, action) -> PopoverItem(label) { action(); open = false } }
        }
    }
}


/**
 * Whether movement is kept to short fades or none ([Motion] Reduced or E-ink, the user's choice, whatever
 * Android's animation setting says). Read it rather than hard-coding durations, so it means the same
 * thing everywhere.
 */
@Composable
fun reduceMotion(): Boolean {
    val prefs by (androidx.lifecycle.viewmodel.compose.viewModel<dev.nori.music.app.vm.SettingsViewModel>()).prefs.collectAsStateWithLifecycle()
    return prefs.motion != dev.nori.music.ffi.settings.Motion.FULL
}

/**
 * The E-ink motion, provided once by App: the player redraws only when something changes (a song, play
 * or pause, a seek, a touch, the next line of lyrics), so its seek bar and playing bars do not run on their own.
 */
val LocalEinkScreen = androidx.compose.runtime.compositionLocalOf { false }

/** How much room a [GlyphButton] takes for a finger. */
private val GLYPH_TARGET = 48.dp

/**
 * How fast every animation in the app runs. Compose scales each one by whatever MotionDurationScale is
 * in its coroutine's context, and by default that is Android's animator duration scale - so with system
 * animations off, every tween and spring in the app finished on its first frame. The activity starts
 * the whole composition with this object in its context instead (see MainActivity), so every animation
 * there is, ours and the libraries', runs at full speed: nori's own Motion setting is what reduces them.
 */
object AppMotion : androidx.compose.ui.MotionDurationScale {
    /**
     * The user's reduce-motion answer, for code that must not look it up per call - a cover in a grid
     * of a hundred cannot each collect the settings. Kept current by App; [reduceMotion] is the same
     * answer for everything else.
     */
    @Volatile var reduce = false
    /** The E-ink motion: every animation ends on its first frame. Kept current by App. */
    @Volatile var still = false
    override val scaleFactor: Float get() = if (still) 0f else 1f
}


/**
 * A switch drawn the way iOS draws one: UISwitch's 51 by 31 pt track and a 27 pt white thumb on a
 * soft shadow, the track filling with the page's accent when on. Material's switch - a thin outlined
 * pill whose thumb grows when it is on - was the most Android-looking thing left in settings.
 *
 * The thumb slides when it is tapped, which is the one kind of motion this app allows: something the
 * user touched, answering. Nothing moves otherwise.
 */
@Composable
fun NoriSwitch(checked: Boolean, onCheckedChange: ((Boolean) -> Unit)?, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val scheme = MaterialTheme.colorScheme
    val t by androidx.compose.animation.core.animateFloatAsState(
        if (checked) 1f else 0f, androidx.compose.animation.core.tween(if (reduceMotion()) 0 else 180), label = "switch",
    )
    // The season's color for what is on, not its accent: the accent is kept for the one current thing.
    val on = scheme.secondary
    val off = LocalLook.current.color(CoverLook.SWITCH_OFF)
    val track = androidx.compose.ui.graphics.lerp(off, on, t).let { if (enabled) it else it.copy(alpha = 0.4f) }
    // The thumb is white, except over a near-white track (the monochrome theme's), where it would vanish.
    val thumb = when {
        !enabled -> Color(0xFFE0E0E0)
        on.luminance() > 0.8f -> androidx.compose.ui.graphics.lerp(Color.White, scheme.onSecondary, t)
        else -> Color.White
    }
    Box(
        modifier.size(width = 51.dp, height = 31.dp)
            .then(
                if (onCheckedChange != null) Modifier.clickable(
                    enabled = enabled,
                    interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() },
                    indication = null,
                ) { onCheckedChange(!checked) } else Modifier,
            )
            .semantics { role = androidx.compose.ui.semantics.Role.Switch; toggleableState = androidx.compose.ui.state.ToggleableState(checked) }
            .drawBehind {
                val h = size.height
                drawRoundRect(track, cornerRadius = androidx.compose.ui.geometry.CornerRadius(h / 2f, h / 2f))
                val r = 13.5f.dp.toPx()
                val pad = 2.dp.toPx()
                val cx = pad + r + (size.width - 2 * (pad + r)) * t
                drawCircle(Color.Black.copy(alpha = 0.16f), r + 0.5f.dp.toPx(), androidx.compose.ui.geometry.Offset(cx, h / 2f + 1.dp.toPx()))
                drawCircle(thumb, r, androidx.compose.ui.geometry.Offset(cx, h / 2f))
            },
    )
}

/** A decibel figure with its sign, one decimal, never "-0.0" (nori-core's `fmt::signed_db`). */
fun signedDb(db: Float): String = dev.nori.music.text.Fmt.signedDb(db)

/**
 * A menu dropped from what opened it: raised on the surface with a hairline edge and the tile's corners,
 * no shadow. Its rows are [PopoverItem]s.
 */
@Composable
fun NoriPopover(
    expanded: Boolean, onDismissRequest: () -> Unit, modifier: Modifier = Modifier,
    content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit,
) {
    androidx.compose.material3.DropdownMenu(
        expanded, onDismissRequest, modifier,
        shape = TileShape, containerColor = MaterialTheme.colorScheme.surfaceContainer,
        tonalElevation = 0.dp, shadowElevation = 0.dp, border = hairlineBorder(), content = content,
    )
}

/** A row of a [NoriPopover]; the [chosen] one is marked with a quiet check. */
@Composable
fun PopoverItem(label: String, chosen: Boolean = false, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().widthIn(min = 160.dp, max = 280.dp).heightIn(min = 44.dp)
            .clickable(onClick = onClick).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, color = scheme.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (chosen) Icon(Iconoir.Check, say.chosen, Modifier.padding(start = 12.dp).size(18.dp), tint = scheme.onSurfaceVariant)
    }
}


/**
 * Something is on its way: three dots breathing one after another, the mark Apple's lyrics show for an
 * instrumental break. Quiet on purpose - it says "coming", not "look at me". It stays invisible for the
 * first quarter-second, so anything that arrives quickly never shows a loader at all, and then fades
 * in rather than appearing. Only animates while it is on screen; with reduce motion it holds still.
 */
@Composable
fun LoadingDots(modifier: Modifier = Modifier, dot: androidx.compose.ui.unit.Dp = 7.dp, color: Color = Color.Unspecified) {
    // The page's text colour, read while drawing: a page changing colour under it only redraws the dots.
    val look = LocalLook.current
    val plain = AppMotion.reduce
    val appear = remember { androidx.compose.animation.core.Animatable(0f) }
    androidx.compose.runtime.LaunchedEffect(Unit) {
        kotlinx.coroutines.delay(250)
        appear.animateTo(1f, androidx.compose.animation.core.tween(350))
    }
    val phase = if (plain) null else androidx.compose.animation.core.rememberInfiniteTransition(label = "dots").animateFloat(
        0f, 1f,
        androidx.compose.animation.core.infiniteRepeatable(androidx.compose.animation.core.tween(1300, easing = androidx.compose.animation.core.LinearEasing)),
        label = "dots",
    )
    androidx.compose.foundation.Canvas(modifier.size(dot * 4.4f, dot).graphicsLayer { alpha = appear.value }) {
        val r = size.height / 2f
        val gap = (size.width - size.height * 3f) / 2f
        val ink = if (color.isSpecified) color else look.color(dev.nori.music.look.CoverLook.ON)
        for (i in 0..2) {
            // Each dot swells and brightens in turn, a third of a cycle behind the one before it.
            val t = phase?.value?.let { ((it - i / 3f) % 1f + 1f) % 1f } ?: 0.5f
            val pulse = 0.5f - 0.5f * kotlin.math.cos(t * 2f * Math.PI.toFloat())
            drawCircle(
                ink.copy(alpha = 0.22f + 0.5f * pulse),
                radius = r * (0.78f + 0.22f * pulse),
                center = androidx.compose.ui.geometry.Offset(r + i * (size.height + gap), r),
            )
        }
    }
}

/**
 * A soft sheen gliding across a placeholder while its picture is on the way, so a slow cover reads as
 * loading rather than missing. Like [LoadingDots] it waits a quarter-second before showing and fades
 * in, so a cover that comes from the cache never shimmers. Draw-phase only: a running sheen redraws
 * one layer and recomposes nothing, and when [active] goes false it stops entirely. [leaving]: the load
 * is over with nothing to cover the sheen, so it fades out (in [SHEEN_LEAVE_MS]) rather than vanishing;
 * the caller drops it after that.
 */
/** How long a sheen whose load is over takes to fade out. */
const val SHEEN_LEAVE_MS = 300

fun Modifier.loadingSheen(active: Boolean, leaving: Boolean = false): Modifier = if (!active) this else composed {
    val appear = remember { androidx.compose.animation.core.Animatable(0f) }
    androidx.compose.runtime.LaunchedEffect(leaving) {
        if (leaving) appear.animateTo(0f, androidx.compose.animation.core.tween(SHEEN_LEAVE_MS))
        else {
            kotlinx.coroutines.delay(250)
            appear.animateTo(1f, androidx.compose.animation.core.tween(400))
        }
    }
    val plain = AppMotion.reduce
    val sweep = if (plain) null else androidx.compose.animation.core.rememberInfiniteTransition(label = "sheen").animateFloat(
        0f, 1f,
        androidx.compose.animation.core.infiniteRepeatable(
            androidx.compose.animation.core.tween(1500, easing = androidx.compose.animation.core.FastOutSlowInEasing),
            initialStartOffset = androidx.compose.animation.core.StartOffset(0),
        ),
        label = "sheen",
    )
    val look = LocalLook.current
    drawWithCache {
        // One band, made once per size and colour and slid across by translation, not a new gradient on
        // every frame of every loading cover.
        val color = look.color(dev.nori.music.look.CoverLook.ON)
        val band = size.width * 0.9f
        val sheen = Brush.linearGradient(
            0f to Color.Transparent, 0.5f to color.copy(alpha = 0.10f), 1f to Color.Transparent,
            start = androidx.compose.ui.geometry.Offset(-band / 2f, 0f),
            end = androidx.compose.ui.geometry.Offset(band / 2f, size.height),
        )
        val still = color.copy(alpha = 0.05f)
        onDrawWithContent {
            drawContent()
            val a = appear.value
            if (a <= 0f) return@onDrawWithContent
            if (sweep == null) { drawRect(still, alpha = a); return@onDrawWithContent }
            val x = -band + sweep.value * (size.width + band * 2f)
            translate(left = x) {
                drawRect(sheen, topLeft = androidx.compose.ui.geometry.Offset(-x, 0f), size = size, alpha = a)
            }
        }
    }
}

/**
 * Play, pause, or "starting": the glyph changes by cross-fading with a slight scale, never by swapping
 * in one frame. The spinner only comes in when the wait is long enough to notice - past 300 ms - since
 * most skips start playing within that, and a spinner flicking in and out of the pause button for a
 * frame was one of the things that made skipping feel rough.
 *
 * While it waits for that, the wait is "playing": the player is not playing yet, but it is going to
 * (that is what [buffering] means - waiting *with* play-when-ready), and showing the play arrow in the
 * meantime said "paused" for a fraction of a second after every skip. A run of quick skips flashed it
 * on and off with every press.
 */
@Composable
fun PlayPauseGlyph(
    playing: Boolean, buffering: Boolean, size: androidx.compose.ui.unit.Dp, spinner: androidx.compose.ui.unit.Dp,
    /** The glyph's colour read while drawing; the content colour when null. */
    tint: androidx.compose.ui.graphics.ColorProducer? = null,
) {
    var busy by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(false) }
    androidx.compose.runtime.LaunchedEffect(buffering) {
        if (buffering) kotlinx.coroutines.delay(stage.spinnerAfterMs)
        busy = buffering
    }
    // Which glyph is the core's (`transport_glyph`), asked over JNI when one of the three changes.
    val glyph = androidx.compose.runtime.remember(playing, buffering, busy) {
        dev.nori.music.ffi.TransportGlyph.entries[dev.nori.music.look.CoverLook.transportGlyph(playing, buffering, busy)]
    }
    androidx.compose.animation.AnimatedContent(
        glyph,
        transitionSpec = {
            (androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(180)) +
                androidx.compose.animation.scaleIn(androidx.compose.animation.core.tween(180), initialScale = 0.8f)) togetherWith
                (androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(140)) +
                    androidx.compose.animation.scaleOut(androidx.compose.animation.core.tween(140), targetScale = 0.8f))
        },
        contentAlignment = Alignment.Center,
        label = "playPause",
    ) { g ->
        Box(Modifier.size(size), Alignment.Center) {
            when (g) {
                dev.nori.music.ffi.TransportGlyph.SPINNER -> androidx.compose.material3.CircularProgressIndicator(Modifier.size(spinner), color = tint?.invoke() ?: androidx.compose.material3.LocalContentColor.current, strokeWidth = 2.dp)
                dev.nori.music.ffi.TransportGlyph.PAUSE -> if (tint != null) LookIcon(Iconoir.PauseSolid, say.pause, Modifier.fillMaxSize(), tint)
                    else androidx.compose.material3.Icon(Iconoir.PauseSolid, say.pause, Modifier.fillMaxSize())
                else -> if (tint != null) LookIcon(Iconoir.PlaySolid, say.play, Modifier.fillMaxSize(), tint)
                    else androidx.compose.material3.Icon(Iconoir.PlaySolid, say.play, Modifier.fillMaxSize())
            }
        }
    }
}
