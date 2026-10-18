package dev.nori.music.app.ui

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.drag
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.VelocityTracker1D
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.navigation.NavHostController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlin.math.abs

/**
 * Going back the way iOS and Telegram do it: the page on top follows the finger exactly and the page it
 * covers comes back from a third of the way left, out of the dark; let go, the page carries on at the
 * finger's own speed - a flick is gone at once - or springs back if the finger was not going anywhere.
 *
 * Android's back gesture alone cannot do this: its progress trails the finger (the system smooths it by
 * about 130 ms) and it has no speed, so every version of the back motion driven by it either lagged or
 * finished at the same pace however it was flicked. Here a swipe on the page drives the motion from the
 * touches themselves, the edge gesture from its raw finger position, and the speed is measured from them.
 *
 * The page underneath is not composed while another covers it, so what shows under the swipe is a
 * picture of it taken as it was covered ([keep]); when the motion ends, the stack is popped with no
 * transition and the page itself takes the picture's place, drawn the same. The last [KEPT] pictures are
 * kept, one a page deep; a page deeper than that goes back with the plain transition (PageMotion).
 */
@Stable
class PageSwipe(private val controller: NavHostController, private val scope: CoroutineScope) {
    /** The page on top's offset, px: right of its place from the left edge, left of it from the right. */
    var offset by mutableFloatStateOf(0f)
        private set
    /** The width the pages fill, px. */
    var width by mutableFloatStateOf(0f)
    /** The picture of the page coming back, while a swipe shows it; null otherwise. */
    var under: ImageBitmap? by mutableStateOf(null)
        private set
    /** The id of the page the motion is taking away, which wears [offset]; null at rest. */
    var moving: String? by mutableStateOf(null)
        private set

    private val pictures = object : LinkedHashMap<String, ImageBitmap>() {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ImageBitmap>?) = size > KEPT
    }

    /** Changes with [pictures], so whatever asks [canGoBack] in composition asks again when one arrives. */
    private var kept by mutableIntStateOf(0)

    /** Keeps a picture of the page [id] as a page opened over it covers it. */
    fun keep(id: String, picture: ImageBitmap) { pictures[id] = picture; kept++ }

    /**
     * Whether the page on top can be taken back: the one under it was pictured, and - unless it is the
     * system's [edge] gesture or the back key - it is not a tab's own page. A swipe on a tab's page does not
     * leave the tab, as on iOS; Android's back from it goes to Home, and goes the same way as any other back.
     */
    fun canGoBack(edge: Boolean = false): Boolean {
        kept
        val top = controller.currentBackStackEntry ?: return false
        val back = controller.previousBackStackEntry ?: return false
        return (edge || top.destination.route !in ROOTS) && back.id in pictures
    }

    /** Starts a motion back, showing the page underneath; false when there is none to show. */
    fun begin(edge: Boolean = false): Boolean {
        if (moving != null || width <= 0f || !canGoBack(edge)) return false
        under = pictures[controller.previousBackStackEntry!!.id]
        moving = controller.currentBackStackEntry!!.id
        plain()
        return true
    }

    /** Puts the page on top at [x] px, the finger's place. */
    fun dragTo(x: Float) { offset = x }

    /**
     * The finger let go at [velocity] px/s along the swipe. [commit] says whether to go back: when null,
     * the motion decides - a flick, or past [COMMIT] of the width without the finger turning back.
     * [flung]: the system decided it was a back from a gesture, which goes at least at a flick's speed -
     * a quick edge flick can end on one or two finger positions, too few to measure a speed from, and it
     * then went with the button's slow ease.
     */
    fun release(velocity: Float, commit: Boolean? = null, flung: Boolean = false) {
        val side = if (offset < 0f) -1f else 1f
        val along = if (flung) maxOf(velocity * side, FLICK * 2f) else velocity * side
        val speed = along * side
        val back = commit ?: (along > FLICK || abs(offset) > width * COMMIT && along > -FLICK)
        scope.launch {
            when {
                // Going back from a gesture keeps the finger's momentum: straight on at its speed (a brisk
                // walk at least) until the page is off the screen. A spring from that speed slowed the page
                // down as it left - a flick that visibly braked.
                back && abs(speed) > STILL -> {
                    val pace = maxOf(abs(speed), MIN_PACE)
                    val ms = ((width - abs(offset)) / pace * 1000f).toInt().coerceIn(MIN_MS, SETTLE_MS)
                    animate(offset, side * width, speed, tween(ms, easing = androidx.compose.animation.core.LinearEasing)) { v, _ -> offset = v }
                }
                // The back button or key, with no speed: the push's own ease, in reverse.
                back -> animate(offset, side * width, 0f, tween(SETTLE_MS, easing = PageMotion.Settle)) { v, _ -> offset = v }
                // Called off: springs back from the finger's speed.
                else -> animate(offset, 0f, speed, spring(dampingRatio = 1f, stiffness = STIFFNESS)) { v, _ ->
                    offset = if (side > 0f) v.coerceAtLeast(0f) else v.coerceAtMost(0f)
                }
            }
            if (back) finish() else rest()
        }
    }

    /** Whether a motion back is under way. */
    val busy: Boolean get() = moving != null

    /** Backs asked for while another is still under way: each runs as soon as the one before it is done. */
    private var queued = 0

    /**
     * The back button or key: the same motion, from rest. Pops plainly when the page underneath was not
     * pictured; asked again while a back is under way, it waits for it - popped at once, the stack lost the
     * page the motion was still taking away, and the motion then popped one page too many.
     */
    fun back() {
        when {
            busy -> queued++
            begin(edge = true) -> release(0f, commit = true)
            else -> { plain(); controller.popBackStack() }
        }
    }

    private fun finish() {
        // The motion has already shown the change; the stack follows with none of its own, until anything
        // next moves the stack ([plain]). The page taken away keeps its offset, off the screen, until it
        // leaves the composition ([left]). Ended a fixed two frames after the pop, a phone slower than that
        // to build the page come back would see the stack run its own pop over it, or the page taken away
        // drawn whole again over it.
        PageMotion.instant = true
        controller.popBackStack()
    }

    /** The page [id] has left the composition: if a swipe took it away, the swipe is over. */
    fun left(id: String) { if (moving == id) rest() }

    /** Anything else is moving the stack: it does so with its own motion again. */
    fun plain() { PageMotion.instant = false }

    private fun rest() {
        offset = 0f
        under = null
        moving = null
        if (queued > 0) {
            queued--
            scope.launch { back() }
        }
    }

    companion object {
        private const val KEPT = 4
        /** A flick, px/s: faster than this along the swipe goes back however short it was. */
        private const val FLICK = 1000f
        /** Past this share of the width a slow swipe goes back. */
        private const val COMMIT = 0.35f
        /** Slower than this, px/s, the finger had stopped: the page eases rather than springs. */
        private const val STILL = 50f
        /** The back button's and key's motion, from rest: the push's own length. */
        private const val SETTLE_MS = 200
        /** The spring a page called back goes home on, from the finger's speed: critically damped, about 170 ms. */
        private const val STIFFNESS = 800f
        /** The slowest a page going back from a gesture leaves at, px/s, and the shortest it takes. */
        private const val MIN_PACE = 2500f
        private const val MIN_MS = 60
        /** How far the page underneath starts, as a share of the width, and how dark. */
        const val UNDER = 3
        const val SCRIM = 0.28f
        val ROOTS = setOf("home", "search", "library", "settings")
    }
}

