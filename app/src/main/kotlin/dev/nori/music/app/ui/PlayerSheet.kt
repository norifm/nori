package dev.nori.music.app.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitHorizontalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.awaitVerticalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.foundation.gestures.verticalDrag
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.unit.IntSize
import androidx.lifecycle.compose.LifecycleResumeEffect
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * The full player as a sheet over the app rather than a page navigated to. It is one number,
 * [progress], from 0 (only the mini player showing) to 1 (the player filling the screen), and every
 * part of the transition is drawn from it: the sheet's leading edge travels from the mini player's inner
 * edge to the far side of the screen (up from the bottom, or across from the side the mini player stands
 * on, [edge]), and the cover grows from the mini player's thumbnail into the sleeve. A drag
 * sets the number directly, so the whole thing follows the finger and holds wherever it is held; a
 * release carries on at the finger's own speed (AppMotion.moveFrom), and a tap moves it as a page moves
 * (AppMotion.move). Closing is the same path backwards.
 *
 * Before this the player was a route: a drag on the mini player had to cross a threshold, and then a
 * separate slide played on its own - you pulled, and then something else happened.
 *
 * [startOpen]: put back together open (the app recreated on the player), open from its first frame.
 */
@Stable
class PlayerSheet(private val scope: CoroutineScope, startOpen: Boolean = false) {
    val progress = Animatable(if (startOpen) 1f else 0f).also { it.updateBounds(0f, 1f) }

    /** The mini player's artwork, in root coordinates: where the cover flies from and back to. */
    var miniCover by mutableStateOf(Rect.Zero)

    /**
     * The cover a panel of its own shows - the lyrics header's thumbnail - in the sheet's coordinates,
     * or [Rect.Zero] when the panel on screen has no cover. What the cover flies to and from when the
     * player is put away from that panel, in place of the sleeve.
     */
    var panelCover by mutableStateOf(Rect.Zero)

    /**
     * One cover is travelling between the sleeve and a panel's own thumbnail right now, so neither end
     * draws its own copy: the flight is the only one on screen. See PanelFlight in PlayerScreen.
     */
    var panelFlight by mutableStateOf(false)

    /** The panel the player opens on (the lyrics widget's tap, a jam strip's); the player takes it and clears it. */
    var panelAsked by mutableStateOf<Panel?>(null)

    /** The edge of the window the mini player stands on, which the sheet comes in from and goes back to. */
    var edge by mutableStateOf(SheetEdge.BOTTOM)

    /**
     * The mini player's side facing the page, in root coordinates along [edge]'s way: its top, or its inner
     * side when it stands on a side. Where the sheet's leading edge starts; 0 with no mini player.
     */
    var miniEdge by mutableFloatStateOf(0f)

    /** The window's size, for when there is no mini player to start from. */
    var rootSize by mutableStateOf(IntSize.Zero)

    /**
     * Open, or on the way there. What the back button, the test bridge and the chrome go by.
     *
     * It is the last thing the sheet was *asked* to do, not where it happens to be: a drag or a back
     * gesture moves [progress] (and with it the Animatable's target) without meaning anything by it,
     * and reading that turned the sheet "closed" on the first pixel of a gesture - which switched the
     * back handler off underneath the finger, so the player never sank and then vanished in one frame
     * instead of settling back onto the now playing bar.
     */
    var isOpen by mutableStateOf(startOpen)
        private set

    /** How far the sheet's leading edge travels, in pixels. */
    val travel: Float get() = when (edge) {
        SheetEdge.BOTTOM -> if (miniEdge > 0f) miniEdge else rootSize.height.toFloat()
        SheetEdge.RIGHT -> if (miniEdge > 0f) miniEdge else rootSize.width.toFloat()
        SheetEdge.LEFT -> rootSize.width - miniEdge
    }.coerceAtLeast(1f)

    /** Where the sheet sits put away, relative to filling the window. */
    fun closed(): Offset = when (edge) {
        SheetEdge.BOTTOM -> Offset(0f, travel)
        SheetEdge.RIGHT -> Offset(travel, 0f)
        SheetEdge.LEFT -> Offset(-travel, 0f)
    }

    /** Where the sheet is now, relative to filling the window. */
    fun offset(): Offset = closed() * (1f - progress.value)

    fun open() {
        settle(1f, 0f)
    }

    /**
     * The back gesture at [progress] (0..1): the sheet sinks with it, a fifth of the way at most - a hint
     * of where it is going, not the whole trip, which is the release's to make.
     */
    suspend fun backBy(progress: Float) {
        val eased = 1f - (1f - progress.coerceIn(0f, 1f)).let { it * it }
        this.progress.snapTo(1f - BACK_TRAVEL * eased)
    }

    fun close() {
        settle(0f, 0f)
    }

    /** Which end the sheet was nearer when the finger went down. */
    private var from = 0f

    /** A drag begins. */
    fun dragStart() {
        from = if (progress.value > 0.5f) 1f else 0f
    }

    /** Moves the sheet with a finger that moved [d] pixels its way, towards putting it away. */
    fun dragBy(d: Float) {
        scope.launch { progress.snapTo((progress.value - d / travel).coerceIn(0f, 1f)) }
    }

    /** The finger let go at [velocity] pixels a second, towards putting it away: it settles where [sheetTarget] says. */
    fun release(velocity: Float) {
        val target = sheetTarget(from, progress.value, velocity)
        settle(target, (if (target == 0f) velocity else -velocity) / travel)
    }

