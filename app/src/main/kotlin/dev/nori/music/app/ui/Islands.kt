package dev.nori.music.app.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionContext
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCompositionContext
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.platform.ComposeView

/**
 * Small things that move by themselves - the playing row's bars, the seek bar - drawn by views of their own
 * over the Compose window rather than inside it. Compose repaints its whole window for any change inside
 * it, so a step drawn there is a frame of the whole screen; a step drawn by an [Island] repaints its own
 * few pixels. Each is used only while nothing else moves where it is ([rememberIsland]); otherwise Compose
 * draws the thing in its place among the page's layers, where a frame being drawn anyway costs nothing more.
 *
 * The islands inside an [IslandContent] are in a layer of their own within it, [outer] being the window's:
 * they move with the content they are part of, and share the window's holds and covers.
 */
class Islands(context: Context, private val outer: Islands? = null) : ViewGroup(context) {
    // Its islands are sized and placed by their elements, in the frame the element is placed
    // ([Island.place]), never by a layout pass of this view: a size asked for halfway through a frame
    // waited for the next pass, and one asked for while drawing could be dropped, leaving an island as
    // large as it last was, or nothing, for good. Content islands are measured again at that size here,
    // for a layout their own content asks for.
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        setMeasuredDimension(getDefaultSize(0, widthMeasureSpec), getDefaultSize(0, heightMeasureSpec))
        for (i in 0 until childCount) (getChildAt(i) as? IslandHost)?.measureAgain()
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        for (i in 0 until childCount) (getChildAt(i) as? IslandHost)?.let { it.layout(0, 0, it.measuredWidth, it.measuredHeight) }
    }

    /** What the app draws over the page (chrome, banners), by who drew it, in window pixels: see [coversPage]. */
    val covers: MutableMap<Any, Rect> = outer?.covers ?: mutableStateMapOf()

    fun clearOf(r: Rect): Boolean = covers.values.none { Rect.intersects(it, r) }

    /**
     * What is moving or fading the screen round the islands' elements now (a page sliding, a panel
     * dissolving, the player's sheet or colours on their way): see [HoldIslands]. An island is drawn
     * over the window, outside those animations, so while any runs none draws and Compose draws them all.
     */
    val holds: MutableList<Any> = outer?.holds ?: mutableStateListOf()

    /**
     * A hold starting stops every island in that frame. An element being moved or faded is not always drawn
     * again (a panel leaving is moved and faded as it last drew), so waiting for its drawing to take it back
     * left its island drawing it on top of the copy Compose moves in. Released, an island starts again only
     * from its element's drawing ([IslandSlot.handOff]).
     */
    internal fun hold(key: Any) {
        (outer ?: this).let { it.holds.add(key); it.stopAll() }
    }

    internal fun stopAll() {
        for (i in 0 until childCount) (getChildAt(i) as? IslandView)?.stop()
    }

    internal fun take(): Island = Island(context).also { addView(it, LayoutParams(0, 0)) }
    /** Under every drawn island: what is drawn over the content goes over it. */
    internal fun take(host: IslandHost): IslandHost = host.also { addView(it, 0, LayoutParams(0, 0)) }
    internal fun give(island: View) = removeView(island)
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

/** An island's view, whichever kind: the slot it serves places it, and tells it to stop. */
internal interface IslandView {
    var slot: IslandSlot?
    fun place(spot: IslandSpot)
    fun hide()
    /** Draws nothing from this frame until its element's drawing hands it the element again. */
    fun stop()
    /** The element's drawing handed it the element, or took it back ([IslandSlot.told]), in this drawing pass. */
    fun handed()
}

/**
 * Moves [view] to [spot] (in the window) within its parent, which starts wherever the activity's content
 * does - not at the window's corner under a cutout or a system bar set beside it - cut to what shows.
 */
private fun place(view: View, spot: IslandSpot, layer: IntArray, cut: Rect) {
    (view.parent as? View)?.getLocationInWindow(layer)
    view.translationX = spot.x - layer[0]
    view.translationY = spot.y - layer[1]
    cut.set(spot.visible)
    cut.offset(-spot.x.toInt(), -spot.y.toInt())
    view.clipBounds = cut
    if (view.visibility == View.GONE) { view.visibility = View.VISIBLE; view.invalidate() }
}

/**
 * One island's view: where its element is, cut to the part of it that shows, stepping itself between draws.
 * It is there for as long as its element wants it, and draws only what the element's own drawing last left
 * to it ([IslandSlot.handOff]), told in that same drawing pass: the two never both draw the element, and
 * never neither, not even for the frame a hand-over happens in.
 */