/** The swipe right on a page that takes it back; a child that takes the drag (a row of covers, a slider) keeps it. */
fun Modifier.swipeBack(swipe: PageSwipe, enabled: Boolean): Modifier = if (!enabled) this else pointerInput(swipe) {
    val slop = viewConfiguration.touchSlop
    awaitEachGesture {
        val down = awaitFirstDown(requireUnconsumed = false)
        var dx = 0f
        var dy = 0f
        // Only a drag that is plainly sideways and to the right, and that nothing under the finger took.
        while (true) {
            val event = awaitPointerEvent()
            val change = event.changes.firstOrNull { it.id == down.id } ?: return@awaitEachGesture
            if (!change.pressed || change.isConsumed) return@awaitEachGesture
            val d = change.positionChange()
            dx += d.x
            dy += d.y
            if (abs(dy) > slop && abs(dy) >= abs(dx)) return@awaitEachGesture
            if (dx > slop && dx > abs(dy) * 1.5f) {
                if (!swipe.begin()) return@awaitEachGesture
                change.consume()
                break
            }
        }
        val tracker = VelocityTracker()
        var x = 0f
        val done = drag(down.id) { change ->
            x = (x + change.positionChange().x).coerceAtLeast(0f)
            swipe.dragTo(x)
            tracker.addPointerInputChange(change)
            change.consume()
        }
        swipe.release(if (done) tracker.calculateVelocity().x else 0f, commit = if (done) null else false)
    }
}

/**
 * Measures the edge gesture's speed from its raw finger positions: [add] each one, then [velocity], px/s,
 * along the offset (the offset follows the finger one to one).
 */
class EdgeSpeed {
    private val tracker = VelocityTracker1D(isDataDifferential = false)
    fun add(x: Float) = tracker.addDataPoint(android.os.SystemClock.uptimeMillis(), x)
    fun velocity(): Float = tracker.calculateVelocity()
}

/** The page coming back, under the one being taken away: the picture of it, a third of the way out and darkened, brightening as it comes. */
fun DrawScope.drawUnder(swipe: PageSwipe, background: Color) {
    val picture = swipe.under ?: return
    val w = swipe.width.takeIf { it > 0f } ?: return
    val shown = (abs(swipe.offset) / w).coerceIn(0f, 1f)
    val side = if (swipe.offset < 0f) -1f else 1f
    translate(left = -side * (w / PageSwipe.UNDER) * (1f - shown)) {
        drawRect(background)
        drawImage(picture)
        drawRect(Color.Black.copy(alpha = PageSwipe.SCRIM * (1f - shown)))
    }
}

/** Kept here so a page can reach the app's one [PageSwipe]. */
val LocalPageSwipe = androidx.compose.runtime.staticCompositionLocalOf<PageSwipe?> { null }

@Composable
internal fun rememberPageSwipe(controller: NavHostController): PageSwipe {
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    return androidx.compose.runtime.remember(controller) { PageSwipe(controller, scope) }
}
