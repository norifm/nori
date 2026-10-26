package dev.nori.music.app.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.Rect
import android.view.View
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow

/**
 * Small things that move by themselves - the playing row's bars, the seek bar - drawn by views of their own
 * over the Compose window rather than inside it. Compose repaints its whole window for any change inside
 * it, so a step drawn there is a frame of the whole screen; a step drawn by an [Island] repaints its own
 * few pixels. Each is used only while nothing else moves where it is ([rememberIsland]); otherwise Compose
 * draws the thing in its place among the page's layers, where a frame being drawn anyway costs nothing more.
 */
class Islands(context: Context) : FrameLayout(context) {
    /** What the app draws over the page (chrome, banners), by who drew it, in window pixels: see [coversPage]. */
    val covers = mutableStateMapOf<Any, Rect>()

    fun clearOf(r: Rect): Boolean = covers.values.none { Rect.intersects(it, r) }

    internal fun take(): Island = Island(context).also { addView(it, LayoutParams(1, 1)) }
    internal fun give(island: Island) = removeView(island)
}

/** The islands over the window, or null where there are none (previews). */
val LocalIslands = staticCompositionLocalOf<Islands?> { null }

/**
 * What an island draws, on the main thread, in its own pixels, from its top left. Returns the ms before it
 * draws again: 0 for the next frame, -1 for not until [IslandSlot.poke].
 */
fun interface IslandDrawing {
    fun draw(canvas: Canvas, width: Int, height: Int): Long
}

/**
 * One island's view: where its element is, cut to the part of it that shows, stepping itself between draws.
 * It is there for as long as its element wants it, and draws only while [IslandSlot.apart] - read while
 * drawing, as the element reads it while drawing itself, in the same frame - so the two never both draw
 * the element, and never neither.
 */
class Island internal constructor(context: Context) : View(context) {
    internal var slot: IslandSlot? = null
    internal var drawing: IslandDrawing? = null
    internal var margin = 0
    private val step = Runnable { invalidate() }
    private val cut = Rect()

    init {
        visibility = GONE
    }

    internal fun place(spot: IslandSpot) {
        if (layoutParams.width != spot.width || layoutParams.height != spot.height) {
            layoutParams = layoutParams.apply { width = spot.width; height = spot.height }
        }
        translationX = spot.x
        translationY = spot.y
        margin = spot.margin
        cut.set(spot.visible)
        cut.offset(-spot.x.toInt(), -spot.y.toInt())
        clipBounds = cut
        if (visibility != VISIBLE) { visibility = VISIBLE; invalidate() }
    }

    internal fun hide() {
        visibility = GONE
        removeCallbacks(step)
    }

    override fun onDraw(canvas: Canvas) {
        removeCallbacks(step)
        if (slot?.apart != true) return
        val m = margin.toFloat()
        canvas.translate(m, m)
        val wait = drawing?.draw(canvas, width - 2 * margin, height - 2 * margin) ?: -1L
        if (wait >= 0) postOnAnimationDelayed(step, wait)
    }
}

/**
 * Where an island is in the window: its element with [margin] round it (for what the element draws past its
 * edges, a glow), and the part of that not cut off by what holds the element.
 */
class IslandSpot {
    var x = 0f
    var y = 0f
    var width = 0
    var height = 0
    var margin = 0
    val visible = Rect()
    /** Nothing of the element cut off by the list or panel holding it. */
    var whole = false

    internal fun update(c: LayoutCoordinates, m: Int) {
        val at = c.positionInWindow()
        val b = c.boundsInWindow()
        margin = m
        x = at.x - m
        y = at.y - m
        width = c.size.width + 2 * m
        height = c.size.height + 2 * m
        visible.set(b.left.toInt() - m, b.top.toInt() - m, kotlin.math.ceil(b.right).toInt() + m, kotlin.math.ceil(b.bottom).toInt() + m)
        whole = b.width >= c.size.width - 0.5f && b.height >= c.size.height - 0.5f
    }
}

