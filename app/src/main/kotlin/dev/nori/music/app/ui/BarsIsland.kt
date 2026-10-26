package dev.nori.music.app.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned

/**
 * The playing row's bars, drawn by a view of their own over the Compose page while it stands still (see
 * [PlayingBars]): a step here repaints these few pixels, where one drawn inside Compose repaints the
 * whole window. The row places it and lets it go; between those it steps itself.
 */
class BarsIsland(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val step = Runnable { invalidate() }
    private val cut = android.graphics.Rect()

    /** What the app draws over the page (chrome, banners), by who drew it, in window pixels: see [coversPage]. */
    val covers = androidx.compose.runtime.mutableStateMapOf<Any, android.graphics.Rect>()

    /** Whether [r] is clear of everything drawn over the page. */
    fun clearOf(r: android.graphics.Rect): Boolean = covers.values.none { android.graphics.Rect.intersects(it, r) }

    init {
        visibility = GONE
    }

    /** Shows the bars where [spot] says, in [tint]. */
    fun place(spot: BarsSpot, tint: Int) {
        if (layoutParams.width != spot.side) layoutParams = layoutParams.apply { width = spot.side; height = spot.side }
        translationX = spot.x
        translationY = spot.y
        cut.set(spot.visible)
        cut.offset(-spot.x.toInt(), -spot.y.toInt())
        clipBounds = cut
        if (paint.color != tint) { paint.color = tint; invalidate() }
        if (visibility != VISIBLE) { visibility = VISIBLE; invalidate() }
    }

    fun hide() {
        visibility = GONE
        removeCallbacks(step)
    }

    override fun onDraw(canvas: Canvas) {
        val t = barsClock()
        val w = width.toFloat() / (BARS * 2 - 1)
        val h = height.toFloat()
        for (i in 0 until BARS) {
            val bar = h * barLevel(t, i)
            canvas.drawRoundRect(i * w * 2f, h - bar, i * w * 2f + w, h, w / 2f, w / 2f, paint)
        }
        removeCallbacks(step)
        postOnAnimationDelayed(step, BARS_STEP_MS)
    }
}

/**
 * Marks this as drawn over the page, so the playing row's bars are drawn inside Compose, under it, rather
 * than by [BarsIsland] on top of it, while they meet.
 */
@androidx.compose.runtime.Composable
fun androidx.compose.ui.Modifier.coversPage(): androidx.compose.ui.Modifier {
    val island = LocalBarsIsland.current ?: return this
    val key = androidx.compose.runtime.remember { Any() }
    androidx.compose.runtime.DisposableEffect(island) { onDispose { island.covers.remove(key) } }
    return onGloballyPositioned { c ->
        val b = c.boundsInWindow()
        val r = android.graphics.Rect(b.left.toInt(), b.top.toInt(), kotlin.math.ceil(b.right).toInt(), kotlin.math.ceil(b.bottom).toInt())
        if (island.covers[key] != r) island.covers[key] = r
    }
}

/** The bars' view over the page, or null where there is none (previews). */
val LocalBarsIsland = androidx.compose.runtime.staticCompositionLocalOf<BarsIsland?> { null }
