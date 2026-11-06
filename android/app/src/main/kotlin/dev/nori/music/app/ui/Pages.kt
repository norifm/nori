package dev.nori.music.app.ui

import androidx.compose.animation.core.animate
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.drag
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.VelocityTracker1D
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.viewmodel.compose.LocalViewModelStoreOwner
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavDestination
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.NavOptions
import androidx.navigation.Navigator
import androidx.navigation.compose.LocalOwnersProvider
import androidx.navigation.createGraph
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlin.math.abs

/**
 * The pages' place in Navigation: it keeps the stack (the NavController's entries, their ViewModels and saved
 * state) and [PageStack] draws it. Each change is started with its transition and left in progress until
 * [PageStack] has shown it ([Pages.settle]): a page going away keeps its ViewModels while it is still on screen.
 */
@Navigator.Name(PageNavigator.NAME)
class PageNavigator : Navigator<PageNavigator.Destination>(NAME) {
    class Destination(navigator: PageNavigator, val content: @Composable (NavBackStackEntry) -> Unit) : NavDestination(navigator)

    val backStack get() = state.backStack
    val transitions get() = state.transitionsInProgress

    override fun createDestination() = Destination(this) {}

    override fun navigate(entries: List<NavBackStackEntry>, navOptions: NavOptions?, navigatorExtras: Extras?) {
        entries.forEach(state::pushWithTransition)
    }

    override fun popBackStack(popUpTo: NavBackStackEntry, savedState: Boolean) {
        // Every page popped stays until the change is shown, not only the lowest: a tab tapped on a page deep in
        // its stack pops the pages over the tab's own too, and the page on screen is among them.
        state.backStack.value.dropWhile { it != popUpTo }.forEach(state::prepareForTransition)
        state.popWithTransition(popUpTo, savedState)
    }

    /** [entry], under the page on top, comes on screen (a swipe back starts): started, though not on top. */
    fun prepare(entry: NavBackStackEntry) = state.prepareForTransition(entry)

    /** Every change under way has been shown: each entry goes to its place in the stack's lifecycle, or away. */
    fun settle() = state.transitionsInProgress.value.forEach(state::markTransitionComplete)

    companion object {
        const val NAME = "page"
    }
}

/** One route of the app and what it shows, for [PageStack]'s graph. */
fun NavGraphBuilder.addPage(route: String, content: @Composable (NavBackStackEntry) -> Unit) {
    addDestination(PageNavigator.Destination(provider.getNavigator(PageNavigator.NAME), content).also { it.route = route })
}

/**
 * A change of page as it is shown, between [over] and [under]. Pushed, [over] is the page opened; popped, it is
 * the page going away. [offset] is how far the change has gone from [over] to [under], px of the width, towards
 * [side] (1 the right, -1 the left), which is how far a finger has to go to finish it. A [fade] runs the same
 * number as [over]'s opacity alone, with nothing moving.
 */
@Stable
class PageChange(val over: NavBackStackEntry, val under: NavBackStackEntry, val pop: Boolean, val fade: Boolean, start: Float, val side: Float = 1f) {
    var offset by mutableFloatStateOf(start)
    /** The page shown once this is over. */
    val target: NavBackStackEntry get() = if (pop) under else over
    /** Started running by itself; a change is run once. */
    internal var ran = false
}

/**
 * The pages: a page opened fades in over the page that opened it, as a tab tapped does, with nothing moving.
 * Going back, the two pages cross-fade, each shifting a little the same way (stage.pageShift), neither crossing
 * the screen: run from rest by the back button or key, or held under the finger by a swipe on the page or
 * Android's back gesture from either edge, and, let go, finished or put back. A tab tapped from deep in a stack
 * pops, but fades all the same: Home and Library are siblings, not one under the other.
 *
 * Every change of page is one number ([PageChange.offset]), and each is drawn from live
 * pages: the page under the one on top stays composed, hidden and stopped (its lifecycle created, not
 * started), so a swipe back shows it as it is now from its first frame, without composing it on that frame.
 *
 * Navigation's NavHost could not do this: its AnimatedContent knows a time and a curve but no finger and no
 * speed, Android's back gesture alone trails the finger (the system smooths it by about 130 ms), and the page
 * underneath existed only while shown - a swipe either composed it in its first frame or showed a picture of
 * it, which went stale while covered and jumped when the page itself replaced it.
 */
@Stable
class Pages(private val controller: NavHostController, val navigator: PageNavigator, private val scope: CoroutineScope) {
    /** The width the pages fill, px. */
    var width by mutableFloatStateOf(0f)

    // What is shown changes in composition, as the stack does (changeTo), and from the motion's own coroutines;
    // only the latter are told to the composition, by [shownVersion]: it already knows of its own changes.
    private var shownVersion by androidx.compose.runtime.mutableIntStateOf(0)

