package dev.nori.music.app.ui

import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onPlaced
import dev.nori.music.app.R

import androidx.compose.runtime.mutableStateOf
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.animation.togetherWith
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import dev.nori.music.ffi.settings.SwipeAction
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.isActive
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.Load
import dev.nori.music.ffi.model.Album
import dev.nori.music.ffi.model.Song
import dev.nori.music.look.CoverLook
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.drawscope.clipRect
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/**
 * A cover of an octo-fiesta provider item (external song, album, artist or playlist), which is never
 * kept on disk (`Covers.isProvider`).
 */
fun isProviderCover(url: String): Boolean = dev.nori.music.data.Covers.isProvider(url)

/**
 * The sizes covers are drawn at, nori-core's (`cover_rules`): two, not four, because a Subsonic server
 * renders each size it is asked for on demand. A list thumbnail and a grid card share one rendition, and
 * the full-screen artwork shares its rendition with the notification and the lock screen.
 */
object CoverSize {
    private val rules get() = dev.nori.music.data.Covers.rules
    val ROW: Int = rules.row.toInt()
    val CARD: Int = rules.card.toInt()
    val FULL: Int = rules.full.toInt()
}

/**
 * Artwork with the app's corner radius. The picture is asked for at the view's own size, decoded by the
 * core straight into a Bitmap that size (rememberCover), and let go if the view leaves first; the
 * rounded clip is a plain render-node clip, which the GPU does for free and which a grid of covers needs
 * to not look like a spreadsheet. Pass `radius = 0.dp` for the full-bleed artwork at the top of a page;
 * `size = 0.dp` sizes it by its layout, and it asks once it has been measured.
 *
 * Nothing about it appears in one frame. A picture that has to be fetched fades in over its plate
 * (one kept in memory is simply there - fading those in made every scroll shimmer); one that takes a
 * while shows a soft sheen crossing the plate, so a slow server reads as loading rather than as a
 * missing cover; and one that never comes settles into the plate's note glyph, faded in too.
 *
 * [plate] false draws nothing of its own - no plate, sheen or note - so the picture fades in over
 * whatever is behind it (a mix tile's colour), and a missing one simply leaves that showing.
 */
@Composable
fun Cover(url: String?, size: Dp, modifier: Modifier = Modifier, radius: Dp = Radius.cover, plate: Boolean = true, standIns: List<String?> = emptyList()) {
    val px = with(LocalDensity.current) { size.roundToPx() }
    val cover = rememberCover(url, px, px, standIns)
    // Sized by its layout: asked for from layout, where the size is known, rather than through a state
    // that would compose every such cover a second time. The last size is kept for the next address.
    val measured = remember { IntArray(2) }
    if (px == 0) androidx.compose.runtime.DisposableEffect(cover) {
        if (measured[0] > 0 && measured[1] > 0) cover.want(measured[0], measured[1])
        onDispose {}
    }
    val shape = remember(radius) { androidx.compose.foundation.shape.RoundedCornerShape(radius) }
    // A flat grey square is what makes a library of half-loaded covers look broken. Underneath every
    // cover sits a soft two-tone plate with a note on it, which is what shows while the picture loads
    // and what stays when a track simply has no artwork. One gradient in the page's look, drawn - and
    // read while drawing, so the player's page changing colour under a cover only redraws it.
    val look = LocalLook.current
    val loading = cover.state == CoverImage.LOADING
    val missing = cover.state == CoverImage.MISSING
    // The sheen outlives the load by the length of the picture's fade, so it goes away underneath a
    // picture that is already covering it instead of vanishing from on top of the plate.
    var sheen by remember(cover) { mutableStateOf(loading) }
    androidx.compose.runtime.LaunchedEffect(loading) { if (!loading) kotlinx.coroutines.delay(SHEEN_LEAVE_MS.toLong()); sheen = loading }
    // Read while drawing, so the fade redraws the picture and recomposes nothing.
    val fade = remember(cover) { androidx.compose.animation.core.Animatable(if (cover.image != null) 1f else 0f) }
    val here = cover.image != null
    androidx.compose.runtime.LaunchedEffect(cover, here) {
        if (!here || fade.value == 1f) return@LaunchedEffect
        fade.animateTo(1f, AppMotion.fadeIn())
    }
    Box(
        // Without a size it fills what it is given.
        (if (px > 0) modifier.size(size) else modifier.fillMaxSize().onSizeChanged {
            measured[0] = it.width; measured[1] = it.height
            if (it.width > 0 && it.height > 0) cover.want(it.width, it.height)
        })
            .then(if (radius > 0.dp) Modifier.clip(shape) else Modifier)
            .drawWithCache {
                val brush = if (plate) Brush.linearGradient(listOf(look.color(CoverLook.VEIL_13), look.color(CoverLook.VEIL_6))) else null
                onDrawWithContent {
                    brush?.let { drawRect(it) }
                    drawContent()
                    cover.image?.let { drawCover(it, fade.value) }
                }
            },
    ) {
        // A load that ends with no picture (offline, a failure) fades the sheen out as the note fades in.
        if (sheen && plate) Box(Modifier.matchParentSize().loadingSheen(true, leaving = !loading))
        androidx.compose.animation.AnimatedVisibility(
            missing && plate, Modifier.align(Alignment.Center),
            enter = androidx.compose.animation.fadeIn(AppMotion.fadeIn()), exit = androidx.compose.animation.fadeOut(AppMotion.fadeOut()),
        ) {
            LookIcon(
                Iconoir.MusicNote, null,
                Modifier.size(if (size > 0.dp) size * 0.34f else 40.dp),
            ) { look.color(CoverLook.ON_22) }
        }
    }
}

