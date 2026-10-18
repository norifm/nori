package dev.nori.music.app.ui

import androidx.compose.foundation.layout.height
import androidx.compose.ui.layout.layout
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.drawBehind
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.navigationBarsIgnoringVisibility
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsIgnoringVisibility
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.Column
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.drawWithCache
import dev.nori.music.look.CoverLook
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.ui.composed
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.graphics.vector.ImageVector
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.Spacer
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.PlayerViewModel

/**
 * The chrome that never leaves: what is playing, and where to go, in one outline - the mini player over the
 * tabs with a hairline between them, nothing filled, no shadow, no Material indicator pill - over the page,
 * which the list scrolls under and fades into.
 *
 * It is drawn in two layers, each drawing its own part of the outline. This one, under the player sheet, is
 * the mini player (the sheet grows out of it, so it is simply covered as the sheet rises) with room left below
 * it for the tabs. The tabs are [TabBar], over the sheet, so that they can slide down out of the way as the
 * player opens rather than vanish under it in one frame. The two are split because only the tabs can move:
 * the mini player holds the drag that opens the player, and moving the element a drag started on corrupts it.
 * On its side the mini player is the foot of the rail ([TabRail]), along the bottom of the page; [railLeft]:
 * which side the rail is on.
 */
@Composable
fun BottomChrome(
    player: PlayerViewModel, actions: ActionsViewModel, onOpenPlayer: () -> Unit, tabsHeight: androidx.compose.ui.unit.Dp, look: Look,
    railLeft: Boolean,
) {
    // The chrome is only outlines, so the page has to be clear under it: a wash from a little above the
    // mini player, the page's own color by the time it reaches the tabs, which the list fades out into as
    // it passes behind. Apple gets this from blurring what is behind the bars; one vertical gradient costs
    // nothing and reads much the same. Made once per size and page color, not on every draw.
    // Not on its side: there the bar floats over a page cut into halves, and a wash across the bottom of it
    // ran over the cover and stopped at the camera's strip; the mini player carries the page in itself.
    val wide = LocalWide.current
    Column(
        Modifier.drawWithCache {
            // Clear at the top of the wash, mostly page at the mini player's top edge, all page by its middle.
            val above = WASH_ABOVE.toPx()
            val total = size.height + above
            val page = look.color(CoverLook.CHROME_PAGE)
            val top = above / total
            val fade = androidx.compose.ui.graphics.Brush.verticalGradient(
                0f to Color.Transparent, top * 0.55f to look.color(CoverLook.CHROME_FADE), top to page.copy(alpha = 0.94f),
                (top + 22.dp.toPx() / total).coerceAtMost(1f) to page, 1f to page,
                startY = -above, endY = size.height,
            )
            onDrawBehind { if (!wide) drawRect(fade, Offset(0f, -above), androidx.compose.ui.geometry.Size(size.width, size.height + above)) }
        },
    ) {
        SelectionBar(actions)
        // Upright the tabs carry on below it; on its side the rail, at the end it stands against, square there.
        val r = CHROME_CORNER
        val (shape, open) = when {
            !wide -> androidx.compose.foundation.shape.AbsoluteRoundedCornerShape(r, r, 0.dp, 0.dp) to 2
            railLeft -> androidx.compose.foundation.shape.AbsoluteRoundedCornerShape(0.dp, r, r, 0.dp) to 3
            else -> androidx.compose.foundation.shape.AbsoluteRoundedCornerShape(r, 0.dp, 0.dp, r) to 1
        }
        val chrome = ChromeColours(look)
        val fill = if (wide) chrome.page else Color.Transparent
        Box(
            Modifier.padding(start = if (wide && railLeft) 0.dp else BAR_END, end = if (wide && !railLeft) 0.dp else BAR_END)
                .drawBehind {
                    if (size.height == 0f) return@drawBehind
                    val c = r.toPx()
                    val corners = when (open) { 2 -> floatArrayOf(c, c, 0f, 0f); 3 -> floatArrayOf(0f, c, c, 0f); else -> floatArrayOf(c, 0f, 0f, c) }
                    outlinePart(corners, chrome.outline, fill, skip = open)
                },
        ) { MiniPlayer(player, actions, onOpenPlayer, look, shape) }
        Spacer(Modifier.height(tabsHeight))
        // On its side there is no bar under it: its foot stands where the rail's ends (BAR_END off the
        // bottom), clear of the gesture bar only when there is one there.
        if (wide) Spacer(Modifier.height(BAR_END)) else Spacer(Modifier.windowInsetsPadding(gestureBarRoom))
    }
}