    /** The page on top at rest, once every change of page has been shown. */
    private var settled: NavBackStackEntry? = null

    /** The change being shown, or null at rest. Read in composition, it follows the motion's changes. */
    val change: PageChange? get() { shownVersion; return changing }
    private var changing: PageChange? = null

    /** Held by a finger (a swipe or the back gesture) rather than run by itself. */
    private var held = false
    private var running: Job? = null

    /**
     * The page on top asks before it goes (a form with changes): no swipe or edge gesture takes it back, and
     * the system's back reaches the page's own handler.
     */
    var asks by mutableStateOf(false)

    /** The next change is a tab tapped: a fade, not a push or a pop. */
    private var tabbed = false

    /** The page on top goes back to its tab's page, or Home's: a cross-fade, whatever the stack does to get there. */
    fun tab() { tabbed = true }

    /**
     * The change that shows [top], now on top of the stack: from the page shown until now, or from the page a
     * change under way was going to. A page pushed comes in over the one shown; one popped uncovers it; a tab
     * cross-fades; a page shown no more (the stack let it go with no change, as a tab's pop does to the pages
     * above the one it pops to) is replaced at once.
     */
    internal fun changeTo(top: NavBackStackEntry): PageChange? {
        val moving = changing
        if (held && moving != null && moving.under == top) return moving
        val from = moving?.target ?: settled ?: return null.also { settled = top }
        if (from == top) return moving
        val stack = navigator.backStack.value
        val alive = from in stack || from in navigator.transitions.value
        val fade = tabbed || AppMotion.reduce
        tabbed = false
        held = false
        changing = when {
            !alive -> null.also { settled = top }
            from in stack -> PageChange(over = top, under = from, pop = false, fade = true, start = width)
            else -> PageChange(over = from, under = top, pop = true, fade = fade, start = 0f)
        }
        return changing
    }

    /** Shows [c], a change the stack made by itself (not one a finger holds), then settles on its page. */
    internal suspend fun run(c: PageChange) {
        if (c.ran || held && changing === c) return
        c.ran = true
        // Started after the frame the new page is first drawn in, however long that took, so none of it is skipped.
        withFrameNanos {}
        // A fade either way as long as the chrome's colours and the tab's own take, so a tab tapped ends at once.
        animate(c.offset, if (c.pop) width else 0f, animationSpec = if (c.fade) AppMotion.fadeIn() else AppMotion.page()) { v, _ -> c.offset = v }
        if (changing === c) settle(c.target)
    }

    private fun settle(top: NavBackStackEntry) {
        settled = top
        changing = null
        held = false
        shownVersion++
        navigator.settle()
        if (queued > 0) {
            queued--
            scope.launch { back() }
        }
    }

    /**
     * Whether the page on top can be taken back: there is a page under it, it does not ask first, and - unless
     * it is the system's [edge] gesture or the back key - it is not a tab's own page. A swipe on a tab's page
     * does not leave the tab, as on iOS; Android's back from it goes to Home, the same way as any other back.
     */
    fun canGoBack(stack: List<NavBackStackEntry>, edge: Boolean): Boolean =
        !asks && stack.size > 1 && (edge || stack.last().destination.route !in ROOTS)

    /** Starts holding the page on top for a swipe back from [side] (1 the left edge, -1 the right); false when it cannot go. */
    fun begin(edge: Boolean, side: Float = 1f): Boolean {
        val stack = navigator.backStack.value
        if (changing != null || width <= 0f || !canGoBack(stack, edge)) return false
        val under = stack[stack.size - 2]
        navigator.prepare(under)
        held = true
        changing = PageChange(over = stack.last(), under = under, pop = true, fade = false, start = 0f, side = side)
        shownVersion++
        return true
    }

    /** Puts the page held at [x] px from its place, towards the side the swipe goes. */
    fun dragTo(x: Float) { changing?.takeIf { held }?.offset = x.coerceIn(0f, width) }

    /**
     * The finger let go at [velocity] px/s towards going back: the rest of the change runs. [commit] says whether to go back: when null,
     * the motion decides - a flick, or past [COMMIT] of the width without the finger turning back. [flung]:
     * the system decided it was a back from a gesture, which goes at least at a flick's speed - a quick edge
     * flick can end on one or two finger positions, too few to measure a speed from.
     */
    fun release(velocity: Float, commit: Boolean? = null, flung: Boolean = false) {
        val c = changing?.takeIf { held } ?: return
        val along = if (flung) maxOf(velocity, FLICK * 2f) else velocity
        val back = commit ?: (along > FLICK || c.offset > width * COMMIT && along > -FLICK)
        if (back) controller.popBackStack()
        running?.cancel()
        running = scope.launch {
            if (back) {
                animate(c.offset, width, animationSpec = AppMotion.page(1f - c.offset / width)) { v, _ -> c.offset = v }
                if (changing === c) settle(c.under)
            } else {
                animate(c.offset, 0f, animationSpec = AppMotion.page(c.offset / width)) { v, _ -> c.offset = v }
                if (changing === c) settle(c.over)
            }
        }
    }