/**
 * "3:07", or "1:02:03" from an hour ([dev.nori.music.text.Fmt.duration]). Each second's text is made once
 * for the life of the process and kept: the seek bar asks for two of these every second a song plays, and
 * every list row for its song's length, so after the first time through they cost an array read.
 */
fun duration(seconds: Long): String = dev.nori.music.text.Fmt.duration(seconds)

/** The time left, "-3:07": kept the same way. */
fun durationLeft(seconds: Long): String = dev.nori.music.text.Fmt.durationLeft(seconds)

/** What a sideways drag on a song row does: what it uncovers under the row, and what letting go past [SWIPE_ARM] does. */
class RowSwipe(val icon: ImageVector, val label: String, val action: () -> Unit)

/** One per row: the Animatable the row slides on, and whether letting go now would act. */
@Stable
class SwipeState {
    /** Where the row is, written straight from the finger; see [swipeable]. */
    val offset = androidx.compose.runtime.mutableFloatStateOf(0f)
    var armed by mutableStateOf(false)
    /** The spring back, while it runs: a new drag takes the row from wherever it has got to. */
    var settling: kotlinx.coroutines.Job? = null
}

/** A head pinned over a list (an album's title and buttons): the rows under it are out of sight. */
class PinnedHead {
    var placed: androidx.compose.ui.layout.LayoutCoordinates? = null
    /** Its bottom edge, px in the window; 0 while it is not laid out. */
    fun bottom(): Float = placed?.takeIf { it.isAttached }?.let { it.boundsInWindow().bottom } ?: 0f
}

val LocalPinnedHead = androidx.compose.runtime.staticCompositionLocalOf<PinnedHead?> { null }

/** Where a row of a list is laid out, for whether enough of it is in sight to be touched. */
class RowPlace {
    var placed: androidx.compose.ui.layout.LayoutCoordinates? = null

    /** Half the row or more is in sight: inside the list that holds it, and below the [head] pinned over it. */
    fun inSight(head: PinnedHead?): Boolean {
        val at = placed?.takeIf { it.isAttached } ?: return false
        val shown = at.boundsInWindow()
        return shown.bottom - maxOf(shown.top, head?.bottom() ?: 0f) >= at.size.height / 2f
    }
}

/**
 * A row mostly scrolled under its list's edge or a pinned head: a finger landing on the sliver left does
 * nothing to it (no tap, hold, press or swipe), and the list still scrolls from there.
 */
internal fun Modifier.touchedInSight(place: RowPlace, head: PinnedHead?): Modifier =
    onPlaced { place.placed = it }.pointerInput(place, head) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = androidx.compose.ui.input.pointer.PointerEventPass.Initial)
            if (!place.inSight(head)) down.consume()
        }
    }

/** How far across the row a drag has to go before letting go acts: the share that turns a record. */
private const val SWIPE_ARM = TURN

/**
 * Sideways drag on a row. Only a direction with an action moves at all. Past [SWIPE_ARM] of the width
 * the row goes heavier (it follows at 40%), the strip underneath turns to ink and the phone
 * ticks, so the finger knows before it lifts; letting go then acts and the row springs back. The row
 * paints [fill] under itself only while it is off its place, so the strip never shows through it.
 */
internal fun Modifier.swipeable(
    s: SwipeState, right: RowSwipe?, left: RowSwipe?, fill: Color?,
    /** The action takes the row away (a song out of the queue): armed and let go, the row slides off first. */
    gone: Boolean = false,
    /** A side with no action still gives a little ([GIVE] of the finger, at most [GIVE_LIMIT] of the width) and comes back: "not this one". */
    resist: Boolean = false,
    /** Whether the row is in sight enough to be swiped ([RowPlace.inSight]). */
    shown: () -> Boolean = { true },
): Modifier = composed {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val back = AppMotion.settle<Float>()
    // The gesture outlives recompositions (it is keyed on which sides act, not on the actions), so it
    // reads the actions as they are now: a favourite swiped once must offer "Remove" the second time.
    val acts by rememberUpdatedState(right to left)
    val inSight by rememberUpdatedState(shown)
    pointerInput(right != null, left != null, resist) {
        var x = 0f
        // The finger's own travel, for a side that only gives.
        var travel = 0f
        fun settle() {
            s.armed = false
            s.settling = scope.launch { androidx.compose.animation.core.animate(s.offset.floatValue, 0f, animationSpec = back) { v, _ -> s.offset.floatValue = v } }
        }
        // A row that went but is still here long after (the change did not happen): it comes back rather
        // than leave a hole. A row that really went has left the composition by then, and this with it.
        suspend fun comeBackIfStill() {
            kotlinx.coroutines.delay(SWIPE_GONE_WAIT_MS)
            s.armed = false
            androidx.compose.animation.core.animate(s.offset.floatValue, 0f, animationSpec = back) { v, _ -> s.offset.floatValue = v }
        }
        sidewaysDrag(
            takes = { inSight() },
            onDragStart = { s.settling?.cancel(); x = s.offset.floatValue; travel = x },
            onDragEnd = {
                val act = if (s.armed) (if (x > 0f) acts.first else acts.second) else null
                when {
                    act == null -> settle()
                    gone -> s.settling = scope.launch {
                        // Off the side it was going, at the speed of a short slide, and only then acted on:
                        // the rows under it close up once the queue has changed.
                        val to = kotlin.math.sign(x) * size.width.toFloat()
                        androidx.compose.animation.core.animate(s.offset.floatValue, to, animationSpec = AppMotion.leave()) { v, _ -> s.offset.floatValue = v }
                        act.action()
                        comeBackIfStill()
                    }
                    else -> { act.action(); settle() }
                }
            },
            onDragCancel = { settle() },
        ) { change, delta ->
            val w = size.width.toFloat()
            val arm = w * SWIPE_ARM
            travel += delta
            change.consume()
            val acting = if (travel > 0f) right != null else left != null
            if (resist && !acting) {
                x = kotlin.math.sign(travel) * kotlin.math.min(kotlin.math.abs(travel) * GIVE, w * GIVE_LIMIT)
                s.offset.floatValue = x
                return@sidewaysDrag
            }
            val heavy = kotlin.math.abs(x) > arm && (delta > 0f) == (x > 0f)
            x = (x + if (heavy) delta * 0.4f else delta).coerceIn(if (left != null) -w * 0.6f else 0f, if (right != null) w * 0.6f else 0f)
            travel = x
            val armed = kotlin.math.abs(x) > arm
            if (armed != s.armed) { s.armed = armed; haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove) }
            // Written, not snapped to from a coroutine: one launch per pointer event was a job per frame.
            s.offset.floatValue = x
        }
    }
        .graphicsLayer { translationX = s.offset.floatValue }
        .then(if (fill != null) Modifier.drawBehind { if (s.offset.floatValue != 0f) drawRect(fill) } else Modifier)
}

