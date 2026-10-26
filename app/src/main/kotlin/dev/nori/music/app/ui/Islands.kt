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

/** One island's view: where its element is, cut to the part of it that shows, stepping itself between draws. */
class Island internal constructor(context: Context) : View(context) {
    internal var drawing: IslandDrawing? = null
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
        val wait = drawing?.draw(canvas, width, height) ?: -1L
        if (wait >= 0) postOnAnimationDelayed(step, wait)
    }
}

/** Where an element is in the window: its top left, its size, and the part of it not cut off by what holds it. */
class IslandSpot {
    var x = 0f
    var y = 0f
    var width = 0
    var height = 0
    val visible = Rect()
    /** Nothing of it cut off by the list or panel holding it. */
    var whole = false

    internal fun update(c: LayoutCoordinates) {
        val at = c.positionInWindow()
        val b = c.boundsInWindow()
        x = at.x
        y = at.y
        width = c.size.width
        height = c.size.height
        visible.set(b.left.toInt(), b.top.toInt(), kotlin.math.ceil(b.right).toInt(), kotlin.math.ceil(b.bottom).toInt())
        whole = b.width >= width - 0.5f && b.height >= height - 0.5f
    }
}

/** An element's island: whether it is drawn there now ([apart]), and how to ask it to draw again ([poke]). */
class IslandSlot internal constructor(private val islands: Islands?) {
    internal var drawing: IslandDrawing? = null
    internal var wanted = false
    internal var overPage = false
    internal val spot = IslandSpot()
    private var clear by mutableStateOf(false)
    private var island: Island? = null

    /** Drawn by the island, so the element draws nothing itself. Read in composition. */
    val apart: Boolean get() = wanted && clear

    /** Asks the island to draw again, for a change it cannot see (a colour, playing again). */
    fun poke() {
        if (apart) island?.invalidate()
    }

    internal fun placed(c: LayoutCoordinates) {
        spot.update(c)
        recheck()
        if (apart) island?.place(spot)
    }

    internal fun recheck() {
        val i = islands ?: return
        clear = spot.whole && (!overPage || i.clearOf(spot.visible))
    }

    internal fun sync() {
        val i = islands ?: return
        if (apart) {
            val view = island ?: i.take().also { island = it }
            view.drawing = drawing
            view.place(spot)
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
 * dragged, not fading) and while the element shows whole and, [overPage], clear of everything drawn over the
 * page. Place it with [island] on the element; draw nothing there while [IslandSlot.apart].
 */
@Composable
fun rememberIsland(drawing: IslandDrawing, wanted: Boolean, overPage: Boolean): IslandSlot {
    val islands = LocalIslands.current
    val slot = remember(islands) { IslandSlot(islands) }
    slot.drawing = drawing
    slot.wanted = wanted && islands != null
    slot.overPage = overPage
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