class Island internal constructor(context: Context) : View(context), IslandView {
    override var slot: IslandSlot? = null
    private var margin = 0
    private val step = Runnable { invalidate() }
    private val cut = Rect()
    private val layer = IntArray(2)

    init {
        visibility = GONE
    }

    override fun place(spot: IslandSpot) {
        if (width != spot.width || height != spot.height) layout(0, 0, spot.width, spot.height)
        margin = spot.margin
        place(this, spot, layer, cut)
    }

    override fun hide() {
        visibility = GONE
        removeCallbacks(step)
    }

    override fun stop() {
        slot?.told = false
        removeCallbacks(step)
        invalidate()
    }

    override fun handed() = invalidate()

    override fun onDraw(canvas: Canvas) {
        removeCallbacks(step)
        val slot = slot ?: return
        if (!slot.told) return
        val m = margin.toFloat()
        canvas.translate(m, m)
        val wait = slot.drawing?.draw(canvas, width - 2 * margin, height - 2 * margin) ?: -1L
        if (wait >= 0) postOnAnimationDelayed(step, wait)
    }
}

/**
 * The view an [IslandContent] composes its content in: the content's one composition, drawn two ways. Apart,
 * this view draws it over the window, so a change in it repaints this view's pixels alone. Otherwise the
 * window draws this view in the content's place, inside whatever moves or fades it there, and this view
 * draws nothing itself - Compose's own AndroidView is drawn so. Touches come here either way.
 */
class IslandHost internal constructor(context: Context, parent: CompositionContext, outer: Islands) : FrameLayout(context), IslandView {
    override var slot: IslandSlot? = null
    internal val content = mutableStateOf<@Composable () -> Unit>({})
    /** The content's own islands, which move with it. */
    private val inner = Islands(context, outer)
    /** Draws the content's place in the window again; set by that place while it is there. */
    internal var redrawPlace: (() -> Unit)? = null
    private var forPlace = false
    private val cut = Rect()
    private val layer = IntArray(2)

    // Invisible whenever the window draws the content: the window drawing this view marks it drawn, so a
    // picture of its own made while it was apart was never made again, and stayed on screen over the
    // window's fading copy until the panel was gone. Invisible, it shows nothing of its own and takes no
    // touches; made visible, it makes its picture afresh.
    init {
        visibility = INVISIBLE
        addView(ComposeView(context).apply {
            setParentCompositionContext(parent)
            setContent { CompositionLocalProvider(LocalIslands provides inner) { content.value() } }
        })
        addView(inner)
    }

    override fun place(spot: IslandSpot) {
        if (width != spot.width || height != spot.height) {
            measure(exactly(spot.width), exactly(spot.height))
            layout(0, 0, spot.width, spot.height)
        }
        place(this, spot, layer, cut)
    }

    internal fun measureAgain() {
        if (width > 0) measure(exactly(width), exactly(height))
    }

    override fun hide() {
        visibility = GONE
    }

    // The window draws the content's place again in that frame: a panel leaving is faded as it last drew,
    // and its last drawing, made while this view drew the content, had nothing there.
    override fun stop() {
        slot?.told = false
        inner.stopAll()
        visibility = INVISIBLE
        redrawPlace?.invoke()
    }

    override fun handed() {
        visibility = if (slot?.told == true) VISIBLE else INVISIBLE
    }

    override fun draw(canvas: Canvas) {
        if (forPlace || slot?.told == true) super.draw(canvas)
    }

    /** Draws the content on [canvas], the window's, at the content's place. */
    internal fun drawAt(canvas: Canvas) {
        forPlace = true
        try { draw(canvas) } finally { forPlace = false }
    }

    // A change in the content while the window draws it is the window's to draw again: its picture of this
    // view is only made anew as it draws that place.
    override fun onDescendantInvalidated(child: View, target: View) {
        super.onDescendantInvalidated(child, target)
        if (slot?.told != true) redrawPlace?.invoke()
    }

    private fun exactly(size: Int) = View.MeasureSpec.makeMeasureSpec(size, View.MeasureSpec.EXACTLY)
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
class IslandSlot internal constructor(private val islands: Islands?, private val make: (Islands) -> IslandView) {
    internal var drawing: IslandDrawing? = null
    /** The caller's own condition, as state like [clear]: a draw that reads [apart] is drawn again when either changes. */
    internal var wanted by mutableStateOf(false)
    internal var overPage = false
    internal var margin = 0
    /** Where the element must lie, in the window, for the island to have it (a list's unfaded middle); null for anywhere. */
    internal var within: Rect? = null
    internal val spot = IslandSpot()
    private var clear by mutableStateOf(false)
    private var island: IslandView? = null
    private var placedOnce = false