/** How long a row taken away by its swipe takes to leave from where the finger let go. */
private const val SWIPE_GONE_WAIT_MS = 1_500L

/**
 * A drag taken only when it is plainly sideways: once the finger has gone [slop] times the touch slop,
 * it has to have moved at least [ratio] times as far across as down. Anything steeper, or anything
 * something else has already taken, is left alone, so a scroll that is a little off vertical scrolls
 * instead of swiping a song, and a diagonal pull upwards opens the player instead of changing it.
 * ([detectHorizontalDragGestures] claims a drag on the sideways distance alone, which a slanted drag
 * easily reaches first - which is what every one of those was.)
 *
 * Everywhere a sideways drag shares its space with an up-and-down one uses this: the rows of a list,
 * the now playing bar and the full-screen sleeve, so the three feel like one gesture.
 */
internal suspend fun androidx.compose.ui.input.pointer.PointerInputScope.sidewaysDrag(
    onDragStart: () -> Unit, onDragEnd: () -> Unit, onDragCancel: () -> Unit,
    slop: Float = 1f, ratio: Float = 2f,
    /** Asked as the finger lands: false leaves the gesture to whatever else wants it. */
    takes: () -> Boolean = { true },
    onDrag: (androidx.compose.ui.input.pointer.PointerInputChange, Float) -> Unit,
) = awaitEachGesture {
    val down = awaitFirstDown(requireUnconsumed = false)
    if (!takes()) return@awaitEachGesture
    val decide = viewConfiguration.touchSlop * slop
    var dx = 0f
    var dy = 0f
    while (true) {
        val c = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: return@awaitEachGesture
        if (!c.pressed || c.isConsumed) return@awaitEachGesture
        val d = c.positionChange()
        dx += d.x; dy += d.y
        if (dx * dx + dy * dy < decide * decide) continue
        if (kotlin.math.abs(dx) < ratio * kotlin.math.abs(dy)) return@awaitEachGesture
        c.consume()
        break
    }
    onDragStart()
    val finished = horizontalDrag(down.id) { onDrag(it, it.positionChange().x) }
    if (finished) onDragEnd() else onDragCancel()
}

/**
 * The strip a swiped row uncovers: the action's icon and words at the edge the row left, fading in over
 * the first 64 dp. Neutral until the drag is far enough to act, then in ink, with the icon giving
 * a small pop. Composed only while the row is off its place, so a list at rest carries none of it.
 */
@Composable
internal fun SwipeBackdrop(
    s: SwipeState, right: RowSwipe?, left: RowSwipe?, modifier: Modifier,
    /** Its colours: the theme's (a list on a page) unless given (the player's queue, on the cover's colours). */
    colours: SwipeColours? = null,
    /** Drawn only where the row has moved off, for a row with no fill of its own to cover the rest. */
    reveal: Boolean = false,
    /** The words' and icon's distance from the edge. */
    inset: Dp = Space.gutter,
) {
    val side by remember { derivedStateOf { kotlin.math.sign(s.offset.floatValue) } }
    if (side == 0f) return
    val face = (if (side > 0f) right else left) ?: return
    val scheme = MaterialTheme.colorScheme
    val c = colours ?: remember(scheme) {
        SwipeColours({ scheme.surfaceContainerHighest }, { scheme.onSurface }, { scheme.onSurfaceVariant }, { scheme.background })
    }
    val armed by animateFloatAsState(if (s.armed) 1f else 0f, AppMotion.fadeIn(), label = "swipe armed")
    val fill = { androidx.compose.ui.graphics.lerp(c.fill(), c.armedFill(), armed) }
    val ink = androidx.compose.ui.graphics.ColorProducer { androidx.compose.ui.graphics.lerp(c.ink(), c.armedInk(), armed) }
    val pop by animateFloatAsState(if (s.armed) 1.15f else 1f, spring(dampingRatio = 0.45f, stiffness = 700f), label = "swipe pop")
    Box(
        modifier.drawWithContent {
            val x = s.offset.floatValue
            // The strip the row has uncovered, or the whole row under a row that paints over the rest itself.
            val from = if (!reveal) 0f else if (x > 0f) 0f else size.width + x
            val to = if (!reveal) size.width else if (x > 0f) x else size.width
            if (to <= from) return@drawWithContent
            clipRect(left = from, right = to) {
                drawRect(fill())
                this@drawWithContent.drawContent()
            }
        },
    ) {
        Row(
            Modifier.align(if (side > 0f) Alignment.CenterStart else Alignment.CenterEnd).padding(horizontal = inset)
                .graphicsLayer { alpha = (kotlin.math.abs(s.offset.floatValue) / 64.dp.toPx()).coerceIn(0f, 1f) },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LookIcon(face.icon, null, Modifier.size(22.dp).graphicsLayer { scaleX = pop; scaleY = pop }, ink)
            LookText(face.label, ink, Modifier.padding(start = 10.dp), style = MaterialTheme.typography.labelLarge, maxLines = 1)
        }
    }
}