    /** Goes to [target] (1 open, 0 put away), carrying on at [speed], in sheets a second towards it; 0 from rest. */
    private fun settle(target: Float, speed: Float) {
        isOpen = target == 1f
        scope.launch {
            val distance = kotlin.math.abs(target - progress.value)
            progress.animateTo(target, if (speed > 0f) AppMotion.moveFrom(distance, speed) else AppMotion.move())
        }
    }

}

/** Where the mini player stands: along the bottom, or on its side down the right or the left edge. */
enum class SheetEdge { BOTTOM, RIGHT, LEFT }

val LocalPlayerSheet = staticCompositionLocalOf<PlayerSheet> { error("no player sheet") }

/**
 * Whether the player is on screen at all. It stays composed while it is put away, so that opening it
 * only has to move it - building it from nothing on the first frame of the drag stalled that frame -
 * and everything in it that ticks, or reaches outside the player, checks this first.
 */
val LocalPlayerShown = androidx.compose.runtime.compositionLocalOf { true }

/** Whether the open player covers the pages under it completely. They stay composed and resumed. */
val LocalPageCovered = androidx.compose.runtime.compositionLocalOf { false }

/**
 * Whether what this composable draws can be seen: its screen resumed, the player not over the page and,
 * inside the player, the player on screen. Anything that ticks runs only while this holds.
 */
@Composable
fun seen(): Boolean {
    var resumed by remember { mutableStateOf(false) }
    LifecycleResumeEffect(Unit) { resumed = true; onPauseOrDispose { resumed = false } }
    return resumed && !LocalPageCovered.current && LocalPlayerShown.current
}

/**
 * Drags along the sheet's way move it: up and down when it rises from the bottom, across towards the edge
 * the mini player stands on when it comes in from the side. The mini player itself is turned with its pill
 * ([onMini]), so its own vertical is the sheet's way either way. A drag the other way is left to whatever
 * else wants it (the songs either side, scrolling). The velocity is taken from the summed deltas, not from
 * the pointer's position: inside the sheet the element moves with the finger, so its local position barely
 * changes and would report a flick as standing still.
 */
internal fun Modifier.dragsSheet(sheet: PlayerSheet, enabled: Boolean = true, onMini: Boolean = false): Modifier {
    if (!enabled) return this
    val across = !onMini && sheet.edge != SheetEdge.BOTTOM
    // Towards putting it away is positive: down, or out to the edge the mini player stands on.
    val sign = if (across && sheet.edge == SheetEdge.LEFT) -1f else 1f
    return pointerInput(sheet, across, sign) {
        val tracker = VelocityTracker()
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            // Only a drag the sheet can go: from open, towards putting it away; from put away, towards opening.
            // A drag the other way stays a tap on whatever is under the finger, so a button pressed with a
            // finger that slips up still presses.
            val at = sheet.progress.value
            fun goes(d: Float) = (at < 1f || sign * d > 0f) && (at > 0f || sign * d < 0f)
            var slop = 0f
            val claim = { change: androidx.compose.ui.input.pointer.PointerInputChange, over: Float -> if (goes(over)) { slop = over; change.consume() } }
            val first = (if (across) awaitHorizontalTouchSlopOrCancellation(down.id, claim) else awaitVerticalTouchSlopOrCancellation(down.id, claim))
                ?: return@awaitEachGesture
            tracker.resetTracking()
            var moved = 0f
            sheet.dragStart()
            fun move(change: androidx.compose.ui.input.pointer.PointerInputChange, d: Float) {
                moved += d
                tracker.addPosition(change.uptimeMillis, if (across) Offset(moved, 0f) else Offset(0f, moved))
                sheet.dragBy(sign * d)
                change.consume()
            }
            val step = { change: androidx.compose.ui.input.pointer.PointerInputChange -> move(change, change.positionChange().let { if (across) it.x else it.y }) }
            // Past the slop, only what went beyond it moves the sheet, as the stock drag detectors do.
            move(first, slop)
            val ended = if (across) horizontalDrag(first.id, step) else verticalDrag(first.id, step)
            if (ended) sheet.release(sign * tracker.calculateVelocity().let { if (across) it.x else it.y }) else sheet.release(0f)
        }
    }
}

/** How far the back gesture takes the player down before it is let go. */
private const val BACK_TRAVEL = 0.2f

/** The player sheet: a release faster than this, in pixels a second, goes the way it was flicked... */
private const val SHEET_FLICK_PX_S = 900f
/** ...and a slow drag that has come this share of the way finishes the move it started. */
private const val SHEET_COMMIT = 0.15f

/**
 * Where the player sheet settles when the finger lifts: 1 open, 0 put away. [from] is the end it was
 * nearer when the drag began, [progress] where it is now and [velocity] the finger's, positive towards
 * putting it away. A flick goes the way it was flicked; a drag that has come a little way finishes the move
 * it started, and a smaller one goes back. Deciding by the halfway point instead meant a pull down from the
 * player had to cover half the screen before it would close.
 */
private fun sheetTarget(from: Float, progress: Float, velocity: Float): Float {
    val moved = progress - from
    return when {
        velocity < -SHEET_FLICK_PX_S -> 1f
        velocity > SHEET_FLICK_PX_S -> 0f
        moved > SHEET_COMMIT -> 1f
        moved < -SHEET_COMMIT -> 0f
        else -> from
    }
}
