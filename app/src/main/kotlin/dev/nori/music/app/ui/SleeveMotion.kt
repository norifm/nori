package dev.nori.music.app.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.nori.music.app.vm.PlayerViewModel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * A plugin's looping sleeve on screen. Made once a song has had one: before, there is no player, no
 * surface and no touch watcher.
 *
 * Under the live video lies a still of the same picture, so nothing visibly changes when the video comes
 * or goes: the loop's poster (its first frame, from the plugin) before it has played for this song, then
 * the last frame it showed. Only one such frame is kept, and it is let go when the song changes.
 */
@Stable
internal class SleeveMotion(
    /** Hands the video surface to the player as it arrives on screen (true) and as it leaves. */
    val onView: (android.view.TextureView, Boolean) -> Unit,
) {
    /** How much of the live video is drawn. Read in the draw phase. */
    val alpha = Animatable(0f)

    /** How much of the still under it (the poster or the last frame) is drawn over nori's cover. */
    val under = Animatable(0f)

    /** The last frame the video showed, of loop [frameOf]; drawn in place of the poster. */
    var frame by mutableStateOf<ImageBitmap?>(null)
    var frameOf: String? = null

    /** The loop's first frame from its plugin, if it gives one. */
    var poster by mutableStateOf<CoverImage?>(null)

    /** The video's surface, while it is on screen. */
    var view: android.view.TextureView? = null

    /** The loop whose layer is on screen; the stills and fades belong to it. */
    var shown: String? = null

    /** The layer is on screen: this song has a loop, or one is still fading out. */
    var present by mutableStateOf(false)

    /** What the player was last asked to play. */
    var playing: String? = null

    /** Written by the carousel: no record held, lifted, sliding or waiting to land. */
    var resting by mutableStateOf(true)

    /** A finger is on the player. Whatever it does may move the record, so the video steps back first. */
    var touching by mutableStateOf(false)

    /** Whether the sleeve was still (all but the finger) the last time the director looked. */
    var wasCalm = false

    /** When it last became still (uptimeMillis). */
    var calmAt = 0L

    /**
     * The video steps back without a visible change: its frame now is kept and drawn under it, fully, and
     * the video goes. The frame is the sleeve's own size, one at a time.
     */
    suspend fun stepBack() {
        if (alpha.value > 0f) {
            view?.bitmap?.let { frame = it.asImageBitmap(); frameOf = shown }
            under.snapTo(1f)
            alpha.snapTo(0f)
        }
    }
}

/** The loop leaving with its song: quick, since the record is moving on. */
private const val MOTION_OUT_MS = 160

/** The cover becoming the poster, and the video coming in over a still of itself. */
private const val MOTION_IN_MS = 480

/**
 * How long the sleeve is still after a move before the video comes back, so it does not start in the
 * same breath as the move ends. Not after a tap that moved nothing: the video comes straight back.
 */
private const val MOTION_SETTLE_MS = 350L

/**
 * When the song's [loop] plays, and its fades. It plays only while all of this holds: the player is on
 * screen, fully open, with the artwork panel showing and no flight under way; the app is in front and
 * the screen is on; the record is at rest and no finger is on the player; and movement is not reduced.
 *
 * Nothing appears in one frame. The cover crossfades to the loop's poster as soon as it is loaded; the video,
 * which starts where it last stopped (MotionPlayer keeps that), fades in only once its first frame is on
 * the surface and over a still of the same picture. When any of the above stops holding, the video steps
 * back onto its own last frame ([SleeveMotion.stepBack]); put away, left, or with the screen off, the
 * player is also released outright, and nothing goes on decoding. A new song fades the last one's stills
 * out and lets its frame go.
 */