/** A swipe's strip: plain, and once far enough to act; its words and icon the same. Read in the draw phase. */
internal class SwipeColours(val fill: () -> Color, val armedFill: () -> Color, val ink: () -> Color, val armedInk: () -> Color)

/** One track. Numbered rows (an album) carry no artwork; everywhere else the cover leads. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun SongRow(
    song: Song, coverUrl: String?, onClick: () -> Unit, onMenu: () -> Unit, modifier: Modifier = Modifier,
    number: Int? = null, playing: Boolean = false, downloaded: Boolean = false, pageWhole: Boolean = false,
    selected: Boolean = false, onLongClick: (() -> Unit)? = null, swipeRight: RowSwipe? = null, swipeLeft: RowSwipe? = null,
    /** The second line: the song's own (its explicit mark and artist), or what the page makes of it (an album's, without its own artist). */
    line: String = song.line,
) {
    val scheme = MaterialTheme.colorScheme
    val swipe = if (swipeRight != null || swipeLeft != null) remember { SwipeState() } else null
    val place = remember { RowPlace() }
    val head = LocalPinnedHead.current
    Column(modifier.fillMaxWidth().background(if (selected) scheme.secondaryContainer else Color.Transparent).touchedInSight(place, head)) {
      Box(Modifier.fillMaxWidth()) {
        if (swipe != null) SwipeBackdrop(swipe, swipeRight, swipeLeft, Modifier.matchParentSize())
        Row(
            Modifier.fillMaxWidth()
                .then(if (swipe != null) Modifier.swipeable(swipe, swipeRight, swipeLeft, if (selected) scheme.secondaryContainer else scheme.background, shown = { place.inSight(head) }) else Modifier)
                .combinedClickable(onClick = onClick, onLongClick = onLongClick)
                .padding(start = Space.gutter, end = MENU_INSET, top = 9.dp, bottom = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // The track playing shows a waveform where its number would be - the same cue Apple uses, and
            // clearer at a glance than the title merely changing colour.
            if (number != null) Box(Modifier.width(26.dp), Alignment.Center) {
                if (playing) PlayingBars(scheme.primary, Modifier.size(16.dp))
                else Text(
                    if (number > 0) "$number" else "", textAlign = TextAlign.Center,
                    style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant,
                )
            } else Cover(coverUrl, 44.dp, radius = Radius.cover)
            Column(Modifier.weight(1f).padding(start = if (number != null) 14.dp else 12.dp, end = 8.dp)) {
                // With no second line (an album's own tracks) the cloud leads the title rather than standing alone under it.
                RemoteLine(
                    say.title(song.title), song.isExternal && line.isEmpty(), MaterialTheme.typography.bodyLarge,
                    color = if (playing) scheme.primary else scheme.onSurface,
                )
                // Worked out by the core when the song was read (`fmt::song_line`), not per row.
                if (line.isNotEmpty()) RemoteLine(line, song.isExternal, MaterialTheme.typography.bodySmall)
            }
            val tint = scheme.onSurfaceVariant
            // Every row's marks sit in columns of their own, the same width on every row: the download
            // ring or tick, then the seal, then the time, then the menu. A list of favourites then puts all
            // its seals one above another - placed after the title, the seal moved with the width of
            // the time beside it, "3:44" against "12:05" - and a mark arriving or leaving moves nothing
            // else, so a song without a seal keeps its tick in the download column. The time is last before the menu and in a box of one fixed width, its digits held to
            // the right edge, so it sits against the ⋯ rather than with an empty download slot between.
            // An album's rows have no time: its head says how long the album is, and a song's info how long it is.
            // A jam guest's seals would be the host's: none are shown.
            val rules = LocalRules.current
            Box(Modifier.padding(start = 4.dp).width(MARK), Alignment.Center) { DownloadSlot(song.id, downloaded, tint, pageWhole) }
            Box(Modifier.padding(start = MARK_GAP).width(MARK), Alignment.Center) {
                if (rules.stars && LocalStarMarks.current.effectiveStar(dev.nori.music.data.StarKind.SONG, song.id, song.starred)) Icon(Kanji.Sealed, say.favourite, Modifier.size(15.dp), LocalLook.current.color(CoverLook.ACCENT))
            }
            // A song a jam guest asked for says so in its time's place until the host decides.
            val asked = rules.asks && song.id in LocalAsked.current.value
            if (number == null || asked) Text(
                if (asked) words(R.string.jam_asked_mark) else if (song.duration > 0u) duration(song.duration.toLong()) else "",
                Modifier.padding(start = MARK_GAP).widthIn(min = TIME_SLOT), textAlign = TextAlign.End,
                style = MaterialTheme.typography.bodySmall, color = if (asked) scheme.onSurface else tint, maxLines = 1, softWrap = false,
            )
            IconButton(onMenu, Modifier.size(MENU_BUTTON, 40.dp)) { Icon(Iconoir.MoreHoriz, say.more, Modifier.size(MENU_GLYPH), tint) }
        }
      }
    }
}

/** How long the playing bars wait between steps, ms: with the frame after it, about twenty steps a second. */
private const val BARS_STEP_MS = 34L

/**
 * The bars Apple draws where a playing track's number would be. They move while the music does and
 * stand still when it is paused, which is the cue that matters: a frozen glyph beside the marked row
 * says "this one, but stopped" without a second icon.
 *
 * Moving over a page that stands still, they are drawn by an [Island] of their own over the page, which
 * repaints only their few pixels; whenever something is in the way - the row meeting the chrome or a
 * banner, cut by the list's edge, or anything round it moving (HoldIslands) - they are drawn here again, in their place
 * among the page's layers. Both read the same clock, so the hand-over does not jump. Nothing runs at all
 * while the music is paused, while this row is not composed or not [seen] (screen off, or the player over
 * the page).
 */
@Composable
fun PlayingBars(tint: Color, modifier: Modifier = Modifier) {
    // Read here rather than threaded through every list: these bars exist on exactly one row, so this
    // is one collector on one boolean, not one per song.
    val player: dev.nori.music.app.vm.PlayerViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val advancing by player.advancing.collectAsStateWithLifecycle()
    // On an e-ink screen they stand still: every frame there is paid for.
    val moving = advancing && !LocalEinkScreen.current
    val phase = remember { androidx.compose.runtime.mutableFloatStateOf(barsClock()) }
    val seen = seen()
    val drawing = remember { BarsDrawing() }
    drawing.paint.color = tint.toArgb()
    val slot = rememberIsland(drawing, wanted = moving && seen, overPage = true)
    androidx.compose.runtime.LaunchedEffect(moving, seen, slot.apart) {
        if (slot.apart || !moving || !seen) return@LaunchedEffect
        // One callback object for every frame, handed to the frame clock as it is: the millisecond
        // variants wrap it in a new lambda each frame, which is garbage for as long as the music plays.
        val tick: (Long) -> Unit = { phase.floatValue = barsClock() }
        // Not every display frame: each one is a frame of the whole screen, and every display frame of a
        // 120 Hz phone kept a page with this row on it at a fifth of a core. A score or so of steps a second
        // still reads as the bars moving with the music; see BARS_STEP_MS.
        while (coroutineContext.isActive) {
            androidx.compose.runtime.withFrameNanos(tick)
            kotlinx.coroutines.delay(BARS_STEP_MS)
        }
    }
    androidx.compose.foundation.Canvas(modifier.island(slot)) {
        if (slot.handOff()) return@Canvas
        val t = phase.floatValue
        val w = size.width / (BARS * 2 - 1)
        for (i in 0 until BARS) {
            val h = size.height * if (moving) barLevel(t, i) else RESTING[i]
            drawRoundRect(
                tint,
                androidx.compose.ui.geometry.Offset(i * w * 2f, size.height - h),
                androidx.compose.ui.geometry.Size(w, h),
                androidx.compose.ui.geometry.CornerRadius(w / 2f, w / 2f),
            )
        }
    }
}

/** The bars as their island draws them: the same heights on the same clock as [PlayingBars] draws. */
private class BarsDrawing : IslandDrawing {
    val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)

    override fun draw(canvas: android.graphics.Canvas, width: Int, height: Int): Long {
        val t = barsClock()
        val w = width.toFloat() / (BARS * 2 - 1)
        val h = height.toFloat()
        for (i in 0 until BARS) {
            val bar = h * barLevel(t, i)
            canvas.drawRoundRect(i * w * 2f, h - bar, i * w * 2f + w, h, w / 2f, w / 2f, paint)
        }
        return BARS_STEP_MS
    }
}