    /** Backs asked for while another is still under way: each runs as soon as the one before it is done. */
    private var queued = 0

    /**
     * The back button or key: the same motion, from rest, the way it goes when let go. Asked again while a
     * back is under way, it waits for it - popped at once, the stack lost the page the motion was still
     * taking away, and the motion then popped one page too many.
     */
    fun back() {
        when {
            changing != null -> queued++
            begin(edge = true) -> release(0f, commit = true)
            else -> controller.popBackStack()
        }
    }

    companion object {
        /** A flick, px/s: faster than this along the swipe goes back however short it was. */
        private const val FLICK = 1000f
        /** Past this share of the width a slow swipe goes back. */
        private const val COMMIT = 0.35f
        val ROOTS = setOf("home", "search", "library", "settings")
    }
}

/** Kept here so a page can reach the app's one [Pages]. */
val LocalPages = androidx.compose.runtime.staticCompositionLocalOf<Pages?> { null }

/**
 * Whether this page is the one the stack shows or is going to: the page on top at rest, a page coming in, or
 * the page coming back while the one over it goes (or is held going). Only that page lends the chrome and the
 * status bar its colours.
 */
val LocalPageFront = androidx.compose.runtime.compositionLocalOf { true }

@Composable
internal fun rememberPages(controller: NavHostController, navigator: PageNavigator): Pages {
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    return remember(controller) { Pages(controller, navigator, scope) }
}

/** How a page is shown: on top at rest, hidden under it, or one of the two pages of a change. */
private enum class Role { TOP, HIDDEN, OVER, UNDER }

/**
 * Draws [pages]' stack of the graph [build] makes, from [start]: the page on top, and, hidden, the page under
 * it; while the page changes, the two the change moves. See [Pages].
 */
@Composable
fun PageStack(controller: NavHostController, pages: Pages, start: String, modifier: Modifier = Modifier, build: NavGraphBuilder.() -> Unit) {
    val owner = checkNotNull(LocalViewModelStoreOwner.current)
    val lifecycle = LocalLifecycleOwner.current
    controller.setViewModelStore(owner.viewModelStore)
    controller.graph = remember(controller) { controller.createGraph(start, builder = build) }
    androidx.compose.runtime.DisposableEffect(lifecycle) { controller.setLifecycleOwner(lifecycle); onDispose {} }
    val stack by pages.navigator.backStack.collectAsState()
    val top = stack.lastOrNull() ?: return
    val holder = rememberSaveableStateHolder()
    val started = remember(top) { pages.changeTo(top) }
    // A page shown with no change (the first, or one replaced at once) is in place, and resumed, straight away.
    LaunchedEffect(top, started) { if (started != null) pages.run(started) else pages.navigator.settle() }
    val change = pages.change
    HoldIslands(change != null)
    // Let go of the focus of a page as it is covered: a field on it kept the keyboard up over the page opened.
    val focus = androidx.compose.ui.platform.LocalFocusManager.current
    LaunchedEffect(top) { focus.clearFocus() }
    // The page under the one on top is composed once the main thread has nothing else to do, so composing it
    // never holds up the frame a change of page ends on.
    val idle = rememberIdle(top)
    val shown: List<Pair<NavBackStackEntry, Role>> = when {
        change != null -> listOf(change.under to Role.UNDER, change.over to Role.OVER)
        else -> listOfNotNull(stack.getOrNull(stack.size - 2)?.takeIf { idle }?.let { it to Role.HIDDEN }, top to Role.TOP)
    }
    Box(modifier.fillMaxSize().onSizeChanged { pages.width = it.width.toFloat() }) {
        for ((entry, role) in shown) key(entry.id) {
            ShownPage(pages, entry, role, change) {
                entry.LocalOwnersProvider(holder) { (entry.destination as PageNavigator.Destination).content(entry) }
            }
        }
    }
    // Every page that was on screen, settled into place when the stack leaves the composition.
    androidx.compose.runtime.DisposableEffect(Unit) { onDispose { pages.navigator.settle() } }
}

/**
 * One page of the stack as [role] has it: shifted and faded with the other in a change of page, not placed at
 * all while hidden - composed, so it is there the moment a swipe shows it, but neither drawn nor touched. Opaque
 * only for a [PageChange.fade], which fades the page on top over the page under it.
 */