/**
 * The tabs, over the player sheet: the lower part of the chrome's outline, under the mini player, or a pill
 * of its own while nothing plays. As the sheet rises they slide down off the screen - gone by the time it is
 * 70 % open - and come back up as it closes, read in the draw phase from the sheet's progress so nothing
 * recomposes while it moves. Slid away, they are out of reach as well as sight.
 */
@Composable
fun TabBar(route: String?, tabs: List<Tab>, onTab: (String) -> Unit, look: Look, onHeight: (androidx.compose.ui.unit.Dp) -> Unit) {
    val chrome = ChromeColours(look)
    val sheet = LocalPlayerSheet.current
    val density = androidx.compose.ui.platform.LocalDensity.current
    Column(
        Modifier.graphicsLayer {
            val t = (sheet.progress.value / 0.7f).coerceIn(0f, 1f)
            translationY = t * (size.height + 12.dp.toPx())
        },
    ) {
        Row(
            Modifier.onGloballyPositioned { onHeight(with(density) { it.size.height.toDp() }) }
                .padding(start = BAR_END, end = BAR_END, bottom = BAR_OFF)
                .fillMaxWidth().height(BAR_THICK)
                .drawBehind {
                    if (sheet.miniTop > 0f) {
                        val c = CHROME_CORNER.toPx()
                        outlinePart(floatArrayOf(0f, 0f, c, c), chrome.outline, Color.Transparent, skip = 0)
                        drawLine(chrome.divider, Offset(1f, 0.5f), Offset(size.width - 1f, 0.5f), 1f)
                    } else outlinePart(FloatArray(4) { size.height / 2 }, chrome.outline, Color.Transparent)
                }
                .padding(horizontal = BAR_INSET),
            Arrangement.SpaceEvenly, Alignment.CenterVertically,
        ) {
            tabs.forEach { t -> TabButton(t, route == t.route, chrome, Modifier.fillMaxHeight()) { onTab(t.route) } }
        }
        Spacer(Modifier.windowInsetsPadding(gestureBarRoom))
    }
}

/**
 * A window wider than it is tall - a phone on its side, a tablet held across, a car's screen - where the
 * app lays itself out across rather than down: the tabs on a rail, the player and the cover pages in two
 * halves. Decided by the window's size, not by which way the device is held. Laid out down, a cover as
 * wide as the window is taller than any such window, and the controls sit a screen's scroll below it: a
 * car's 1024 x 600 dp screen, taller than a phone's side but still wider than tall, showed only the cover.
 */
val LocalWide = androidx.compose.runtime.compositionLocalOf { false }

/**
 * On its side, the cover on the right and the controls and words on the left, beside the driver of a car
 * whose wheel is on the left ([DriverSide]). Off, the cover is on the left, as a phone on its side has it.
 */
val LocalCoverAtEnd = androidx.compose.runtime.compositionLocalOf { false }

/** Whether a window of this size is laid out across ([LocalWide]). */
fun isWide(widthDp: Int, heightDp: Int): Boolean = widthDp > heightDp

/**
 * The tabs on a wide, short window (a phone on its side): [TabBar] as it lies on the glass, not laid out
 * again. Turned, the phone takes the bar with it - the edge it stood on is a side edge now - so the rail is
 * the same outline, as thick, the same margins, and its tabs in the same order under the finger: only the
 * glyphs turn upright ([LocalTabTurn]). The mini player, which stood on the bar, is the rail's foot along the
 * bottom of the page ([BottomChrome]): the two are one outline bent round the corner, with the hairline where
 * they meet. [atLeft]: the phone turned the other way round, which leaves that edge on the left and the tabs
 * in the other order. The rail is shortened only by what the status bar takes from the end it reaches. It
 * slides off its edge as the player opens, as the bar slides down. How much of that edge it takes is
 * [tabRailWidth], which the page leaves to it.
 */