/** The bars' clock, seconds; one for both places they are drawn, so a hand-over does not jump. */
private fun barsClock(): Float = (android.os.SystemClock.uptimeMillis() % 3_600_000L) / 1000f

/** Bar [i]'s height at [t], as a share of the box: four speeds that never fall into step, so it reads as music, not a meter. */
private fun barLevel(t: Float, i: Int): Float = 0.28f + 0.72f * (0.5f + 0.5f * kotlin.math.sin(t * (5.1f + i * 1.3f) + i * 1.7f))

private const val BARS = 4

/** What the bars stand at while the music is paused: a shape, not a flat line. */
private val RESTING = floatArrayOf(0.532f, 0.856f, 0.64f, 0.748f)

/** What every song list draws its rows with: the song playing, the songs downloaded and selected, and the row's menu. */
@androidx.compose.runtime.Immutable
data class SongListState(val playingId: String?, val downloaded: Set<String>, val selected: Set<String>, val menu: (Song) -> Unit)

@Composable
fun rememberSongListState(actions: ActionsViewModel): SongListState {
    val player: dev.nori.music.app.vm.PlayerViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val playing by player.currentId.collectAsStateWithLifecycle()
    val done = actions.downloads.collectAsState().value.doneIds
    val selection by actions.selection.collectAsStateWithLifecycle()
    val selected = remember(selection) { selection.mapTo(HashSet()) { it.id } }
    val menu = LocalSongMenu.current
    return remember(playing, done, selected, menu) { SongListState(playing, done, selected, menu) }
}