@Composable
private fun ShownPage(pages: Pages, entry: NavBackStackEntry, role: Role, change: PageChange?, content: @Composable () -> Unit) {
    val background = MaterialTheme.colorScheme.background
    val outStart = LocalPageStart.current
    val outEnd = LocalPageEnd.current
    val front = when (role) {
        Role.TOP -> true
        Role.HIDDEN -> false
        Role.OVER -> change?.pop == false
        Role.UNDER -> change?.pop == true
    }
    val swipes = role == Role.TOP && entry.destination.route !in Pages.ROOTS
    Box(
        Modifier.fillMaxSize()
            .layout { measurable, constraints ->
                val p = measurable.measure(constraints)
                layout(p.width, p.height) { if (role != Role.HIDDEN) p.place(0, 0) }
            }
            .graphicsLayer {
                val c = change ?: return@graphicsLayer
                val w = pages.width.takeIf { it > 0f } ?: return@graphicsLayer
                val gone = (c.offset / w).coerceIn(0f, 1f)
                val shift = c.side * stage.pageShift.dp.toPx()
                when {
                    role == Role.OVER && c.fade -> alpha = 1f - gone
                    role == Role.OVER -> { alpha = 1f - gone; translationX = shift * gone }
                    role == Role.UNDER && !c.fade -> { alpha = gone; translationX = -shift * (1f - gone) }
                }
            }
            .drawBehind { if (change?.fade == true) drawRect(background) }
            .swipeBack(pages, enabled = swipes),
    ) {
        CompositionLocalProvider(LocalPageFront provides front) {
            Box(Modifier.fillMaxSize().padding(start = outStart, end = outEnd)) { content() }
        }
    }
}

/** The swipe right on a page that takes it back; a child that takes the drag (a row of covers, a slider) keeps it. */
private fun Modifier.swipeBack(pages: Pages, enabled: Boolean): Modifier = if (!enabled) this else pointerInput(pages) {
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
                if (!pages.begin(edge = false)) return@awaitEachGesture
                change.consume()
                break
            }
        }
        val tracker = VelocityTracker()
        var x = 0f
        val done = drag(down.id) { change ->
            x = (x + change.positionChange().x).coerceAtLeast(0f)
            pages.dragTo(x)
            tracker.addPointerInputChange(change)
            change.consume()
        }
        pages.release(if (done) tracker.calculateVelocity().x else 0f, commit = if (done) null else false)
    }
}

/**
 * Android's back gesture, from either edge, and the back key: the page follows the gesture's raw finger
 * position (its progress is smoothed), and goes at the speed measured from it. The player's and the
 * selection's handlers, registered after this, come before it while they are up.
 */
@Composable
fun EdgeBack(pages: Pages, sheet: PlayerSheet) {
    val stack by pages.navigator.backStack.collectAsState()
    androidx.activity.compose.PredictiveBackHandler(!sheet.isOpen && pages.canGoBack(stack, edge = true)) { events ->
        // The gesture's events have to be taken whatever happens (the handler throws otherwise): a second quick
        // swipe, while the first page is still leaving, is a back of its own, run after it.
        var side = 0f
        var start = Float.NaN
        val speed = VelocityTracker1D(isDataDifferential = false)
        try {
            events.collect { e ->
                if (start.isNaN()) {
                    start = e.touchX
                    side = if (e.swipeEdge == androidx.activity.BackEventCompat.EDGE_RIGHT) -1f else 1f
                    if (!pages.begin(edge = true, side = side)) side = 0f
                }
                if (side == 0f) return@collect
                pages.dragTo((e.touchX - start) * side)
                speed.addDataPoint(android.os.SystemClock.uptimeMillis(), (e.touchX - start) * side)
            }
            // The back key, with no gesture, runs from rest; a gesture the system took as back goes at a flick's speed at least.
            when {
                start.isNaN() -> pages.back()
                side == 0f -> pages.back()
                else -> pages.release(speed.calculateVelocity(), commit = true, flung = true)
            }
        } catch (e: kotlinx.coroutines.CancellationException) {
            if (side != 0f) pages.release(0f, commit = false)
            throw e
        }
    }
}

/**
 * False until the main thread has nothing left to do after the frame [key] changed in (and stays true until it
 * changes again): for work that can wait for a quiet moment rather than lengthen a frame that matters.
 */
@Composable
internal fun rememberIdle(key: Any): Boolean {
    var idle by remember(key) { mutableStateOf(false) }
    val view = androidx.compose.ui.platform.LocalView.current
    androidx.compose.runtime.DisposableEffect(view, key) {
        val handler = android.os.MessageQueue.IdleHandler { idle = true; false }
        val queue = android.os.Looper.myQueue()
        view.post { queue.addIdleHandler(handler) }
        onDispose { queue.removeIdleHandler(handler) }
    }
    return idle
}
