package dev.nori.music.app.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.addOutline
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.nori.music.ffi.library.Art
import dev.nori.music.ffi.library.Reading
import dev.nori.music.ffi.library.Sketch
import dev.nori.music.look.CoverLook

/*
 * A loading page's placeholders, as the core sketches it (`sketch`): plates in the page's quiet ink
 * (CoverLook.SKETCH) where its items will stand, each with its real shape and place, so the answer
 * cross-fades in over them and nothing moves. They wait out the core's quick-load time and fade in, so a
 * page that answers at once never shows them; with Motion Full a slow band of a lighter ink passes over
 * them. The band is one clock for the whole sketch, read while drawing: it redraws the plates and
 * recomposes nothing, and it stops when the sketch leaves the screen.
 */

/** The shimmer's place in its pass, 0 to 1; null while the plates stand still. */
private val LocalSheen = staticCompositionLocalOf<State<Float>?> { null }

/** A page's placeholders for [reading], under [top] (what the page shows of itself before it reads: its title). */
@Composable
fun PageSketch(reading: Reading, modifier: Modifier = Modifier, padding: PaddingValues = PaddingValues(), top: @Composable ColumnScope.() -> Unit = {}) {
    val parts = remember(reading) { dev.nori.music.ffi.library.sketch(reading, false) }
    Column(modifier.fillMaxSize().clipToBounds().padding(padding)) {
        top()
        // As tall as its parts, whatever room is left: the page cuts them at its foot.
        SketchShown(Modifier.wrapContentHeight(Alignment.Top, unbounded = true)) { Column { parts.forEach { SketchPart(it) } } }
    }
}

/** The placeholders for the part of a page still on its way below what it already shows ([sketch] with the head known). */
fun LazyListScope.sketchItem(reading: Reading) = item(key = "sketch", contentType = "sketch") {
    val parts = remember(reading) { dev.nori.music.ffi.library.sketch(reading, true) }
    SketchShown(itemMotion()) { Column { parts.forEach { SketchPart(it) } } }
}

/** [content] once the wait outlasts the core's quick-load time, faded in, its plates shimmering with Motion Full. */
@Composable
fun SketchShown(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    var due by remember { mutableStateOf(false) }
    val shown = remember { Animatable(0f) }
    LaunchedEffect(Unit) {
        kotlinx.coroutines.delay(stage.quickLoadMs)
        due = true
        if (AppMotion.reduce) shown.snapTo(1f) else shown.animateTo(1f, AppMotion.fadeIn())
    }
    // Composed only once due, so nothing runs through the wait, and only while the sketch is on screen.
    if (!due) return
    val sheen = if (AppMotion.reduce) null else rememberInfiniteTransition(label = "sketch").animateFloat(
        0f, 1f, infiniteRepeatable(tween(stage.sheenMs, easing = LinearEasing)), label = "sketch",
    )
    CompositionLocalProvider(LocalSheen provides sheen) {
        Box(modifier.graphicsLayer { alpha = shown.value }) { content() }
    }
}

/** A plate of [shape] in the page's quiet ink, the shimmer's band passing over it left to right. */
private fun Modifier.plate(shape: Shape): Modifier = composed {
    val sheen = LocalSheen.current
    val look = LocalLook.current
    drawWithCache {
        val outline = shape.createOutline(size, layoutDirection, this)
        val clip = Path().apply { addOutline(outline) }
        val plate = look.color(CoverLook.SKETCH)
        val band = size.width.coerceAtLeast(size.height) * 0.6f
        val light = Brush.horizontalGradient(listOf(plate, look.color(CoverLook.SKETCH_SHEEN), plate), startX = 0f, endX = band)
        onDrawBehind {
            drawOutline(outline, plate)
            val at = sheen?.value ?: return@onDrawBehind
            clipPath(clip) { translate(left = -band + at * (size.width + band)) { drawRect(light, size = Size(band, size.height)) } }
        }
    }
}

/** A bar where a line of [style] will stand, [share] of the width, as tall as the line's capitals. */
@Composable
private fun LineBar(style: TextStyle, share: Float, modifier: Modifier = Modifier, align: Alignment = Alignment.CenterStart) {
    val density = LocalDensity.current
    val line = with(density) { style.lineHeight.toDp() }
    val bar = with(density) { style.fontSize.toDp() } * 0.62f
    Box(modifier.fillMaxWidth().height(line), align) { Box(Modifier.fillMaxWidth(share).height(bar).plate(PillShape)) }
}

/** Widths for the lines of a list, so its rows read as titles of their own lengths. */
private val SHARES = floatArrayOf(0.62f, 0.46f, 0.74f, 0.54f, 0.68f, 0.4f, 0.58f, 0.7f)

