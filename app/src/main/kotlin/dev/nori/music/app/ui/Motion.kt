package dev.nori.music.app.ui

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.VisibilityThreshold
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * How everything in the app moves, and how fast. Every animation takes its spec from here:
 *
 * - [move]: a move from rest - the player opened by a tap, a panel changed. One
 *   curve, most of the way early and a long soft landing; [moveFrom] is the same curve shortened to start at
 *   the speed a finger let it go at.
 * - [page]: a change of page, both pages shifting a little and cross-fading.
 * - [settle]: something small a finger let go (a row, a record, a thumb), on a spring from the finger's speed.
 * - [leave]: something a finger sent away (a row swiped off) leaving the screen, quick and speeding up.
 * - [fadeIn] / [fadeOut]: something appearing or going where nothing moves, going quicker than it came.
 *
 * The lengths and the curve are the core's ([stage]), shared by every client. The user's Motion setting is
 * applied here too: Reduced keeps the fades and makes every move direct; E-ink makes everything direct.
 *
 * It is also how fast every animation runs at all. Compose scales each one by whatever MotionDurationScale is
 * in its coroutine's context, by default Android's animator duration scale - so with system animations off,
 * every tween and spring in the app finished on its first frame. The activity starts the whole composition
 * with this object in its context instead (see MainActivity), so every animation there is, ours and the
 * libraries', runs at full speed unless E-ink stills it.
 */
object AppMotion : androidx.compose.ui.MotionDurationScale {
    /** Movement made direct (Motion Reduced or E-ink). Kept current by App; read in composition, drawing or anywhere. */
    var reduce by mutableStateOf(false)
    /** The E-ink motion: every animation ends on its first frame. Kept current by App. */
    var still by mutableStateOf(false)
    override val scaleFactor: Float get() = if (still) 0f else 1f

    private val ease = stage.moveEase
    val Ease = CubicBezierEasing(ease[0], ease[1], ease[2], ease[3])

    /** How fast a move starts, in its own length per its own time: the curve's first slope. */
    private val startSlope = ease[1] / ease[0]

    /** The shortest a move let go by a finger is cut to, ms: a flick still reads as travel, not a jump. */
    private const val SHORTEST_MS = 120

    fun <T> move(): FiniteAnimationSpec<T> = if (reduce) snap() else tween(stage.moveMs, easing = Ease)

    /**
     * A move over [distance] px let go at [speed] px/s along it: the [move] curve, cut short so that it starts
     * at the finger's speed, no faster and no slower - a flick goes at once, a slow release takes the whole move.
     */
    fun <T> moveFrom(distance: Float, speed: Float): FiniteAnimationSpec<T> {
        if (reduce) return snap()
        val ms = if (speed <= 0f) stage.moveMs else (startSlope * distance / speed * 1000f).toInt().coerceIn(SHORTEST_MS, stage.moveMs)
        return tween(ms, easing = Ease)
    }

    private val pageEase = stage.pageEase.let { CubicBezierEasing(it[0], it[1], it[2], it[3]) }

    /** A change of page, or the [left] share of one a finger let go. */
    fun <T> page(left: Float = 1f): FiniteAnimationSpec<T> =
        if (reduce) snap() else tween((stage.pageMs * left).toInt(), easing = pageEase)

    fun <T> settle(visibilityThreshold: T? = null): FiniteAnimationSpec<T> =
        if (reduce) snap() else spring(dampingRatio = 1f, stiffness = stage.settleStiffness, visibilityThreshold = visibilityThreshold)

    /** Something arriving that moves as it fades in (a dialog, a bar): a move, or with movement made direct a fade. */
    fun <T> show(): FiniteAnimationSpec<T> = if (reduce) fadeIn() else move()

    fun <T> leave(): FiniteAnimationSpec<T> = if (reduce) snap() else tween(stage.fadeOutMs, easing = FastOutLinearInEasing)

    fun <T> fadeIn(delayMs: Int = 0): FiniteAnimationSpec<T> = tween(stage.fadeMs, delayMs, LinearOutSlowInEasing)

    fun <T> fadeOut(): FiniteAnimationSpec<T> = tween(stage.fadeOutMs, easing = FastOutLinearInEasing)

    /** Something appearing where it stands. */
    fun appear(): androidx.compose.animation.EnterTransition = androidx.compose.animation.fadeIn(fadeIn())
    fun vanish(): androidx.compose.animation.ExitTransition = androidx.compose.animation.fadeOut(fadeOut())

    /** Something appearing in room it opens for itself, below it or, [across], beside it: the room moves, the thing fades in. */
    fun grow(across: Boolean = false): androidx.compose.animation.EnterTransition = appear() +
        if (across) androidx.compose.animation.expandHorizontally(move()) else androidx.compose.animation.expandVertically(move())

    /** Something going, and the room it took closing behind it. */
    fun shrink(across: Boolean = false): androidx.compose.animation.ExitTransition = vanish() +
        if (across) androidx.compose.animation.shrinkHorizontally(move()) else androidx.compose.animation.shrinkVertically(move())
}

/** A lazy list's item moving to its place, coming and going, by [AppMotion]'s rules. */
fun androidx.compose.foundation.lazy.LazyItemScope.itemMotion(): androidx.compose.ui.Modifier = androidx.compose.ui.Modifier.animateItem(
    fadeInSpec = AppMotion.fadeIn(),
    placementSpec = AppMotion.settle(androidx.compose.ui.unit.IntOffset.VisibilityThreshold),
    fadeOutSpec = AppMotion.fadeOut(),
)