/**
 * A song list wired to the configured tap, swipe and selection behaviour; every screen that lists songs uses
 * this. Each row is an item of its own, so only the rows on screen are composed, however long the list:
 * a thousand-song playlist costs a page the same few rows a ten-song album does.
 */
fun LazyListScope.songRows(
    songs: List<Song>, actions: ActionsViewModel, list: SongListState,
    numbered: Boolean = false, cover: (Song) -> String? = { null }, keyPrefix: String = "",
    /**
     * Which of [songs] are listed, in order, as places in it: one disc of an album, a filtered view. All of
     * them when null. A tap plays the whole of [songs] from the row's own place.
     */
    rows: List<UInt>? = null,
    /**
     * Each listed row's second line where the page words them itself: an album's, which leaves the album's
     * own artist off its tracks (the core's `DiscGroup.lines`), by the row's place in [rows]. Otherwise each
     * song's own.
     */
    lines: List<String>? = null,
    /**
     * Rows keyed by song alone (the ids must be unique) that fade and slide when the list changes under
     * them - a favourite unstarred, a mix drawn again - instead of the rows below jumping up in one frame.
     */
    animated: Boolean = false,
    /** Rows that fade in when they join a list already drawn (an artist's top songs, read after the page), keyed by place. */
    appear: Boolean = false,
    /** The page's arrival ([rememberArrival]), read in each row's draw phase: the rows on screen rise in as one block. */
    arrival: State<Float>? = null,
    /** The page [songs] are the list of: a tap that plays them from the row starts that page's queue. */
    from: dev.nori.music.ffi.model.PageOrigin? = null,
    /** The page's head says every one of [songs] is downloaded ([rememberPageWhole]): their rows carry no mark of it. */
    pageWhole: Boolean = false,
) {
    val (onRight, onLeft) = actions.swipes
    val (playingId, downloaded, selected, menu) = list
    val count = rows?.size ?: songs.size
    items(
        count,
        key = { k ->
            val at = rows?.get(k)?.toInt() ?: k
            if (animated) keyPrefix + songs[at].id else "$keyPrefix$at-${songs[at].id}"
        },
        contentType = { "song" },
    ) { k ->
        val at = rows?.get(k)?.toInt() ?: k
        val s = songs[at]
        val moving = animated || appear
        val sheet = LocalPlayerSheet.current
        val modifier = if (moving) itemMotion() else Modifier
        SongRow(
            s, if (numbered) null else remember(s) { cover(s) },
            onClick = { if (actions.tap(songs, at, from, playing = s.id == playingId)) sheet.open() }, onMenu = { menu(s) },
            modifier = if (arrival != null) modifier.arriving(arrival) else modifier,
            number = if (numbered) s.track.toInt() else null, playing = s.id == playingId, downloaded = s.id in downloaded,
            selected = s.id in selected, onLongClick = { menu(s) },
            swipeRight = rowSwipe(onRight, s, actions), swipeLeft = rowSwipe(onLeft, s, actions),
            line = lines?.getOrNull(k) ?: s.line, pageWhole = pageWhole,
        )
    }
}

/** Whether every one of [songs] is downloaded, so the page head's download mark speaks for their rows. */
@Composable
fun rememberPageWhole(songs: List<Song>, done: Set<String>): Boolean {
    val downloads = LocalRules.current.downloads
    return remember(songs, done, downloads) { downloads && songs.isNotEmpty() && songs.all { it.id in done } }
}

/** The icon and words a swipe setting uncovers under [song]'s row, and the action; null when that side does nothing. */
@Composable
internal fun rowSwipe(action: SwipeAction, song: Song, actions: ActionsViewModel): RowSwipe? {
    val starred = action == SwipeAction.FAVOURITE && LocalStarMarks.current.effectiveStar(dev.nori.music.data.StarKind.SONG, song.id, song.starred)
    // What it does is nori-core's (`row_swipe`); there are twenty answers in all, so each is asked once.
    // What it says is one of Say's words, read once per locale: a row allocates no text.
    val rules = LocalRules.current
    val act = SwipeActs.of(action, starred, rules.stars, rules.downloads) ?: return null
    val (sealed, seal) = Kanji.Sealed to Kanji.Seal
    // Made once per row and answer: a new one on every pass would make the row compose again with it.
    return remember(act, song, actions) {
        val label = say.rowSwipe(act)
        when (act) {
            dev.nori.music.ffi.library.RowSwipeAct.Queue -> RowSwipe(Iconoir.Playlist, label) { actions.enqueue(listOf(song)) }
            dev.nori.music.ffi.library.RowSwipeAct.PlayNext -> RowSwipe(Iconoir.PlaylistPlay, label) { actions.playNext(listOf(song)) }
            dev.nori.music.ffi.library.RowSwipeAct.Download -> RowSwipe(Iconoir.Download, label) { actions.download(listOf(song)) }
            is dev.nori.music.ffi.library.RowSwipeAct.Favourite -> RowSwipe(if (act.on) sealed else seal, label) { actions.star(song, act.on) }
        }
    }
}

/** The core's answer for each swipe setting, hearted or not, asked once each. */
private object SwipeActs {
    private val made = arrayOfNulls<Any>(SwipeAction.entries.size * 8)
    private val NONE = Any()
    fun of(setting: SwipeAction, starred: Boolean, stars: Boolean, downloads: Boolean): dev.nori.music.ffi.library.RowSwipeAct? {
        // A slot per setting, heart and profile kind: a place to remember the answer in, nothing more.
        val i = setting.ordinal * 8 + (if (starred) 1 else 0) + (if (stars) 2 else 0) + (if (downloads) 4 else 0)
        val got = made[i] ?: (dev.nori.music.ffi.library.rowSwipe(setting, starred, stars, downloads) ?: NONE).also { made[i] = it }
        return got as? dev.nori.music.ffi.library.RowSwipeAct
    }
}