@Composable
internal fun MotionDirector(vm: PlayerViewModel, motion: SleeveMotion, loop: String?, sheet: PlayerSheet, onArtwork: Boolean) {
    val poster = motion.poster
    val shown = LocalPlayerShown.current
    var resumed by remember { mutableStateOf(false) }
    LifecycleResumeEffect(Unit) { resumed = true; onPauseOrDispose { resumed = false } }
    val live = shown && resumed
    val open by remember(sheet) { derivedStateOf { sheet.progress.value >= 1f && sheet.isOpen } }
    val still = reduceMotion()
    // Everything but the finger: a tap that moved nothing does not count as a move ending.
    val calm = live && open && onArtwork && !still && motion.resting
    val want = calm && !motion.touching
    // The music paused, the picture goes on moving (the owner's call, 2026-09-26). It still stops with
    // the screen, the app or the player put away, and the screen's own timeout bounds a player left open.
    DisposableEffect(motion) { onDispose { vm.motionRelease() } }
    LaunchedEffect(motion, loop, want, live, calm, still, poster) {
        if (calm && !motion.wasCalm) motion.calmAt = android.os.SystemClock.uptimeMillis()
        motion.wasCalm = calm
        if (loop != motion.shown) {
            // Another song: its stills and video fade out together, and the last frame is let go.
            if (motion.alpha.value > 0f || motion.under.value > 0f) coroutineScope {
                launch { motion.alpha.animateTo(0f, tween(MOTION_OUT_MS)) }
                launch { motion.under.animateTo(0f, tween(MOTION_OUT_MS)) }
            }
            vm.motionPause()
            motion.frame = null
            motion.frameOf = null
            motion.playing = null
            motion.present = false
            motion.shown = loop
        }
        if (loop == null || still) return@LaunchedEffect
        motion.present = true
        if (!live) {
            motion.stepBack()
            vm.motionRelease()
            motion.playing = null
            return@LaunchedEffect
        }
        if (!want) {
            motion.stepBack()
            vm.motionPause()
        }
        // Before the loop has played for this song, its poster takes the cover's place once it is here.
        if (motion.frame == null && poster != null && motion.under.value < 1f) {
            snapshotFlow { poster.state }.first { it != CoverImage.LOADING }
            if (poster.state == CoverImage.READY) motion.under.animateTo(1f, tween(MOTION_IN_MS))
        }
        if (!want) return@LaunchedEffect
        val settle = MOTION_SETTLE_MS - (android.os.SystemClock.uptimeMillis() - motion.calmAt)
        if (settle > 0) delay(settle)
        vm.motionPlay(loop)
        motion.playing = loop
        vm.motionReady.first { it == loop }
        motion.alpha.animateTo(1f, tween(MOTION_IN_MS))
    }
}

/**
 * The loop's layer: the still under it (its last frame, else the poster) and the video's surface. A
 * TextureView rather than a SurfaceView: the sleeve's soft bottom is a blend in an offscreen layer
 * (SoftSleeve), and only a texture is drawn into that layer like the rest of the picture; a SurfaceView
 * would punch a hole through the page with a hard bottom edge.
 */
@Composable
internal fun MotionCover(motion: SleeveMotion) {
    val context = LocalContext.current
    val view = remember(context) { android.view.TextureView(context).apply { isOpaque = false } }
    DisposableEffect(view) {
        motion.view = view
        motion.onView(view, true)
        onDispose {
            if (motion.view === view) motion.view = null
            motion.onView(view, false)
        }
    }
    Box(Modifier.fillMaxSize()) {
        MotionStill(motion) { 1f }
        AndroidView(
            factory = {
                (view.parent as? android.view.ViewGroup)?.removeView(view)
                view
            },
            modifier = Modifier.fillMaxSize().graphicsLayer { alpha = motion.alpha.value },
        )
    }
}

/**
 * The loop's still (its last frame, else its poster) as the sleeve shows it under the video, at [reach]
 * of its own fade: the flights draw it too, so the cover in flight lands on the picture the sleeve has.
 */
@Composable
internal fun MotionStill(motion: SleeveMotion, reach: () -> Float) {
    androidx.compose.foundation.layout.Spacer(
        Modifier.fillMaxSize().drawBehind {
            val still = motion.frame?.takeIf { motion.frameOf == motion.shown } ?: motion.poster?.image
            if (still != null) drawCover(still, motion.under.value * reach())
        },
    )
}

/**
 * [SleeveMotion.touching] from a finger's first touch on the player until the last one leaves. It only
 * watches, in the first pass, and consumes nothing.
 */
internal fun Modifier.watchTouches(motion: SleeveMotion): Modifier = pointerInput(motion) {
    awaitEachGesture {
        awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
        motion.touching = true
        try {
            while (true) {
                val event = awaitPointerEvent(PointerEventPass.Initial)
                if (event.changes.none { it.pressed }) break
            }
        } finally {
            motion.touching = false
        }
    }
}