@Composable
fun TabRail(
    route: String?, tabs: List<Tab>, onTab: (String) -> Unit, look: Look,
    atLeft: Boolean,
) {
    val chrome = ChromeColours(look)
    // Upright the bar reads Home, Library, Settings, Search from the left. Turned left, that end is at the
    // bottom; turned right, at the top.
    val along = if (atLeft) tabs else tabs.reversed()
    val sheet = LocalPlayerSheet.current
    val density = androidx.compose.ui.platform.LocalDensity.current
    var reach by remember { androidx.compose.runtime.mutableFloatStateOf(0f) }
    // Off the edge by what the bar is off the bottom upright: its own gap and the gesture bar's height,
    // which is the same strip of the screen on its side as it was upright.
    val off = with(density) { gestureBarRoom.getBottom(this).toDp() } + BAR_OFF
    val status = with(density) { statusBarRoom.getTop(this).toDp() }
    androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxHeight()) {
        // Upright the bar is the screen's short side less the margins - the screen's height here - and no
        // longer than what is left under the status bar.
        val length = minOf(maxHeight - BAR_END * 2, maxHeight - status - 4.dp - BAR_END)
        Column(
            Modifier.fillMaxHeight()
                .onGloballyPositioned {
                    reach = if (atLeft) it.boundsInRoot().right else it.findRootCoordinates().size.width - it.boundsInRoot().left
                }
                .graphicsLayer {
                    val t = (sheet.progress.value / 0.7f).coerceIn(0f, 1f)
                    translationX = (if (atLeft) -1f else 1f) * t * (reach + 12.dp.toPx())
                }
                .padding(start = if (atLeft) off else 4.dp, end = if (atLeft) 4.dp else off, bottom = BAR_END),
            Arrangement.Bottom, Alignment.CenterHorizontally,
        ) {
            Column(
                Modifier.width(BAR_THICK).height(length)
                    .drawBehind {
                        // The mini player's height, where it stands against the rail's foot: its top to the
                        // rail's bottom, which both keep BAR_END off the bottom of the window.
                        val foot = if (sheet.miniTop > 0f) (sheet.rootHeight - BAR_END.toPx() - sheet.miniTop).coerceIn(0f, size.height) else 0f
                        if (foot == 0f) { outlinePart(FloatArray(4) { size.width / 2 }, chrome.outline, chrome.page); return@drawBehind }
                        val c = CHROME_CORNER.toPx()
                        // Square at the corner the foot comes out of, and that stretch of the inner edge the divider.
                        outlinePart(if (atLeft) floatArrayOf(c, c, 0f, c) else floatArrayOf(c, c, c, 0f), chrome.outline, chrome.page, skip = if (atLeft) 1 else 3)
                        val x = if (atLeft) size.width - 0.5f else 0.5f
                        val joint = size.height - foot
                        drawLine(chrome.outline, Offset(x, c), Offset(x, joint), 1f)
                        drawLine(chrome.divider, Offset(x, joint), Offset(x, size.height - 1f), 1f)
                    }
                    .padding(vertical = BAR_INSET),
                Arrangement.SpaceEvenly, Alignment.CenterHorizontally,
            ) {
                // Each tab is the bar's, laid along the rail as the bar lay along the bottom edge: its name reads
                // up the rail (down it, turned the other way), like a book's spine, and only the glyph stands up.
                val degrees = if (atLeft) 90f else -90f
                along.forEach { t -> TabButton(t, route == t.route, chrome, Modifier.fillMaxWidth().laidAlong(degrees), upright = -degrees) { onTab(t.route) } }
            }
        }
    }
}

/**
 * How much of the screen's edge the rail takes (it stands against the edge): its thickness and its two margins,
 * one of them the gesture bar's height. Known before anything is laid out, so the page leaves the rail its strip
 * - and the shelves fade under it - from the very first frame, rather than one frame after the rail was measured.
 */
@Composable
fun tabRailWidth(): androidx.compose.ui.unit.Dp = tabRailReach() + 4.dp

/** How far from the screen's edge the rail's inner side is: where its foot, the mini player, ends. */
@Composable
fun tabRailReach(): androidx.compose.ui.unit.Dp {
    val d = androidx.compose.ui.platform.LocalDensity.current
    return BAR_THICK + BAR_OFF + with(d) { gestureBarRoom.getBottom(this).toDp() }
}

/** The tab bar's measures, shared by the bar and the rail so a turn changes neither. */
private val BAR_THICK = 50.dp
private val BAR_INSET = 6.dp
private val BAR_END = 10.dp
private val BAR_OFF = 6.dp
/** The corners of the outline the mini player and the tabs share. */
private val CHROME_CORNER = 22.dp
/** How far above the mini player the page's wash begins. */
private val WASH_ABOVE = 40.dp
/** How long a tab takes to light, and its name to open. */
private val TAB_FADE = androidx.compose.animation.core.tween<Color>(220)

/**
 * The chrome's colors, all of them the page's (the chrome is see-through): its ink, the idle tabs' quieter
 * ink, the outline and the fainter hairline between the mini player and the tabs, and the accent the one
 * current thing wears, which the page's look has already made read on it (nori_look::dress::accented on a
 * cover's page).
 */
@androidx.compose.runtime.Immutable
private class ChromeColours(look: Look) {
    val ink = look.color(CoverLook.CHROME_CONTENT)
    val quiet = look.color(CoverLook.CHROME_CONTENT_65)
    val outline = look.color(CoverLook.CHROME_OUTLINE)
    val divider = outline.copy(alpha = outline.alpha * 0.45f)
    val accent = look.color(CoverLook.ACCENT)
    val page = look.color(CoverLook.CHROME_PAGE)
}