/** A cover with its title under it: the tile every shelf and grid is made of. */
/**
 * [fill] is for a grid, where the cell decides the width and the artwork has to take all of it: given a
 * fixed width inside a wider cell the card hugs the left edge of it and the grid looks ragged.
 */
@Composable
@OptIn(ExperimentalFoundationApi::class)
fun CoverCard(
    title: String, subtitle: String, coverUrl: String?, size: Dp, onClick: () -> Unit, modifier: Modifier = Modifier,
    fill: Boolean = false, remote: Boolean = false, onLongClick: (() -> Unit)? = null,
) {
    Column((if (fill) modifier else modifier.width(size)).combinedClickable(onClick = onClick, onLongClick = onLongClick)) {
        if (fill) Cover(coverUrl, 0.dp, Modifier.fillMaxWidth().aspectRatio(1f), radius = Radius.card)
        else Cover(coverUrl, size, radius = Radius.card)
        Text(
            title, maxLines = 1, overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.bodyMedium.copy(fontSize = 14.sp, fontWeight = FontWeight.Medium),
            modifier = Modifier.padding(top = 8.dp),
        )
        if (subtitle.isNotEmpty() || remote) RemoteLine(subtitle, remote, MaterialTheme.typography.bodySmall.copy(fontSize = 12.5f.sp))
    }
}

/**
 * A line of a row or card, led by the cloud when the item is a provider's (not in the library yet). Which provider
 * is not said: to the listener remote is remote, and a name there only pushed the line out of its width.
 * The cloud comes first so the text, not the mark, is what gets cut short.
 */
@Composable
fun RemoteLine(
    text: String, remote: Boolean, style: androidx.compose.ui.text.TextStyle, modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    val tint = MaterialTheme.colorScheme.onSurfaceVariant
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        if (remote) Icon(
            Iconoir.CloudDownload, say.notInLibraryYet,
            Modifier.padding(end = if (text.isEmpty()) 0.dp else 4.dp).size(with(LocalDensity.current) { style.fontSize.toDp() } + 1.dp), tint,
        )
        if (text.isNotEmpty()) Text(text, maxLines = 1, overflow = TextOverflow.Ellipsis, style = style, color = color)
    }
}

/** A round portrait, for artists; [remote] leads the line under it with the cloud, as a [CoverCard]'s. */
@Composable
fun ArtistCard(name: String, subtitle: String, coverUrl: String?, size: Dp, onClick: () -> Unit, modifier: Modifier = Modifier, remote: Boolean = false, onLongClick: (() -> Unit)? = null) {
    Column(modifier.width(size).combinedClickable(onClick = onClick, onLongClick = onLongClick), horizontalAlignment = Alignment.CenterHorizontally) {
        Cover(coverUrl, size, radius = size / 2)
        // Two lines: names that start alike ("Aphex Twin", "Aphex Twin & …") are told apart by how they end.
        Text(
            name, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
            style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 7.dp),
        )
        if (subtitle.isNotEmpty() || remote) RemoteLine(subtitle, remote, MaterialTheme.typography.bodySmall)
    }
}

/** A long press opens its menu, unless [menu] is off (a downloaded album plays from the phone). */
@Composable
fun AlbumCard(album: Album, coverUrl: String?, size: Dp, onClick: () -> Unit, modifier: Modifier = Modifier, fill: Boolean = false, menu: Boolean = true) {
    val open = LocalCardMenu.current
    CoverCard(
        album.name,
        album.subtitle,
        coverUrl, size, onClick, modifier, fill, remote = album.isExternal,
        onLongClick = if (menu) { { open(dev.nori.music.app.vm.CardItem.OfAlbum(album)) } } else null,
    )
}

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier, onMore: (() -> Unit)? = null, compact: Boolean = false) = SectionHeader(text, modifier, onMore, compact = compact)

/**
 * Content that was not on the page at first (the songs under a hero hint) fades in once, as one block, where
 * it stands: it was not touched, so it does not move. The body of a page is many lazy items - a row each, so
 * a long list composes only what is on screen - and they share this one clock, started when [ready] turns
 * true: each item reads it in its draw phase ([arriving]), so the fade redraws the rows on screen and
 * recomposes none of them, and a row composed later (scrolled to mid-fade) joins wherever the block is. Once
 * run it stays at 1: rows scrolled to afterwards are simply there - and so is the whole body when the page
 * is come back to, which composes it again: run a second time, it showed as the page changing once more
 * after a swipe back had already brought it.
 */
@Composable
fun rememberArrival(ready: Boolean): State<Float> {
    var arrived by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
    val progress = remember { Animatable(if (arrived) 1f else 0f) }
    LaunchedEffect(ready) {
        if (!ready || progress.value >= 1f) return@LaunchedEffect
        arrived = true
        progress.animateTo(1f, AppMotion.fadeIn())
    }
    return progress.asState()
}

/**
 * One item of a page's arriving body ([rememberArrival]), read in the draw phase. The alpha is applied to
 * each thing drawn rather than through a layer of its own, so a row costs no offscreen buffer while it fades.
 */
fun Modifier.arriving(arrival: State<Float>): Modifier = graphicsLayer {
    alpha = arrival.value
    compositingStrategy = androidx.compose.ui.graphics.CompositingStrategy.ModulateAlpha
}