/** Rows and grids run to the page's end; the page clips the last. */
private const val FILL = 14

@Composable
private fun SketchPart(part: Sketch) {
    val type = MaterialTheme.typography
    when (part) {
        is Sketch.Head -> Column(Modifier.fillMaxWidth().padding(bottom = Space.section), horizontalAlignment = Alignment.CenterHorizontally) {
            if (part.art == Art.ROUND) Box(Modifier.padding(top = 56.dp).size(200.dp).plate(CircleShape))
            else Box(Modifier.fillMaxWidth().aspectRatio(1f).plate(androidx.compose.ui.graphics.RectangleShape))
            LineBar(type.headlineSmall, 0.6f, Modifier.padding(top = 16.dp).padding(horizontal = 48.dp), Alignment.Center)
            LineBar(type.bodyLarge, 0.36f, Modifier.padding(top = 4.dp).padding(horizontal = 48.dp), Alignment.Center)
            Row(Modifier.padding(top = 18.dp, start = Space.gutter, end = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                repeat(2) { Box(Modifier.weight(1f).height(ACTION_SIDE).plate(PillShape)) }
            }
        }
        is Sketch.Shelf -> Column {
            // A section header's room, its title a bar.
            LineBar(type.titleLarge, 0.36f, Modifier.padding(start = Space.gutter, end = Space.gutter, top = Space.section, bottom = 12.dp))
            // Its cards run past the page's edge, as a shelf's do.
            Row(Modifier.fillMaxWidth().clipToBounds().wrapContentWidth(Alignment.Start, unbounded = true).padding(horizontal = Space.gutter), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                repeat(6) { i -> Card(part.art, Modifier.width(PRINT), i) }
            }
        }
        is Sketch.Grid -> BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Space.gutter)) {
            val columns = if (LocalWide.current) ((maxWidth + GRID_GAP) / (GRID_MIN + GRID_GAP)).toInt().coerceAtLeast(2) else 2
            Column(verticalArrangement = Arrangement.spacedBy(GRID_GAP)) {
                repeat(FILL / columns + 1) { r ->
                    Row(horizontalArrangement = Arrangement.spacedBy(GRID_GAP)) { repeat(columns) { c -> Card(part.art, Modifier.weight(1f), r * columns + c) } }
                }
            }
        }
        is Sketch.Rows -> Column { repeat(FILL) { i -> SketchRow(part.art, i) } }
    }
}

/** A card: its picture, then its name and its line, as [CoverCard] lays them. */
@Composable
private fun Card(art: Art, modifier: Modifier, i: Int) = Column(modifier) {
    val round = art == Art.ROUND
    Box(Modifier.fillMaxWidth().aspectRatio(1f).plate(if (round) CircleShape else RoundedCornerShape(Radius.card)))
    val type = MaterialTheme.typography
    val align = if (round) Alignment.Center else Alignment.CenterStart
    LineBar(type.bodyMedium, SHARES[i % SHARES.size] + 0.2f, Modifier.padding(top = 8.dp), align)
    LineBar(type.bodySmall, SHARES[(i + 3) % SHARES.size] * 0.7f, align = align)
}

/** A list row: its picture (or a song's number), then its two lines, as [SongRow] lays them. */
@Composable
private fun SketchRow(art: Art, i: Int) {
    val type = MaterialTheme.typography
    // As tall as the real row: its picture, or the ⋯ beside a song's number.
    val (side, padding) = if (art == Art.ROUND) 48.dp to 7.dp else 44.dp to 9.dp
    Row(Modifier.fillMaxWidth().padding(start = Space.gutter, end = Space.gutter, top = padding, bottom = padding).heightIn(min = if (art == Art.NUMBER) 40.dp else side), verticalAlignment = Alignment.CenterVertically) {
        val gap: Dp = when (art) {
            Art.SQUARE -> { Box(Modifier.size(side).plate(RoundedCornerShape(Radius.cover))); 12.dp }
            Art.ROUND -> { Box(Modifier.size(side).plate(CircleShape)); 12.dp }
            Art.NUMBER -> { Box(Modifier.width(26.dp), Alignment.Center) { Box(Modifier.size(12.dp, 9.dp).plate(PillShape)) }; 14.dp }
            Art.BARE -> 0.dp
        }
        Column(Modifier.weight(1f).padding(start = gap, end = 40.dp)) {
            LineBar(type.bodyLarge, SHARES[i % SHARES.size])
            if (art != Art.NUMBER) LineBar(type.bodySmall, SHARES[(i + 3) % SHARES.size] * 0.6f)
        }
    }
}