/**
 * One part of the chrome's outline: [fill] inside a hairline of [colour], with these corner radii in pixels
 * (top left, top right, bottom right, bottom left). [skip] (0 top, 1 right, 2 bottom, 3 left) is the side
 * another part carries on from, which this one leaves undrawn.
 */
private fun androidx.compose.ui.graphics.drawscope.DrawScope.outlinePart(r: FloatArray, colour: Color, fill: Color, skip: Int? = null) {
    val w = size.width
    val h = size.height
    if (fill.alpha > 0f) drawPath(
        androidx.compose.ui.graphics.Path().apply {
            addRoundRect(androidx.compose.ui.geometry.RoundRect(0f, 0f, w, h,
                androidx.compose.ui.geometry.CornerRadius(r[0]), androidx.compose.ui.geometry.CornerRadius(r[1]),
                androidx.compose.ui.geometry.CornerRadius(r[2]), androidx.compose.ui.geometry.CornerRadius(r[3])))
        },
        fill,
    )
    // The hairline's middle, half a pixel inside the edge.
    val l = 0.5f; val t = 0.5f; val rt = w - 0.5f; val b = h - 0.5f
    // Where each side starts and ends going clockwise from the top, and the corner after it.
    val starts = arrayOf(Offset(l + r[0], t), Offset(rt, t + r[1]), Offset(rt - r[2], b), Offset(l, b - r[3]))
    val ends = arrayOf(Offset(rt - r[1], t), Offset(rt, b - r[2]), Offset(l + r[3], b), Offset(l, t + r[0]))
    val corner = { i: Int ->
        val k = r[(i + 1) % 4]
        when (i) {
            0 -> androidx.compose.ui.geometry.Rect(rt - 2 * k, t, rt, t + 2 * k)
            1 -> androidx.compose.ui.geometry.Rect(rt - 2 * k, b - 2 * k, rt, b)
            2 -> androidx.compose.ui.geometry.Rect(l, b - 2 * k, l + 2 * k, b)
            else -> androidx.compose.ui.geometry.Rect(l, t, l + 2 * k, t + 2 * k)
        } to (i * 90f - 90f)
    }
    val path = androidx.compose.ui.graphics.Path()
    var i = skip ?: 3
    path.moveTo(ends[i].x, ends[i].y)
    repeat(4) { n ->
        val (oval, from) = corner(i)
        if (oval.width > 0f) path.arcTo(oval, from, 90f, false) else path.lineTo(starts[(i + 1) % 4].x, starts[(i + 1) % 4].y)
        i = (i + 1) % 4
        if (n < 3 || skip == null) path.lineTo(ends[i].x, ends[i].y)
    }
    drawPath(path, colour, style = androidx.compose.ui.graphics.drawscope.Stroke(1f))
}

/**
 * Lays its content out turned a quarter: measured with the width and height swapped and drawn turned by
 * [degrees] (90 or -90), so a row put down a rail takes the rail's length, not its thickness.
 */
private fun Modifier.laidAlong(degrees: Float): Modifier = layout { measurable, constraints ->
    val p = measurable.measure(androidx.compose.ui.unit.Constraints(constraints.minHeight, constraints.maxHeight, constraints.minWidth, constraints.maxWidth))
    layout(p.height, p.width) {
        p.placeWithLayer((p.height - p.width) / 2, (p.width - p.height) / 2) { rotationZ = degrees }
    }
}

/**
 * How far the tab glyphs are turned, in degrees, read while drawing: a turn of the phone lays the bar out
 * again where it already was, and its glyphs turn from where they were to upright, while the page fades
 * into its new layout (App). 0 at rest.
 */
val LocalTabTurn = androidx.compose.runtime.staticCompositionLocalOf<() -> Float> { { 0f } }

/**
 * What the page is kept off at its left edge on its side (the camera's strip, or the rail when it is on
 * that side), for a page that draws under it anyway: a cover runs to the screen's edge.
 */
val LocalPageStart = androidx.compose.runtime.compositionLocalOf { 0.dp }

/** What the page is kept off at its right edge on its side (the rail, or the camera's strip). */
val LocalPageEnd = androidx.compose.runtime.compositionLocalOf { 0.dp }

/**
 * The chrome's look: the page it sits on, so its outlines, its ink and the accent of its current tab are
 * the page's. Only a page that is *about* one cover - an album, an artist - wears that cover's colors; a
 * bar tinted by whatever happens to be playing turned the whole app red on screens that have nothing to
 * do with the record. The accent is the theme's on every page, made to read on a cover's.
 *
 * The bar changes with the page rather than switching over in the frame the page does: one cross-fade
 * over the span the player's own colours travel in, shared by the bar and the tabs.
 */