    /** Drawn by the island, so the element draws nothing itself. Read in composition or while drawing. */
    val apart: Boolean get() = wanted && clear && islands?.holds?.isEmpty() == true

    /** What the element's drawing last left to the island. */
    internal var told = false

    /**
     * For the element's own drawing, every time it draws: whether the island has it now, so the element draws
     * nothing. A change is passed to the island in this drawing pass, so it draws, or stops, in the same frame.
     */
    fun handOff(): Boolean {
        val a = apart
        if (a != told) { told = a; island?.handed() }
        return a
    }

    /** Asks the island to draw again, for a change it cannot see (a colour, playing again). */
    fun poke() {
        if (told) (island as? View)?.invalidate()
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
        if (now != clear) clear = now
    }

    /** After each composition: the island is there while the element wants it, drawing what it is given now. */
    internal fun sync() {
        val i = islands ?: return
        if (wanted) {
            val view = island ?: make(i).also { island = it; it.slot = this }
            if (placedOnce) view.place(spot)
            (view as View).invalidate()
        } else { told = false; island?.hide() }
    }

    internal fun release() {
        (island as? View)?.let { islands?.give(it) }
        island = null
    }
}

/**
 * An island for one element, drawn with [drawing] while [wanted] (the caller's own "nothing moves here": not
 * dragged, not fading) and while the element shows whole, inside [within], and, [overPage], clear of
 * everything drawn over the page; [margin] pixels round it are its too. Place it with [island] on the
 * element, and draw nothing there while [IslandSlot.handOff], asked in the element's drawing.
 */
@Composable
fun rememberIsland(drawing: IslandDrawing, wanted: Boolean, overPage: Boolean, margin: Int = 0, within: Rect? = null): IslandSlot {
    val islands = LocalIslands.current
    val slot = rememberSlot(islands, wanted, overPage, margin, within) { it.take() }
    slot.drawing = drawing
    return slot
}

@Composable
private fun rememberSlot(islands: Islands?, wanted: Boolean, overPage: Boolean, margin: Int, within: Rect?, make: (Islands) -> IslandView): IslandSlot {
    val slot = remember(islands) { IslandSlot(islands, make) }
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

/**
 * [content] composed in an island of its own ([IslandHost]), for content that changes much while all round it
 * stays put - the lyrics gliding from line to line. It is drawn apart while nothing moves where it is and it
 * shows whole; then a change in it repaints only its own place. Anything inside it that changes has to be
 * drawn by it, and nothing may be drawn over it but islands.
 */
@Composable
fun IslandContent(modifier: Modifier, content: @Composable () -> Unit) {
    val islands = LocalIslands.current
    if (islands == null) { Box(modifier) { content() }; return }
    val parent = rememberCompositionContext()
    val host = remember(islands) { IslandHost(islands.context, parent, islands) }
    SideEffect { host.content.value = content }
    val slot = rememberSlot(islands, wanted = true, overPage = false, margin = 0, within = null) { it.take(host) }
    Box(modifier.island(slot).then(HostPlaceElement(slot, host)))
}

/** An [IslandContent]'s place in the window: draws the content there while it is not drawn apart. */
private data class HostPlaceElement(val slot: IslandSlot, val host: IslandHost) : ModifierNodeElement<HostPlace>() {
    override fun create() = HostPlace(slot, host)
    override fun update(node: HostPlace) {
        node.slot = slot
        if (node.host !== host) { node.host.redrawPlace = null; node.host = host; if (node.isAttached) host.redrawPlace = node.redraw }
    }
}

private class HostPlace(var slot: IslandSlot, var host: IslandHost) : Modifier.Node(), DrawModifierNode {
    val redraw = { invalidateDraw() }
    override fun onAttach() { host.redrawPlace = redraw }
    override fun onDetach() { host.redrawPlace = null }
    override fun ContentDrawScope.draw() {
        if (!slot.handOff()) host.drawAt(drawContext.canvas.nativeCanvas)
    }
}

/**
 * Holds every island off while [moving]: for an animation that moves or fades part of the screen an island's
 * element may be in, which the island, drawn over the window, would not follow.
 */
@Composable
fun HoldIslands(moving: Boolean) {
    val islands = LocalIslands.current ?: return
    val key = remember { Any() }
    DisposableEffect(islands, moving) {
        if (moving) islands.hold(key)
        onDispose { islands.holds.remove(key) }
    }
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