@Composable
fun <T> LoadBox(load: Load<T>, modifier: Modifier = Modifier, content: @Composable (T) -> Unit) {
    // A page's content fades in over its loader instead of replacing it in one frame. Keyed on the kind
    // of state only: fresh data for a page already showing just recomposes it, with no fade.
    //
    // But only once a loader has really been seen. The dots stay invisible for their first quarter
    // second (LoadingDots), so data that arrives inside that window replaces nothing anyone saw - and
    // a fade there is not a fade over a loader, it is the page's content fading up out of the page's
    // own background while the page is still sliding in (Pages): a black card arriving, and then
    // the album appearing on it. Quick data snaps in; only a page that was waiting gets the fade.
    val opened = remember { android.os.SystemClock.uptimeMillis() }
    androidx.compose.animation.AnimatedContent(
        load, contentKey = { it::class },
        transitionSpec = {
            if (android.os.SystemClock.uptimeMillis() - opened < stage.quickLoadMs) {
                androidx.compose.animation.fadeIn(androidx.compose.animation.core.snap()) togetherWith
                    androidx.compose.animation.fadeOut(androidx.compose.animation.core.snap())
            } else {
                androidx.compose.animation.fadeIn(AppMotion.fadeIn()) togetherWith androidx.compose.animation.fadeOut(AppMotion.fadeOut())
            }
        },
        label = "load",
    ) { state ->
        when (state) {
            is Load.Ready -> content(state.data)
            is Load.Loading -> Box(modifier.fillMaxSize(), Alignment.Center) { LoadingDots() }
            is Load.Failed -> Column(modifier.fillMaxSize().padding(Space.gutter), Arrangement.Center, Alignment.CenterHorizontally) {
                Text(noteText(Note.COULD_NOT_LOAD), style = MaterialTheme.typography.titleLarge)
                Text(state.message, Modifier.padding(top = 4.dp), color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
            }
        }
    }
}


/** One of the app's notes ([Note]), read once where it is shown. */
@Composable
fun noteText(note: Note): String = remember(note) { say.note(note) }

/** Big, quiet type for an empty list, under the glyph of what is missing. */
@Composable
fun EmptyNote(note: Note, modifier: Modifier = Modifier) = EmptyNote(noteText(note), modifier, noteIcon(note))

/** Big, quiet type for an empty list: "Nothing here yet", under [icon] in a soft disc when there is one. */
@Composable
fun EmptyNote(text: String, modifier: Modifier = Modifier, icon: androidx.compose.ui.graphics.vector.ImageVector? = null) =
    Column(modifier.fillMaxWidth().padding(Space.gutter), horizontalAlignment = Alignment.CenterHorizontally) {
        if (icon != null) Box(
            Modifier.padding(top = 24.dp, bottom = 14.dp).size(64.dp).background(LocalLook.current.color(dev.nori.music.look.CoverLook.VEIL_6), androidx.compose.foundation.shape.CircleShape),
            Alignment.Center,
        ) { Icon(icon, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
        Text(
            text, textAlign = TextAlign.Center,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.Medium), color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }

/** The glyph over a note of an empty list; none over help and failures. */
@Composable
private fun noteIcon(note: Note): androidx.compose.ui.graphics.vector.ImageVector? = when (note) {
    Note.NO_INDEX, Note.NO_SONGS -> Iconoir.MusicNote
    Note.NO_ALBUMS -> Iconoir.CompactDisc
    Note.NO_ARTISTS -> Iconoir.User
    Note.NO_PLAYLISTS -> Iconoir.Playlist
    Note.NO_STATIONS -> Iconoir.AntennaSignal
    Note.NOTHING_DOWNLOADED, Note.NO_DOWNLOADS -> Iconoir.Download
    Note.NO_FAVOURITE_SONGS, Note.NO_FAVOURITE_ALBUMS -> Kanji.Seal
    Note.NOTHING_TO_MIX -> Iconoir.Sparks
    Note.NO_HISTORY -> Iconoir.ClockRotateRight
    Note.NOTHING_FOUND -> Iconoir.Search
    else -> null
}

/**
 * Warms artwork that is about to be needed. A server renders each thumbnail the first time it is
 * asked for, which on a real library is the better part of a second per cover; asking for the next
 * screenful while the current one is being read turns that wait into something already done. They go
 * through the same loader, decoded at the rendition's own size into memory, so a prefetched cover is
 * simply there when its view appears, whatever size that is. For a caller that works out [urls] outside
 * composition, from a scroll observer. Main thread.
 */
fun prefetchCovers(context: android.content.Context, urls: List<String?>) {
    val loader = dev.nori.music.data.CoverLoader.get(context)
    urls.forEach { url -> if (url != null) loader.prefetch(url) }
}

/**
 * The time's column: wide enough for "59:59" in the row's small type, so every ordinary track's time
 * ends in the same place. A track over an hour is wider and pushes left, which is rare enough to allow.
 */
/** A time's width that "0:00" fills: the marks sit one [MARK_GAP] from most songs' time, and a longer one pushes them along. */
private val TIME_SLOT = 26.dp

/**
 * A song row's ⋯: narrow (as tall as ever, which sets the row's height), so it stands as near the seal as the seal does the download mark (its tap target
 * is still the minimum, past its sides), and inset so its glyph ends at the gutter, where the list's head
 * and hairline do.
 */
private val MENU_BUTTON = 32.dp
private val MENU_GLYPH = 20.dp
private val MENU_INSET = Space.gutter - (MENU_BUTTON - MENU_GLYPH) / 2