@Composable
fun rememberChromeLook(): Look {
    val base = LocalLook.current
    val made = pagePalette.value?.look ?: (base as? FixedLook)?.table ?: IntArray(CoverLook.LEN) { base.argb(it) }
    // Keyed on the colours, not the array: a look read out afresh is a new array with the same colours
    // every time this composes, and keying the fade on that restarted it every frame - the fade's own
    // state recomposing this, which made another array, sixty times a second on every screen.
    val kept = remember { arrayOf(made) }
    if (!kept[0].contentEquals(made) && pagesWaiting.intValue == 0) kept[0] = made
    val target = kept[0]
    val t = remember { androidx.compose.animation.core.Animatable(1f) }
    val live = remember { LiveLook { t.value }.also { it.set(null, target, 0) } }
    var first by remember { mutableStateOf(true) }
    LaunchedEffect(target) {
        if (first) { first = false; return@LaunchedEffect }
        // From wherever it has got to, so a page change mid-fade turns round instead of jumping.
        val now = IntArray(CoverLook.LEN) { live.argb(it) }
        t.snapTo(0f)
        live.set(now, target, 0)
        t.animateTo(1f, androidx.compose.animation.core.tween(420))
    }
    return live
}

data class Tab(val route: String, val label: String, val icon: ImageVector)

/**
 * One tab: its glyph, quiet until it is the current one, which takes the accent and its name beside it.
 * Only the current tab is named; the name opens out as the tab is chosen, the others closing up to make
 * room. [upright]: how far the glyph is turned back to stand up, on the rail.
 */