/** An element's island: whether it is drawn there now ([apart]), and how to ask it to draw again ([poke]). */
class IslandSlot internal constructor(private val islands: Islands?) {
    internal var drawing: IslandDrawing? = null
    /** The caller's own condition, as state like [clear]: a draw that reads [apart] is drawn again when either changes. */
    internal var wanted by mutableStateOf(false)
    internal var overPage = false
    internal var margin = 0
    /** Where the element must lie, in the window, for the island to have it (a list's unfaded middle); null for anywhere. */
    internal var within: Rect? = null
    internal val spot = IslandSpot()
    private var clear by mutableStateOf(false)
    private var island: Island? = null
    private var placedOnce = false

    /** Drawn by the island, so the element draws nothing itself. Read in composition or while drawing. */
    val apart: Boolean get() = wanted && clear

    /** Asks the island to draw again, for a change it cannot see (a colour, playing again). */
    fun poke() {
        if (apart) island?.invalidate()
    }

    internal fun placed(c: LayoutCoordinates) {
        spot.update(c, margin)
        placedOnce = true
        recheck()
        island?.place(spot)
    }

    internal fun recheck() {
        val i = islands ?: return
        val now = spot.whole && (!overPage || i.clearOf(spot.visible)) && within.let { it == null || it.contains(spot.visible) }
        if (now != clear) { clear = now; island?.invalidate() }
    }

    /** After each composition: the island is there while the element wants it, drawing what it is given now. */
    internal fun sync() {
        val i = islands ?: return
        if (wanted) {
            val view = island ?: i.take().also { island = it; it.slot = this }
            view.drawing = drawing
            if (placedOnce) view.place(spot)
            view.invalidate()
        } else island?.hide()
    }

    internal fun release() {
        island?.let { islands?.give(it) }
        island = null
    }
}

/**
 * An island for one element, drawn with [drawing] while [wanted] (the caller's own "nothing moves here": not
 * dragged, not fading) and while the element shows whole, inside [within], and, [overPage], clear of
 * everything drawn over the page; [margin] pixels round it are its too. Place it with [island] on the
 * element; draw nothing there while [IslandSlot.apart].
 */
@Composable
fun rememberIsland(drawing: IslandDrawing, wanted: Boolean, overPage: Boolean, margin: Int = 0, within: Rect? = null): IslandSlot {
    val islands = LocalIslands.current
    val slot = remember(islands) { IslandSlot(islands) }
    slot.drawing = drawing
    (wanted && islands != null).let { if (slot.wanted != it) slot.wanted = it }
    slot.overPage = overPage
    slot.margin = margin
    slot.within = within
    DisposableEffect(slot) { onDispose { slot.release() } }
    SideEffect { slot.sync() }
    // Something drawn over the page coming or going (a banner, the chrome moving) is checked again here; the
    // element's own moves are checked as it is placed.
    if (islands != null && overPage && slot.wanted) LaunchedEffect(slot) {
        snapshotFlow { islands.covers.values.toList() }.collect { slot.recheck() }
    }
    return slot
}

/** Tells [slot] where this element is, as it is placed. */
fun Modifier.island(slot: IslandSlot): Modifier = if (!slot.wanted) this else onGloballyPositioned { slot.placed(it) }

/**
 * Marks this as drawn over the page, so an island on the page under it hands its element back to Compose,
 * which draws it under this, while they meet.
 */
@Composable
fun Modifier.coversPage(): Modifier {
    val islands = LocalIslands.current ?: return this
    val key = remember { Any() }
    DisposableEffect(islands) { onDispose { islands.covers.remove(key) } }
    return onGloballyPositioned { c ->
        val b = c.boundsInWindow()
        val r = Rect(b.left.toInt(), b.top.toInt(), kotlin.math.ceil(b.right).toInt(), kotlin.math.ceil(b.bottom).toInt())
        if (islands.covers[key] != r) islands.covers[key] = r
    }
}