@Composable
private fun TabButton(tab: Tab, selected: Boolean, chrome: ChromeColours, modifier: Modifier, upright: Float = 0f, onClick: () -> Unit) {
    val colour by androidx.compose.animation.animateColorAsState(if (selected) chrome.accent else chrome.quiet, TAB_FADE, label = "tab")
    val press = remember { androidx.compose.foundation.interaction.MutableInteractionSource() }
    val turn = LocalTabTurn.current
    Row(
        modifier.clip(PillShape)
            .clickable(interactionSource = press, indication = null, onClick = onClick)
            .semantics { contentDescription = tab.label }
            .padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(tab.icon, null, Modifier.size(23.dp).graphicsLayer { rotationZ = upright + turn() }, tint = colour)
        androidx.compose.animation.AnimatedVisibility(
            selected,
            enter = androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(220, 60)) + androidx.compose.animation.expandHorizontally(androidx.compose.animation.core.tween(220), Alignment.Start),
            exit = androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(120)) + androidx.compose.animation.shrinkHorizontally(androidx.compose.animation.core.tween(220), Alignment.Start),
        ) {
            Text(
                tab.label, Modifier.padding(start = 7.dp).widthIn(max = 120.dp),
                style = MaterialTheme.typography.labelLarge.copy(fontSize = 13.sp, letterSpacing = 0.sp),
                color = colour, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/**
 * The bar above the tabs: artwork, what is playing, and the controls a thumb wants. Sideways
 * flings skip, an upward fling opens the player - the same gestures the full screen answers to.
 * No progress bar on purpose: it would tick for as long as the app is open.
 */
@Composable
fun MiniPlayer(vm: PlayerViewModel, actions: ActionsViewModel, onOpen: () -> Unit, look: Look, shape: androidx.compose.ui.graphics.Shape) {
    val content = look.color(CoverLook.CHROME_CONTENT)
    val state by vm.state.collectAsStateWithLifecycle()
    val sheet = LocalPlayerSheet.current
    val title = state.current?.title ?: state.radio ?: return
    // Gone, it leaves the tabs an outline of their own, and the sheet the whole window to rise over.
    androidx.compose.runtime.DisposableEffect(sheet) { onDispose { sheet.miniTop = 0f } }
    val settings: dev.nori.music.app.vm.SettingsViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val system = androidx.compose.foundation.isSystemInDarkTheme()
    val dark = remember(prefs.theme, system) { dev.nori.music.ffi.settings.themeIsDark(prefs.theme, system) }
    // The colours of what is playing and of the songs either side, worked out before they are reached. Their covers are
    // already fetched ahead (PlayerViewModel); this is the other half of that, and it is what stops the
    // page wearing the last song's colour for a moment after a skip. It happens here rather than in the
    // player because the bar is on screen whenever something is playing, so a skip from the
    // notification or the lock screen is covered too.
    val context = androidx.compose.ui.platform.LocalContext.current
    // Which songs either side is the core's (`covers_around`, the skips' own targets, shuffle included),
    // worked out once per skip with the covers fetched ahead.
    val near by vm.coversNear.collectAsStateWithLifecycle()
    val around = remember(near) { near.mapNotNull { vm.cover(it, CoverSize.ROW) }.filterNot(::isProviderCover) }
    LaunchedEffect(around, dark, prefs.coverColors, prefs.amoled) {
        if (prefs.coverColors.isEmpty()) return@LaunchedEffect
        val plain = !dev.nori.music.ffi.settings.coverBlack(prefs.coverColors, true, dark, prefs.amoled)
        for (url in around) {
            // The full-screen player keeps the record's colours where the bar goes black, so that is a
            // second set of colours for the same cover, worked out from the same decode.
            warmCoverPalette(context, url, dev.nori.music.ffi.look.pageBlack(dark, prefs.amoled, false), andPlain = plain)
        }
    }
    val scheme = MaterialTheme.colorScheme
    // Under the open player the bar is still composed, only covered: a title walking there redrew the
    // whole screen every frame for nobody.
    val covered = LocalPageCovered.current
    // Its outline is the chrome's, drawn round it by BottomChrome.
    Surface(
        shape = shape, color = Color.Transparent, contentColor = content,
        modifier = Modifier.fillMaxWidth()
            .semantics { contentDescription = say.nowPlayingBar }
            .onGloballyPositioned { sheet.miniTop = it.positionInRoot().y }
            // Up opens the player, following the finger the whole way; see PlayerSheet.
            .dragsSheet(sheet),
    ) {
        // The tap has to be a child of the drag detectors, not a sibling behind them: a pointerInput
        // waiting for drag slop swallows a tap offered to a clickable further up the same chain.
        Surface(onClick = onOpen, color = Color.Transparent, contentColor = content) {
        Column {
        // The jam this phone hosts or is a guest in.
        val jam by vm.jamStrip.collectAsStateWithLifecycle()
        Row(Modifier.fillMaxWidth().padding(end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            // What is playing slides aside for the next (or last) song, which comes in from the other
            // edge already showing; the buttons stay where they are. Radio has no neighbours.
            val song = state.current
            val track: @Composable (dev.nori.music.ffi.model.Song?, Boolean) -> Unit = { s, real ->
                Row(Modifier.fillMaxWidth().padding(start = 6.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Cover(
                        vm.cover(s?.coverArt, CoverSize.ROW), 40.dp,
                        if (real) Modifier.onGloballyPositioned {
                            // Only the whole square: slid part-way out of the bar it is clipped, and a
                            // thumbnail with no width is not something to grow the cover from.
                            val b = it.boundsInRoot()
                            if (b.height > 0f && kotlin.math.abs(b.width - b.height) < 1f) sheet.miniCover = b
                        } else Modifier, radius = 7.dp,
                    )
                    Column(Modifier.weight(1f).padding(horizontal = 11.dp)) {
                        // A long title here reads itself out twice when the song comes on and then
                        // settles, as the full player's does (see readable). Only the row really
                        // showing: the neighbours wait off either edge at alpha zero, and a long title
                        // there walked after every change of song, a frame each time, on every page.
                        Text(
                            s?.title ?: title, Modifier.readable(iterations = if (real && !covered) READ_OUT else 0),
                            maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.bodyLarge,
                        )
                        val error = if (real) state.error else null
                        Text(
                            remember(error, s?.artist) { say.barLine(error, s?.artist) }, maxLines = 1, overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.bodySmall,
                            color = if (real && state.error != null) scheme.error else look.color(CoverLook.CHROME_CONTENT_65),
                        )
                    }
                }
            }
            SwipeCarousel(
                current = song,
                previous = state.queue.getOrNull(state.previousIndex)?.takeIf { song != null },
                next = state.queue.getOrNull(state.nextIndex)?.takeIf { song != null },
                same = { a, b -> a?.id == b?.id },
                onPrevious = vm::previousItem, onNext = vm::next,
                still = covered,
                modifier = Modifier.weight(1f),
                item = track,
            )
            // Where the sound goes: the speaker, filled in while another device plays, a tap away from the
            // devices. Then the one judgement worth making without opening the player: whether this is a song to
            // keep. Apple has only the transport here; the owner asked for the favorite, and the bar has the room
            // for it because the title beside it is already allowed to run out of space gracefully. The skip is
            // a swipe away. A jam guest's speaker and seal would be the host's; its play is the one its role offers.
            // While a jam is on it stands in the speaker's place: a jam plays on this device only.
            jam?.let { JamButton(it, 24.dp) { look.color(CoverLook.CHROME_CONTENT_75) } }
            if (!state.jamGuest) {
                if (jam == null) OutputButton(
                    state.playingOn != null, 24.dp,
                    idle = { look.color(CoverLook.CHROME_CONTENT_75) }, lit = { content },
                )
                // Seals where songs carry favorites (a provider's only signed in): the bar's one color, the accent.
                if (LocalRules.current.stars) song?.let { s ->
                    val starred = LocalStarMarks.current.effectiveStar(dev.nori.music.data.StarKind.SONG, s.id, s.starred)
                    FavoriteSeal(starred, tint = look.color(CoverLook.ACCENT), muted = look.color(CoverLook.CHROME_CONTENT_75)) { actions.star(s, !starred) }
                }
            }
            if (state.offersPlayPause) IconButton(vm::toggle) { PlayPauseGlyph(state.playing, state.buffering, 26.dp, 20.dp) }
        }
        }
        }
    }
}

private val pagePalette = androidx.compose.runtime.mutableStateOf<PagePalette?>(null)

/**
 * How much room the floating chrome takes at the bottom. Screens add it to the bottom of their own
 * scrolling content, so a list can run underneath the mini player - the page's colour reaches the
 * bottom edge of the screen, and the last row is still reachable.
 */
val LocalChromeInset = androidx.compose.runtime.compositionLocalOf { 0.dp }

/**
 * Pages whose cover's colors are still being worked out. While there are any, the chrome keeps the
 * colors it has: from one album to the next they went to the playing song's (or the
 * theme's) for the moment the new cover took, and then to the new page's, flashing twice where the page
 * changed once.
 */
private val pagesWaiting = androidx.compose.runtime.mutableIntStateOf(0)

/**
 * A tinted page (an album, an artist, the player) lends its colours to the chrome while it is open.
 * [waiting]: it will have colours, but they are not worked out yet.
 */
@Composable
fun PageTint(palette: PagePalette?, waiting: Boolean = false) {
    androidx.compose.runtime.DisposableEffect(palette, waiting) {
        if (waiting) pagesWaiting.intValue++ else pagePalette.value = palette
        onDispose {
            if (waiting) pagesWaiting.intValue--
            else if (pagePalette.value === palette) pagePalette.value = null
        }
    }
}

/**
 * A row of records, one showing: a sideways drag slides the showing one aside and brings its neighbour
 * in from the other edge, already drawn. Let go past a third of the way, or flicked, the neighbour
 * lands and [onNext] / [onPrevious] runs; it stays drawn in place of [current] until [current] is that
 * song too (the player answers a few frames later), so the old one never comes back for a frame.
 * [item]'s second argument is true for the one really showing. Nothing runs until a finger is down, or
 * until [current] steps to a neighbour by itself, which slides the same way unless the row is [still]
 * (out of sight).
 */
@Composable
internal fun <T> SwipeCarousel(
    current: T, previous: T?, next: T?, same: (T?, T?) -> Boolean,
    onPrevious: () -> Unit, onNext: () -> Unit,
    modifier: Modifier = Modifier,
    still: Boolean = false,
    item: @Composable (T?, Boolean) -> Unit,
) {
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val haptics = androidx.compose.ui.platform.LocalHapticFeedback.current
    // Straight from the finger, not through a coroutine per pointer event: those queued up on a flick
    // and landed after the settle had started, which pulled the row back mid-change. Same story as the
    // sleeve's; see SleeveCarousel.
    var offset by remember { androidx.compose.runtime.mutableFloatStateOf(0f) }
    var moving by remember { mutableStateOf<kotlinx.coroutines.Job?>(null) }
    val hasBefore by androidx.compose.runtime.rememberUpdatedState(previous != null)
    val hasAfter by androidx.compose.runtime.rememberUpdatedState(next != null)
    val nextNow by androidx.compose.runtime.rememberUpdatedState(next)
    val previousNow by androidx.compose.runtime.rememberUpdatedState(previous)
    // The neighbour that has been slid in, shown until the player has caught up with it.
    var landed by remember { mutableStateOf<Any?>(NONE) }
    val currentNow by androidx.compose.runtime.rememberUpdatedState(current)
    androidx.compose.runtime.LaunchedEffect(landed) {
        if (landed === NONE) return@LaunchedEffect
        @Suppress("UNCHECKED_CAST")
        kotlinx.coroutines.withTimeoutOrNull(2_000) {
            androidx.compose.runtime.snapshotFlow { same(currentNow, landed as T?) }.first { it }
        }
        landed = NONE
    }
    // A song that changes by itself (its end, a mix, the bar's own next button, the notification) comes
    // in as a swipe brings it: the one it left goes out one side, the new one in from the other. As on
    // the player's sleeve, decided while composing, so the new song is never drawn in place first; only
    // for a step to a neighbour (the one left is then the neighbour, drawn where the slide starts).
    val last = remember { Last<T>() }
    val go = when {
        !last.set || same(current, last.item) || current == null -> 0
        AppMotion.reduce || still || landed !== NONE || moving?.isActive == true -> 0
        same(previous, last.item) -> 1
        same(next, last.item) -> -1
        else -> 0
    }
    /** Where the row is while it slides by itself, in rows: 1 a whole row to the right, 0 in place. */
    val natural = remember(current) { androidx.compose.animation.core.Animatable(go.toFloat()) }
    val naturalNow by androidx.compose.runtime.rememberUpdatedState(natural)
    androidx.compose.runtime.SideEffect {
        if (!last.set || !same(current, last.item)) { last.item = current; last.set = true }
    }
    androidx.compose.runtime.LaunchedEffect(natural) {
        // Counted in frames, as the sleeve's is: the new song's first frames can be slow ones.
        if (natural.value != 0f) settleByFrames(natural, 560f)
    }
    Box(
        modifier.clipToBounds().pointerInput(Unit) {
            val tracker = androidx.compose.ui.input.pointer.util.VelocityTracker()
            var x = 0f
            val release: (Float) -> Unit = { v ->
                val o = offset
                val w = size.width.toFloat()
                // The same gesture as the sleeve's, by the same rule (swipeTurn), with the bar's slower flick.
                val go = swipeTurn(o, v, w, hasBefore, hasAfter, bar = true)
                val running = moving
                moving = scope.launch {
                    running?.cancelAndJoin()
                    val settle = androidx.compose.animation.core.spring<Float>(dampingRatio = 1f, stiffness = 560f)
                    if (go == 0) {
                        androidx.compose.animation.core.animate(offset, 0f, v, settle) { value, _ -> offset = value }
                        return@launch
                    }
                    haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                    val arriving = if (go < 0) nextNow else previousNow
                    var changed = false
                    try {
                        if (AppMotion.reduce) offset = go * w
                        else androidx.compose.animation.core.animate(offset, go * w, v, settle) { value, _ -> offset = value }
                        landed = arriving
                        offset = 0f
                        changed = true
                        if (go < 0) onNext() else onPrevious()
                    } finally {
                        if (!changed) { offset = 0f; if (go < 0) onNext() else onPrevious() }
                    }
                }
            }
            // The bar answers an upward drag by opening the player (dragsSheet, on the surface around
            // this), so the sideways drag has to be plainly sideways or the two fight over every
            // diagonal - which is the bar changing the song when it was asked to open.
            sidewaysDrag(
                slop = 1.5f, ratio = 1.8f,
                onDragStart = {
                    tracker.resetTracking(); x = 0f; moving?.cancel()
                    // A row still sliding in by itself is taken over where it is.
                    val n = naturalNow.value
                    if (n != 0f) { offset += n * size.width; scope.launch { naturalNow.snapTo(0f) } }
                },
                onDragEnd = { release(tracker.calculateVelocity().x) },
                onDragCancel = { release(0f) },
            ) { change, d ->
                x += d
                tracker.addPosition(change.uptimeMillis, Offset(x, 0f))
                val w = size.width.toFloat()
                val moved = offset + d
                val allowed = (moved > 0f && hasBefore) || (moved < 0f && hasAfter)
                offset = if (allowed) moved.coerceIn(-w, w) else (offset + d * GIVE).coerceIn(-w * GIVE_LIMIT, w * GIVE_LIMIT)
            }
        },
    ) {
        @Suppress("UNCHECKED_CAST")
        val showing = if (landed === NONE) current else landed as T?
        Box(Modifier.graphicsLayer { translationX = offset + natural.value * size.width }) { item(showing, landed === NONE) }
        Box(Modifier.graphicsLayer { val o = offset + natural.value * size.width; alpha = if (o < 0f) 1f else 0f; translationX = o + size.width }) { item(next, false) }
        Box(Modifier.graphicsLayer { val o = offset + natural.value * size.width; alpha = if (o > 0f) 1f else 0f; translationX = o - size.width }) { item(previous, false) }
    }
}

private val NONE = Any()

/** What a row of records was last composed with; written after each composition, read by the next. */
private class Last<T> {
    var item: T? = null
    var set = false
}

/**
 * The gesture bar's room, the same while the bar is put away: what stands clear of it keeps its place
 * whether the bar is there or not.
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
val gestureBarRoom: androidx.compose.foundation.layout.WindowInsets
    @Composable get() = androidx.compose.foundation.layout.WindowInsets.navigationBarsIgnoringVisibility

/** Whether the status bar's room is kept while it is away (the core's `bar_room_kept`). */
val LocalStatusRoomKept = androidx.compose.runtime.staticCompositionLocalOf { false }

/**
 * The status bar's room: kept while the bar is away where it comes and goes in front of the page (burn-in
 * protection), given up where it goes with a turn of the phone, so the page then reaches the top edge.
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
val statusBarRoom: androidx.compose.foundation.layout.WindowInsets
    @Composable get() = if (LocalStatusRoomKept.current) androidx.compose.foundation.layout.WindowInsets.statusBarsIgnoringVisibility
    else androidx.compose.foundation.layout.WindowInsets.statusBars
