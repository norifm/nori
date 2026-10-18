package dev.nori.music.app.ui

import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onStart
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.runtime.collectAsState
import android.content.Intent
import android.net.Uri
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.LibraryMusic
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.Surface
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.material3.MaterialTheme
import androidx.compose.foundation.background
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.Alignment
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import dev.nori.music.look.CoverLook
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.drawWithCache
import kotlinx.coroutines.launch
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.animation.core.animateFloat
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.PlayerViewModel
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.model.Song
import dev.nori.music.settings.loggedIn
import dev.nori.music.settings.server
import androidx.compose.foundation.layout.windowInsetsPadding

/** Plain screens sit below the status bar; album, artist and playlist pages draw under it. */
@Composable
private fun Inset(content: @Composable () -> Unit) = androidx.compose.foundation.layout.Box(Modifier.windowInsetsPadding(statusBarRoom)) { content() }

/**
 * Navigation as the screens see it; they never touch the NavController. Going anywhere puts the player
 * away first, the way an artist tapped on Apple's player drops it and opens the artist underneath.
 */
class Nav(private val c: NavHostController, private val sheet: PlayerSheet, private val swipe: PageSwipe) {
    fun go(route: String) { if (sheet.isOpen) sheet.close(); swipe.plain(); c.navigate(route) }

    /**
     * What the row that opened an album already knew about it - name, artist, cover - kept for the page
     * to draw its header from while the album itself is on the way. The page slides in (PageMotion)
     * the moment it is asked for, and without this it slid in as a bare card and filled in a few frames
     * later; with it the header is there from the first frame and only the songs arrive. The last few
     * are kept, so going back and forward between albums does not lose them.
     */
    private val albums = object : LinkedHashMap<String, dev.nori.music.ffi.model.Album>() {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, dev.nori.music.ffi.model.Album>?) = size > 8
    }
    private val artists = object : LinkedHashMap<String, dev.nori.music.ffi.model.Artist>() {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, dev.nori.music.ffi.model.Artist>?) = size > 8
    }
    private val playlists = object : LinkedHashMap<String, dev.nori.music.ffi.model.Playlist>() {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, dev.nori.music.ffi.model.Playlist>?) = size > 8
    }
    fun albumHint(id: String): dev.nori.music.ffi.model.Album? = albums[id]
    fun artistHint(id: String): dev.nori.music.ffi.model.Artist? = artists[id]
    fun playlistHint(id: String): dev.nori.music.ffi.model.Playlist? = playlists[id]

    fun album(id: String, hint: dev.nori.music.ffi.model.Album? = null) {
        if (hint != null) albums[id] = hint
        go("album/${Uri.encode(id)}")
    }
    fun downloadedAlbum(id: String, hint: dev.nori.music.ffi.model.Album) {
        albums[id] = hint
        go("downloaded/${Uri.encode(id)}")
    }
    fun artist(id: String, hint: dev.nori.music.ffi.model.Artist? = null) {
        if (hint != null) artists[id] = hint
        go("artist/${Uri.encode(id)}")
    }
    fun playlist(id: String, hint: dev.nori.music.ffi.model.Playlist? = null) {
        if (hint != null) playlists[id] = hint
        go("playlist/${Uri.encode(id)}")
    }
    fun genre(name: String) = go("genre/${Uri.encode(name)}")
    fun folder(id: String) = go("folder/${Uri.encode(id)}")
    fun decade(year: Int) = go("decade/$year")
    fun smart(id: String) = go("smart/${Uri.encode(id)}")
    fun mix(id: String) = go("mix/${Uri.encode(id)}")
    fun smartEdit(id: String) = go("smartEdit/${Uri.encode(id.ifEmpty { "new" })}")
    fun stats() = go("stats")
    /** The download queue. Asked for again while it is showing (the notification tapped), it only puts the player away. */
    fun downloads() { if (c.currentDestination?.route == "downloads") { if (sheet.isOpen) sheet.close() } else go("downloads") }
    /** A settings group, optionally landing on one row of it (from the settings search). */
    fun settingsGroup(id: String, key: String = "") = go("settings/$id?key=${Uri.encode(key)}")
    /** Opens the player, on [panel] if one is asked for. */
    fun player(panel: Panel? = null) { sheet.panelAsked = panel; sheet.open() }
    fun equalizer() = go("equalizer")
    fun homeRows() = go("home-rows")
    fun librarySections() = go("library-sections")
    fun librarySection(s: dev.nori.music.ffi.settings.LibrarySection) = go("library/${s.name}")
    fun autoEq() = go("autoeq")
    fun back() { if (sheet.isOpen) sheet.close() else swipe.back() }
    /**
     * A tab always lands on that tab's own page: nothing above the tab roots survives a tab tap. A saved
     * and restored stack would put an album straight back after Home popped it, and the tab would look dead.
     */
    fun tab(route: String) {
        if (sheet.isOpen) sheet.close()
        // Tapping Search is a request for the keyboard, whether or not the screen is already open -
        // and with launchSingleTop it is not recomposed, so nothing else would notice the tap.
        if (route == "search") searchTaps.intValue++
        // Already there: nothing to go to. Navigating anyway ran the page's arrival again on every tap,
        // so a tab pressed twice redrew the page it was already on.
        if (c.currentBackStackEntry?.destination?.route == route) return
        PageMotion.tabbedTo = route
        swipe.plain()
        c.navigate(route) {
            popUpTo(c.graph.startDestinationId)
            launchSingleTop = true
        }
    }
}

/** Counts taps on the search tab; the search field takes focus again on each one. */
private val searchTaps = androidx.compose.runtime.mutableIntStateOf(0)

@Composable
internal fun searchFocusKey(): Int = searchTaps.intValue

val LocalNav = staticCompositionLocalOf<Nav> { error("no nav") }
val LocalSongMenu = staticCompositionLocalOf<(Song) -> Unit> { {} }

/** The player's own ⋯: the same song menu, with the playback-wide entries the player needs. */
val LocalPlayerMenu = staticCompositionLocalOf<(Song) -> Unit> { {} }

/** Opens the devices sheet (RemoteScreens.DevicesHost): where the music plays. */
val LocalDevices = staticCompositionLocalOf<() -> Unit> { {} }

/** Says one line in the app's message pill at the top ([ActionsViewModel.tell]). */
val LocalMessages = staticCompositionLocalOf<(String) -> Unit> { {} }

/**
 * What the app offers over the profile in use (the core's `profile_rules`): a jam guest's is the same app
 * over its host's library, read only, where a song played or queued is asked of the host.
 */
val LocalRules = staticCompositionLocalOf { dev.nori.music.ffi.library.profileRules(false) }

/**
 * The songs a jam guest asked for that the host has not decided on yet, by id: their rows say "Asked". Read
 * by the rows, so a request changes them and nothing else.
 */
val LocalAsked = staticCompositionLocalOf<androidx.compose.runtime.State<Set<String>>> { androidx.compose.runtime.mutableStateOf(emptySet()) }

/** Whether the lyrics keep the screen on (LyricsView), so burn-in protection covers them too. */
val LocalLyricsKeepOn = staticCompositionLocalOf<androidx.compose.runtime.MutableState<Boolean>> { androidx.compose.runtime.mutableStateOf(false) }

private val tabs = listOf(
    Tab("home", say.home, Icons.Filled.Home),
    Tab("search", say.search, Icons.Filled.Search),
    Tab("library", say.library, Icons.Filled.LibraryMusic),
    Tab("settings", say.settings, Icons.Filled.Settings),
)

/**
 * [launchRoute] is what the activity was asked for from outside - a tap on the download notification, or
 * a launcher shortcut: a screen, or a shuffle to start - set on launch or on a new intent, and cleared
 * here once it has been done.
 */
@Composable
fun App(launchRoute: androidx.compose.runtime.MutableState<String?>? = null) {
    val settings: SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    androidx.compose.runtime.SideEffect { AppMotion.reduce = prefs.motion != dev.nori.music.ffi.settings.Motion.FULL }
    NoriTheme(prefs) {
        // Sign-in fades to the app: the screens are different enough that a direction would invent a
        // relationship they do not have.
        val plain = reduceMotion()
        // A jam guest's profile is the same app over its host's library, started afresh on joining and on leaving.
        val guest = remember(prefs.activeServerId, prefs.server?.apiKey) { prefs.server?.apiKey?.let { dev.nori.music.ffi.remote.isGuestKey(it) } == true }
        val kind = prefs.server?.kind ?: dev.nori.music.ffi.settings.ServerKind.SUBSONIC
        androidx.compose.animation.Crossfade(
            targetState = Triple(prefs.loggedIn, guest, kind),
            animationSpec = if (plain) androidx.compose.animation.core.snap() else androidx.compose.animation.core.tween(280),
            label = "session",
        ) { (loggedIn, guest, kind) ->
        if (!loggedIn) {
            LoginScreen(settings)
            return@Crossfade
        }
        // A provider account's name (its `user`) brings its library and hearts.
        val account = prefs.server?.user.orEmpty()
        val rules = remember(guest, kind, account) { dev.nori.music.ffi.rulesOf(prefs.server ?: dev.nori.music.settings.newServer(""), guest) }
        val home = "home"
        // A guest follows the host while the app is in sight: what plays, the queue, its requests.
        val remote: dev.nori.music.app.vm.RemoteViewModel = viewModel()
        if (guest) androidx.lifecycle.compose.LifecycleResumeEffect(Unit) { remote.watch(true); onPauseOrDispose { remote.watch(false) } }
        val asked = remote.asked.collectAsStateWithLifecycle()

        val controller = rememberNavController()
        val sheetScope = androidx.compose.runtime.rememberCoroutineScope()
        val sheet = remember { PlayerSheet(sheetScope) }
        sheet.plain = reduceMotion()
        // Whether the player was open, kept with the rest of the saved state, so an app Android put back
        // together (after it was stopped in the background) opens on the player it was left on.
        var playerWasOpen by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
        LaunchedEffect(sheet) {
            if (playerWasOpen) sheet.openAtOnce()
            androidx.compose.runtime.snapshotFlow { sheet.isOpen }.collect { playerWasOpen = it }
        }
        // The player covers the page, so a field on it (search) lets go of its focus: held under the player,
        // the keyboard came back over it whenever the window got its focus back (from the background, a menu).
        val focusManager = androidx.compose.ui.platform.LocalFocusManager.current
        val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
        LaunchedEffect(sheet) {
            androidx.compose.runtime.snapshotFlow { sheet.isOpen }.collect { open ->
                if (open) { focusManager.clearFocus(force = true); keyboard?.hide() }
            }
        }
        val swipe = rememberPageSwipe(controller)
        val nav = remember(controller) { Nav(controller, sheet, swipe) }
        // A jam just joined opens on it: the player, turned to the queue with the jam over it.
        if (guest) {
            var arrived by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
            LaunchedEffect(Unit) { if (!arrived) { arrived = true; nav.player(Panel.QUEUE) } }
        }
        val actions: ActionsViewModel = viewModel()
        val player: PlayerViewModel = viewModel()
        val snackbar = remember { SnackbarHostState() }
        var menuSong by remember { mutableStateOf<Song?>(null) }
        var menuFromPlayer by remember { mutableStateOf(false) }
        var devicesOpen by remember { mutableStateOf(false) }
        // Said on the profile the phone went back to: the guest's page is fading out by then.
        if (!guest) LaunchedEffect(Unit) { remote.ended.collect { snackbar.showSnackbar(say.jamEnded(it), withDismissAction = true) } }
        LaunchedEffect(Unit) {
            // Latest, not in turn: showing a message suspends until it goes away, so a plain collect
            // could not even see the next one until the last had sat out its four seconds - a quick
            // favourite then unfavourite read "Added to favourites" for the whole of that. Cancelling
            // the one that is up takes it off at once and the new one replaces it.
            actions.messages.collectLatest { msg ->
                snackbar.currentSnackbarData?.dismiss()
                snackbar.showSnackbar(msg, withDismissAction = true, duration = androidx.compose.material3.SnackbarDuration.Short)
            }
        }
        // Headphones or a DAC connected with nothing chosen for them: the service decided (see DeviceSound);
        // this only says so - "use its AutoEQ curve?", or "using AutoEQ for X" with an undo.
        // Marked as seen only once it has been on screen; unplugging the device takes it away with it.
        val eqNotice by settings.eqNotice.collectAsStateWithLifecycle()
        LaunchedEffect(eqNotice) {
            val n = eqNotice ?: return@LaunchedEffect
            // An offer waits for an answer; a curve already applied only offers to undo it.
            val offer = n.source is dev.nori.music.ffi.devices.CurveNotice.Offer
            val result = snackbar.showSnackbar(
                n.message, actionLabel = n.action, withDismissAction = offer,
                duration = if (offer) androidx.compose.material3.SnackbarDuration.Long else androidx.compose.material3.SnackbarDuration.Short,
            )
            settings.eqNoticeShown(n)
            if (result == androidx.compose.material3.SnackbarResult.ActionPerformed) settings.eqNoticeAction(n)
        }
        val context = LocalContext.current
        LaunchedEffect(Unit) {
            actions.shares.collect { url ->
                context.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url), null))
            }
        }

        if (!guest) SignInFlow(settings, kind)

        // The debug build's test bridge drives the app through this (src/debug TestDriver.kt); every
        // other build has an empty one (src/noTest), so a release carries none of it.
        dev.nori.music.app.TestDriver(controller, nav, sheet, settings, actions, player) { devicesOpen = true }

        // Whether the beat model also reads what is being downloaded, when the settings say to ask.

        // The perf build's recorder starts a new stretch when the player goes up or away. See PerfHooks.
        dev.nori.music.app.PerfHooks.recorder?.let { r ->
            LaunchedEffect(sheet, r) { androidx.compose.runtime.snapshotFlow { sheet.isOpen }.collect(r::playerOpen) }
        }

        // Read from a snapshot observer, not in composition, so a request does not recompose the app.
        if (launchRoute != null) LaunchedEffect(launchRoute, nav) {
            androidx.compose.runtime.snapshotFlow { launchRoute.value }.collect { route ->
                if (route == null) return@collect
                when (route) {
                    "downloads" -> nav.downloads()
                    // A tab, as a tap on it: the keyboard comes up with it.
                    "search" -> nav.tab(route)
                    // A widget's song: the player, or the player turned to the lyrics.
                    "player" -> nav.player()
                    "lyrics" -> nav.player(Panel.LYRICS)
                    dev.nori.music.app.SHUFFLE_SONGS -> actions.shuffleAll()
                    dev.nori.music.app.SHUFFLE_ALBUMS -> actions.shuffleAlbums()
                    else -> nav.go(route)
                }
                launchRoute.value = null
            }
        }

        val lyricsKeepOn = remember { mutableStateOf(false) }
        val config = androidx.compose.ui.platform.LocalConfiguration.current
        val wide = isWide(config.screenWidthDp, config.screenHeightDp)
        // The bars the app has put away, set below where they are decided, read by its dialogs' windows.
        val hiddenBars = remember { mutableStateOf(HiddenBars(status = false, gesture = false)) }
        CompositionLocalProvider(
            LocalNav provides nav,
            LocalLyricsKeepOn provides lyricsKeepOn,
            LocalStatusRoomKept provides dev.nori.music.ffi.settings.barRoomKept(prefs.hideStatusBar, wide),
            LocalPageSwipe provides swipe,
            LocalPlayerSheet provides sheet,
            LocalDownloadMarks provides rememberDownloadMarks(actions),
            LocalSongMenu provides { menuSong = it; menuFromPlayer = false },
            LocalPlayerMenu provides { menuSong = it; menuFromPlayer = true },
            LocalDevices provides { devicesOpen = true },
            LocalMessages provides actions::tell,
            LocalRules provides rules,
            LocalAsked provides asked,
            LocalEinkScreen provides (prefs.motion == dev.nori.music.ffi.settings.Motion.EINK),
            LocalHiddenBars provides hiddenBars,
        ) {
            PairingRequest()
            MobileDownloadQuestion(actions)
            val route = controller.currentBackStackEntryAsState().value?.destination?.route
            // The tab the page on screen belongs to, which is the one that stays lit, as Apple's does: a
            // settings group, or an album opened from Home, is still inside that tab. Lit only on the tab
            // roots themselves, the icon went out the moment anything was opened. There is one back stack
            // and a tab tap rebuilds it from the start, so the page's tab is the last root shown.
            var lastTab by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(home) }
            val onTab = route?.takeIf { r -> tabs.any { it.route == r } }
            LaunchedEffect(onTab) { if (onTab != null) lastTab = onTab }
            val tabRoute = onTab ?: lastTab
            // This session's star changes, so every heart prefers them over the snapshot it painted with.
            val marks by actions.starMarks.collectAsStateWithLifecycle()
            // The chrome floats over the page rather than ending it: the page fills the window, its colour
            // reaches the bottom edge, and the list scrolls under the mini player the way Apple's does.
            // Screens keep the last row reachable by adding LocalChromeInset to their content padding.
            var chromeHeight by remember { mutableStateOf(0.dp) }
            var tabsHeight by remember { mutableStateOf(0.dp) }
            // On its side the tabs stand on a rail at the right edge (TabRail), and the page gives it that edge;
            // the left edge is kept clear of the camera's punch hole.
            // The status bar put away where the setting says (the core's `status_bar_hidden`) - on its side out of
            // the box, where the screen is short and the page runs to the top edge. A swipe from the edge brings it
            // back for a moment, as in any full screen app.
            val barsView = androidx.compose.ui.platform.LocalView.current
            // The screen kept on where the setting says (the core's `keep_awake`), by the window's own flag, which
            // the lyrics' switch (on the view) neither sets nor clears. The charger is only watched for a setting
            // that asks about it, and only while the app is open: otherwise nothing is registered.
            val charging = rememberCharging(dev.nori.music.ffi.settings.keepAwakeWatchesCharging(prefs.keepAwake))
            val awake = remember(prefs.keepAwake, wide, charging) { dev.nori.music.ffi.settings.keepAwake(prefs.keepAwake, wide, charging) }
            androidx.compose.runtime.DisposableEffect(awake) {
                val window = (barsView.context as? android.app.Activity)?.window
                if (awake) window?.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                onDispose { window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
            }
            // Burn-in protection while nori keeps the screen on: everything drifts a few dp once a minute (the
            // core's `burn_in_drift`) and after a while untouched the screen dims. Either bar can be put away
            // while it is at work (the core's `bar_hidden`).
            val protecting = prefs.burnInProtection && (awake || lyricsKeepOn.value)
            val hideBar = remember(prefs.hideStatusBar, wide, protecting) { dev.nori.music.ffi.settings.barHidden(prefs.hideStatusBar, wide, protecting) }
            // From Android 11, where the bar's room reads the same while it is away (gestureBarRoom).
            val hideGestureBar = remember(prefs.hideGestureBar, wide, protecting) {
                android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R && dev.nori.music.ffi.settings.barHidden(prefs.hideGestureBar, wide, protecting)
            }
            val bars = remember(hideBar, hideGestureBar) { HiddenBars(hideBar, hideGestureBar) }
            androidx.compose.runtime.SideEffect { hiddenBars.value = bars }
            LaunchedEffect(bars) {
                val window = (barsView.context as? android.app.Activity)?.window ?: return@LaunchedEffect
                bars.applyTo(window, barsView)
            }
            val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
            // Read only where the layer and the dimming draw, so a move or a dim redraws and recomposes nothing.
            var drift by remember { mutableStateOf<dev.nori.music.ffi.look.Drift?>(null) }
            LaunchedEffect(protecting) {
                if (!protecting) { drift = null; return@LaunchedEffect }
                val since = System.currentTimeMillis()
                lifecycle.repeatOnLifecycle(androidx.lifecycle.Lifecycle.State.STARTED) {
                    while (true) {
                        val now = System.currentTimeMillis()
                        drift = dev.nori.music.ffi.look.drift(now, since)
                        kotlinx.coroutines.delay(dev.nori.music.ffi.look.nextMoveMs(now))
                    }
                }
            }
            val touches = remember { kotlinx.coroutines.flow.MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST) }
            var dim by remember { androidx.compose.runtime.mutableFloatStateOf(0f) }
            LaunchedEffect(protecting, prefs.dimWhenIdle, prefs.dimLevel) {
                dim = 0f
                val after = dev.nori.music.ffi.settings.dimAfterMs(prefs.dimWhenIdle)
                if (!protecting || after == null) return@LaunchedEffect
                val amount = dev.nori.music.ffi.settings.dimAmount(prefs.dimLevel)
                lifecycle.repeatOnLifecycle(androidx.lifecycle.Lifecycle.State.STARTED) {
                    touches.onStart { emit(Unit) }.collectLatest { dim = 0f; kotlinx.coroutines.delay(after); dim = amount }
                }
            }
            // Which way round: the edge the bar stood on is on the right turned one way, on the left the other
            // (TabRail), and the camera on the opposite side.
            val view = androidx.compose.ui.platform.LocalView.current
            val rotation = remember(config) { view.display?.rotation ?: android.view.Surface.ROTATION_0 }
            // On a car's own screen the controls and the tabs go beside the driver (DriverSide), or on the side
            // chosen in Settings (the core's `controls_on_left`). A phone keeps its layout.
            val car = remember(context) { DriverSide.isCar(context) }
            val driverLeft = remember(context) { DriverSide.onLeft(context) }
            val coverAtEnd = wide && car && dev.nori.music.ffi.settings.controlsOnLeft(prefs.controlsSide, driverLeft)
            val railLeft = wide && (rotation == android.view.Surface.ROTATION_270 || coverAtEnd)
            val railInset = if (wide) tabRailWidth() else 0.dp
            val cutout = androidx.compose.foundation.layout.WindowInsets.displayCutout.asPaddingValues()
            val cutoutStart = if (wide && !railLeft) cutout.calculateStartPadding(androidx.compose.ui.unit.LayoutDirection.Ltr) else 0.dp
            val cutoutEnd = if (wide && railLeft) cutout.calculateEndPadding(androidx.compose.ui.unit.LayoutDirection.Ltr) else 0.dp
            val pageStart = if (railLeft) railInset else cutoutStart
            val pageEnd = if (railLeft) cutoutEnd else railInset
            // A turn of the phone: the system dissolves the old layout into the new (MainActivity), the bar is laid
            // out again where it already lay, and its glyphs turn upright from where they were. Nothing here fades
            // the page itself: a page brought up from nothing under the dissolve was a flash of black.
            val turn = remember { androidx.compose.animation.core.Animatable(0f) }
            val lastRotation = remember { intArrayOf(rotation) }
            LaunchedEffect(rotation) {
                val steps = ((rotation - lastRotation[0]) % 4 + 4) % 4
                lastRotation[0] = rotation
                if (steps == 0) return@LaunchedEffect
                turn.snapTo(when (steps) { 1 -> -90f; 3 -> 90f; else -> 180f })
                if (AppMotion.reduce) turn.snapTo(0f) else turn.animateTo(0f, androidx.compose.animation.core.tween(360, easing = androidx.compose.animation.core.FastOutSlowInEasing))
            }
            val density = androidx.compose.ui.platform.LocalDensity.current
            // One look for both halves of the chrome, cross-fading once when the page under it changes.
            val chromeLook = rememberChromeLook()
            // This replaced a Scaffold when the chrome started floating, and with it went the two things
            // Scaffold quietly provided: something that paints the app's background (every screen was
            // showing the window's default grey, lighter than our own cards) and a content colour for
            // text that does not name one (which left titles rendering almost black).
            Surface(color = MaterialTheme.colorScheme.background, contentColor = MaterialTheme.colorScheme.onBackground) {
            Box(Modifier.fillMaxSize().onGloballyPositioned { sheet.rootHeight = it.size.height.toFloat() }.then(if (protecting) Modifier.burnInProtection(touches, { drift }, { dim }) else Modifier)) {
              // Everything under the player. Once the player covers it completely it is not drawn at all (a
              // layer at zero alpha is skipped), and what ticks on the page stops (LocalPageCovered).
              val covered by remember(sheet) { androidx.compose.runtime.derivedStateOf { sheet.progress.value >= 1f } }
              CompositionLocalProvider(LocalWide provides wide, LocalCoverAtEnd provides coverAtEnd, LocalTabTurn provides { turn.value }, LocalPageStart provides pageStart, LocalPageEnd provides pageEnd) {
              CompositionLocalProvider(LocalPageCovered provides covered) {
              Box(Modifier.fillMaxSize().graphicsLayer { alpha = if (covered) 0f else 1f }) {
              // The strips the rail and the camera stand on are the app's own page; a tinted page paints them over
              // itself (HeroPage), so its colour comes and goes with the page. Painted here in the chrome's
              // colour, which follows a page on a slower fade of its own, they held the album's colour at the
              // screen's edges while the page slid away.
              if (wide) {
                  val bg = MaterialTheme.colorScheme.background
                  val strip = Modifier.fillMaxHeight().drawBehind { drawRect(bg) }
                  Box(strip.width(pageStart).align(Alignment.CenterStart))
                  Box(strip.width(pageEnd).align(Alignment.CenterEnd))
              }
              CompositionLocalProvider(LocalStarMarks provides marks, LocalChromeInset provides chromeHeight) {
              val pageBackground = MaterialTheme.colorScheme.background
              Box(
                  Modifier.fillMaxSize().padding(start = pageStart, end = pageEnd)
                      .onSizeChanged { swipe.width = it.width.toFloat() }
                      // The page a swipe back brings, under the one it takes away (PageSwipe).
                      .drawBehind { drawUnder(swipe, pageBackground) },
              ) {
                // One transition for the whole app, and the same one in both directions. See PageMotion.
                val plain = reduceMotion()
                NavHost(
                    controller, home,
                    enterTransition = { PageMotion.enter(this, plain) },
                    exitTransition = { PageMotion.exit(this, plain) },
                    popEnterTransition = { PageMotion.popEnter(this, plain) },
                    popExitTransition = { PageMotion.popExit(this, plain) },
                    // The back gesture: the same slide, held under the finger, from either edge. See PageMotion.
                    predictivePopEnterTransition = { edge -> PageMotion.heldUnder(this, plain, edge) },
                    predictivePopExitTransition = { edge -> PageMotion.held(this, plain, edge) },
                ) {
                    page("home") { HomeScreen(actions) }
                    page("search") { SearchScreen(actions) }
                    page("library") { LibraryScreen(actions) }
                    page("settings") { SettingsScreen(settings) }
                    page("settings/{id}?key={key}") { e ->
                        SettingsGroupScreen(settings, e.arguments!!.getString("id")!!, e.arguments?.getString("key").orEmpty(), actions)
                    }
                    page("equalizer") { EqualizerScreen(settings) }
                    page("home-rows") { HomeRows(settings) }
                    page("library-sections") { LibrarySections(settings) }
                    page("library/{section}") { LibrarySectionScreen(dev.nori.music.ffi.settings.LibrarySection.valueOf(it.arguments!!.getString("section")!!), actions) }
                    page("autoeq") { AutoEqScreen(settings) }
                    page("album/{id}", inset = false) { AlbumScreen(it.arguments!!.getString("id")!!, actions) }
                    page("downloaded/{id}", inset = false) {
                        AlbumScreen(it.arguments!!.getString("id")!!, actions, viewModel<dev.nori.music.app.vm.DownloadedAlbumViewModel>())
                    }
                    page("artist/{id}", inset = false) { ArtistScreen(it.arguments!!.getString("id")!!, actions) }
                    page("playlist/{id}", inset = false) { PlaylistScreen(it.arguments!!.getString("id")!!, actions) }
                    page("mix/{id}", inset = false) { MixScreen(it.arguments!!.getString("id")!!, actions) }
                    page("genre/{id}", inset = false) { GenreScreen(it.arguments!!.getString("id")!!, actions) }
                    page("smart/{id}", inset = false) { SmartScreen(it.arguments!!.getString("id")!!, actions) }
                    page("smartEdit/{id}") { SmartEditScreen(it.arguments!!.getString("id")!!.let { i -> if (i == "new") "" else i }) }
                    page("stats") { StatsScreen() }
                    page("perf") { dev.nori.music.app.PerfHooks.recorder?.Page() }
                    page("downloads") { DownloadsScreen(actions) }
                    page("folder/{id}", inset = false) { FolderScreen(it.arguments!!.getString("id")!!, actions) }
                    page("decade/{year}") { SongsScreen(actions, it.arguments!!.getString("year")!!.toInt()) }
                }
                // A newer release, and its download and install once asked for; under the player, so the
                // player covers it. See UpdateBanner.
                UpdateBanner(settings, Modifier.align(Alignment.TopCenter))
                ChangelogDialog(settings)
              }
              }
              Box(
                  Modifier.align(Alignment.BottomCenter).padding(start = pageStart, end = pageEnd)
                      .onGloballyPositioned { chromeHeight = with(density) { it.size.height.toDp() } },
              ) {
                  // The now playing bar has a heart now, and it reads the stars the user has just
                  // changed from here like every other one. Outside this, it saw only the server's
                  // answer, so a tap on it changed nothing until the song came round again.
                  CompositionLocalProvider(LocalStarMarks provides marks) { BottomChrome(player, actions, nav::player, if (wide) 0.dp else tabsHeight, chromeLook) }
              }
              }
              }
              PlayerLayer(sheet) { CompositionLocalProvider(LocalStarMarks provides marks) { PlayerScreen(player, actions) } }
              // The tab bar is over the player, not under it: as the player rises it slides down off the
              // screen instead of vanishing under the sheet in one frame. See BottomChrome.
              if (wide) Box(Modifier.align(if (railLeft) Alignment.CenterStart else Alignment.CenterEnd).fillMaxHeight()) {
                  TabRail(tabRoute, tabs, nav::tab, chromeLook, railLeft)
              }
              else Box(Modifier.align(Alignment.BottomCenter)) { TabBar(tabRoute, tabs, nav::tab, chromeLook) { tabsHeight = it } }
              }
              // Top: less in the way of the now-playing bar; swipe or the X dismisses.
              SnackbarHost(
                  snackbar,
                  Modifier.align(Alignment.TopCenter).windowInsetsPadding(statusBarRoom).padding(top = 8.dp, start = 12.dp, end = 12.dp),
              ) { data ->
                  val dismiss = androidx.compose.material3.rememberSwipeToDismissBoxState(
                      confirmValueChange = {
                          if (it != androidx.compose.material3.SwipeToDismissBoxValue.Settled) {
                              data.dismiss()
                              true
                          } else false
                      },
                  )
                  androidx.compose.material3.SwipeToDismissBox(
                      state = dismiss,
                      backgroundContent = {},
                      enableDismissFromStartToEnd = true,
                      enableDismissFromEndToStart = true,
                  ) {
                      val v = data.visuals
                      Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                          MessagePill(
                              v.message, chromeLook, Modifier.widthIn(max = 560.dp), maxLines = 3,
                              action = v.actionLabel, onAction = data::performAction,
                              onDismiss = if (v.withDismissAction) data::dismiss else null,
                          )
                      }
                  }
              }
            }
            EdgeBack(swipe, sheet, controller)
            SheetBack(sheet)
            SelectionBack(actions, sheet, controller)
            }
            SongMenu(menuSong, actions, onDismiss = { menuSong = null }, player = player.takeIf { menuFromPlayer })
            DevicesHost(devicesOpen) { devicesOpen = false }
            // A jam that would not start says so, whichever menu started it.
            if (prefs.remoteControl && rules.remote) JamFailures(actions)
        }
        }
    }
}

/**
 * Back puts the player away while it is up. Registered after the NavHost's own, so it goes first; and
 * its own small scope, so the sheet opening and closing recomposes this and not the whole app.
 */
@Composable
private fun SheetBack(sheet: PlayerSheet) {
    // With the back gesture the sheet sinks a little with the finger, the way a page does, and goes
    // on down if the gesture is let go, or back up if it is called off.
    androidx.activity.compose.PredictiveBackHandler(sheet.isOpen) { events ->
        try {
            events.collect { sheet.backBy(it.progress) }
            sheet.backClose()
        } catch (e: kotlinx.coroutines.CancellationException) {
            sheet.open()
            throw e
        }
    }
}

/**
 * Back while songs are selected lets go of them and stays on the page; registered after the NavHost's
 * own, so it goes first. Not while the player is up: back puts the player away first. And any change of
 * page - back, a tap, a tab, a link - ends the selection, so the bar never outlives the list it was for.
 */
@Composable
private fun SelectionBack(actions: ActionsViewModel, sheet: PlayerSheet, controller: NavHostController) {
    val selecting by remember(actions) { actions.selection.map { it.isNotEmpty() }.distinctUntilChanged() }.collectAsState(actions.selection.value.isNotEmpty())
    androidx.activity.compose.BackHandler(selecting && !sheet.isOpen) { actions.backFromSelection() }
    LaunchedEffect(controller, actions) { controller.currentBackStackEntryFlow.collect { actions.onPage(it.id) } }
}

/**
 * One route of the app: its content, wrapped so that it takes part in [PageMotion]. [inset] puts plain
 * screens below the status bar; album, artist and playlist pages draw under it.
 */
private fun androidx.navigation.NavGraphBuilder.page(
    route: String, inset: Boolean = true,
    content: @Composable (androidx.navigation.NavBackStackEntry) -> Unit,
) = composable(route) { entry -> Page(entry) { if (inset) Inset { content(entry) } else content(entry) } }

/**
 * The pages are a stack, and move the way iOS pushes one: a page opened slides in from the right edge,
 * opaque, over the page that opened it, which slides a third of the way off to the left and darkens, so
 * it is plainly still there, behind. Going back is the same in reverse, and is [PageSwipe]'s: held under
 * the finger, or run from rest by the back button or key. The transitions here are what the stack does
 * when [PageSwipe] cannot take a page back itself (a page deeper than its pictures) - and nothing at all
 * after it has ([instant]).
 *
 * A tap on a tab has no stack - Home and Library are siblings - so that change is a plain, short
 * cross-fade with no direction (Apple Music does not animate it at all).
 *
 * Earlier versions, and why they went: a full-width slide scrubbed by Android's back gesture trailed the
 * finger (the system smooths the gesture by about 130 ms) and finished at one pace however it was flicked;
 * a shrinking card for the gesture alone made back two different motions; a short shared-axis drift with
 * a fade read as cheap and was still blind to a flick.
 */
internal object PageMotion {
    /** How long a push, or a pop the stack runs itself, takes: the speed the app always had. */
    private const val MS = 200
    /** The back gesture's slide when the stack scrubs it itself. */
    private const val HELD_MS = 250
    /** A tab change: a cross-fade this long. */
    private const val TAB_MS = 100

    /** Sharp settle: travel early, a short ease into place. */
    val Settle = androidx.compose.animation.core.CubicBezierEasing(0.2f, 0f, 0f, 1f)

    /**
     * The tab last tapped. Only a tap on a tab cross-fades: going back from one tab's page to another's
     * (the tabs sit on Home in the stack) is a back like any other.
     */
    var tabbedTo: String? = null

    /** The pop under way has been shown by [PageSwipe] already: the stack changes with no motion. */
    var instant = false

    /**
     * True while the change under way is a pop, false for a push, null for a tab change or none. Read by
     * each page in flight: the page coming in on a push and the page leaving on a pop are on top; the
     * other is underneath and darkens.
     */
    var pop: Boolean? by mutableStateOf(null)
        private set

    private fun androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>.tab(): Boolean =
        initialState.destination.route in PageSwipe.ROOTS && targetState.destination.route.let { it in PageSwipe.ROOTS && it == tabbedTo }

    private fun androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>.begin(pop: Boolean): Boolean {
        val tab = tab()
        PageMotion.pop = if (tab) null else pop
        return tab
    }

    private val travel = androidx.compose.animation.core.tween<androidx.compose.ui.unit.IntOffset>(MS, easing = Settle)
    private fun heldTravel() = androidx.compose.animation.core.tween<androidx.compose.ui.unit.IntOffset>(HELD_MS, easing = androidx.compose.animation.core.LinearEasing)
    private fun side(edge: Int) = if (edge == androidx.activity.BackEventCompat.EDGE_RIGHT) -1 else 1

    private fun fadeIn(ms: Int) = androidx.compose.animation.fadeIn(androidx.compose.animation.core.tween(ms))
    private fun fadeOut(ms: Int) = androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(ms))

    fun enter(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean): androidx.compose.animation.EnterTransition {
        if (s.begin(pop = false)) return fadeIn(if (plain) 90 else TAB_MS)
        if (plain) return fadeIn(90)
        return androidx.compose.animation.slideInHorizontally(travel) { it }
    }

    fun exit(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean): androidx.compose.animation.ExitTransition {
        if (s.begin(pop = false)) return fadeOut(if (plain) 90 else TAB_MS)
        if (plain) return fadeOut(90)
        return androidx.compose.animation.slideOutHorizontally(travel) { -it / PageSwipe.UNDER }
    }

    fun popEnter(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean): androidx.compose.animation.EnterTransition {
        if (instant) { pop = null; return androidx.compose.animation.EnterTransition.None }
        if (s.begin(pop = true)) return fadeIn(if (plain) 90 else TAB_MS)
        if (plain) return fadeIn(90)
        return androidx.compose.animation.slideInHorizontally(travel) { -it / PageSwipe.UNDER }
    }

    fun popExit(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean): androidx.compose.animation.ExitTransition {
        if (instant) { pop = null; return androidx.compose.animation.ExitTransition.None }
        if (s.begin(pop = true)) return fadeOut(if (plain) 90 else TAB_MS)
        if (plain) return fadeOut(90)
        return androidx.compose.animation.slideOutHorizontally(travel) { it }
    }

    /** The page coming back under the back gesture when the stack scrubs it, from the side the swipe started on. */
    fun heldUnder(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean, edge: Int): androidx.compose.animation.EnterTransition {
        if (s.begin(pop = true)) return fadeIn(if (plain) 90 else TAB_MS)
        if (plain) return fadeIn(90)
        return androidx.compose.animation.slideInHorizontally(heldTravel()) { -side(edge) * it / PageSwipe.UNDER }
    }

    /** The page the back gesture holds when the stack scrubs it, going the finger's way. */
    fun held(s: androidx.compose.animation.AnimatedContentTransitionScope<androidx.navigation.NavBackStackEntry>, plain: Boolean, edge: Int): androidx.compose.animation.ExitTransition {
        if (s.begin(pop = true)) return fadeOut(if (plain) 90 else TAB_MS)
        if (plain) return fadeOut(90)
        return androidx.compose.animation.slideOutHorizontally(heldTravel()) { side(edge) * it }
    }

    /** How dark a page in [state] is: only the page underneath, covered on a push or uncovered on a pop. */
    fun dim(state: androidx.compose.animation.EnterExitState, pop: Boolean?): Float = when {
        pop == null || state == androidx.compose.animation.EnterExitState.Visible -> 0f
        state == androidx.compose.animation.EnterExitState.PostExit && !pop -> PageSwipe.SCRIM
        state == androidx.compose.animation.EnterExitState.PreEnter && pop -> PageSwipe.SCRIM
        else -> 0f
    }

    fun dimSpec(plain: Boolean): androidx.compose.animation.core.FiniteAnimationSpec<Float> =
        if (plain) androidx.compose.animation.core.tween(90) else androidx.compose.animation.core.tween(MS, easing = Settle)

    /** The shadow the page on top casts on the one under it while it moves. */
    val EDGE_SHADOW = 16.dp
}

/**
 * A page as [PageMotion] and [PageSwipe] need it. Opaque while it moves - the screens paint no background,
 * the app's one Surface does - and darkened when it is the one underneath. Drawn through a layer of its
 * own so a picture of it can be kept as a page opened over it covers it: what a swipe back shows under
 * the page on top. The page a swipe is taking away wears the swipe's offset and a shadow on its edge, and
 * is what the swipe on the page itself starts from.
 */
@Composable
private fun androidx.compose.animation.AnimatedVisibilityScope.Page(entry: androidx.navigation.NavBackStackEntry, content: @Composable () -> Unit) {
    val plain = reduceMotion()
    val swipe = LocalPageSwipe.current!!
    val dim by transition.animateFloat(transitionSpec = { PageMotion.dimSpec(plain) }, label = "dim") { state -> PageMotion.dim(state, PageMotion.pop) }
    val background = MaterialTheme.colorScheme.background
    val layer = androidx.compose.ui.graphics.rememberGraphicsLayer()
    val moving = transition.currentState != transition.targetState
    // Covered by a page opened over it, or by a tab tapped (the tabs sit on Home): pictured now, as it was last drawn.
    val covered = transition.targetState == androidx.compose.animation.EnterExitState.PostExit && PageMotion.pop != true
    LaunchedEffect(covered) { if (covered) swipe.keep(entry.id, layer.toImageBitmap()) }
    androidx.compose.runtime.DisposableEffect(entry.id) { onDispose { swipe.left(entry.id) } }
    val swiped = swipe.moving == entry.id
    val onTop = swiped || moving && !plain && when (PageMotion.pop) {
        true -> transition.targetState == androidx.compose.animation.EnterExitState.PostExit
        false -> transition.targetState == androidx.compose.animation.EnterExitState.Visible
        null -> false
    }
    val top = entry.destination.route !in PageSwipe.ROOTS && transition.targetState == androidx.compose.animation.EnterExitState.Visible
    Box(
        Modifier.fillMaxSize()
            .graphicsLayer {
                translationX = if (swiped) swipe.offset else 0f
                shadowElevation = if (onTop) PageMotion.EDGE_SHADOW.toPx() else 0f
            }
            .drawBehind { if (moving || swiped) drawRect(background) }
            .drawWithContent {
                layer.record { this@drawWithContent.drawContent() }
                drawLayer(layer)
                if (dim > 0.002f) drawRect(androidx.compose.ui.graphics.Color.Black.copy(alpha = dim))
            }
            .swipeBack(swipe, enabled = top),
    ) { content() }
}

/**
 * Android's back gesture, from either edge, and the back key, taken by [PageSwipe] while it can take the
 * page back itself: the page follows the gesture's raw finger position (its progress is smoothed), and goes
 * at the speed measured from it. Registered after the stack's own, so it is asked first; the player's and
 * the selection's, registered after it, come before it while they are up.
 */
@Composable
private fun EdgeBack(swipe: PageSwipe, sheet: PlayerSheet, controller: androidx.navigation.NavHostController) {
    // Read so it is asked again as the stack changes.
    controller.currentBackStackEntryAsState().value
    androidx.activity.compose.PredictiveBackHandler(!sheet.isOpen && swipe.canGoBack(edge = true)) { events ->
        if (!swipe.begin(edge = true)) {
            // The gesture's events have to be taken whatever happens (the handler throws otherwise): a
            // second quick swipe, while the first page is still leaving, is a back of its own, run after it.
            events.collect {}
            swipe.back()
            return@PredictiveBackHandler
        }
        val speed = EdgeSpeed()
        var start = Float.NaN
        var side = 1f
        try {
            events.collect { e ->
                if (start.isNaN()) {
                    start = e.touchX
                    side = if (e.swipeEdge == androidx.activity.BackEventCompat.EDGE_RIGHT) -1f else 1f
                }
                val x = e.touchX - start
                swipe.dragTo(if (side > 0f) x.coerceAtLeast(0f) else x.coerceAtMost(0f))
                speed.add(e.touchX)
            }
            // A gesture the system took as back goes at a flick's speed at least; the key, with no gesture, eases.
            swipe.release(if (start.isNaN()) 0f else speed.velocity(), commit = true, flung = !start.isNaN())
        } catch (e: kotlinx.coroutines.CancellationException) {
            swipe.release(0f, commit = false)
            throw e
        }
    }
}

/**
 * The player sheet over everything else, present only while it is open or moving. The page behind
 * darkens as it rises; the sheet's top corners round off while it is in flight and square up once it
 * fills the screen. All of it is read in the draw phase, so a drag redraws two layers and recomposes
 * nothing.
 */
@Composable
private fun PlayerLayer(sheet: PlayerSheet, content: @Composable () -> Unit) {
    val shown by remember { androidx.compose.runtime.derivedStateOf { sheet.progress.value > 0f || sheet.progress.targetValue > 0f } }
    // Built once the app's first frame is drawn and the main thread has nothing else to do, and kept:
    // opening the player then only moves it. An open before that builds it.
    var warm by remember { mutableStateOf(false) }
    val view = androidx.compose.ui.platform.LocalView.current
    androidx.compose.runtime.DisposableEffect(view) {
        val idle = android.os.MessageQueue.IdleHandler { warm = true; false }
        val queue = android.os.Looper.myQueue()
        view.post { queue.addIdleHandler(idle) }
        onDispose { queue.removeIdleHandler(idle) }
    }
    if (!shown && !warm) return
    val radius = with(androidx.compose.ui.platform.LocalDensity.current) { 14.dp.toPx() }
    if (shown) Box(
        Modifier.fillMaxSize().drawBehind {
            drawRect(androidx.compose.ui.graphics.Color.Black.copy(alpha = 0.45f * sheet.progress.value))
        },
    )
    Box(
        Modifier.fillMaxSize().graphicsLayer {
            // Put away, it is parked a whole screen below the bottom edge: nothing drawn, and nothing to
            // catch a touch meant for the mini player (a transparent layer would still be hit).
            val parked = sheet.progress.value == 0f && sheet.progress.targetValue == 0f
            translationY = if (parked) sheet.rootHeight * 2f + 1f else sheet.offset()
            // It comes up *through* the now playing bar rather than on top of it: for the first tenth of
            // the rise the player is still part transparent, so the bar shows through its own place in
            // it. At full strength on the first frame of a drag, a whole dark page would appear out of
            // nothing before it had moved anywhere.
            alpha = if (parked) 0f else (sheet.progress.value / 0.1f).coerceIn(0f, 1f)
            val r = radius * (1f - sheet.progress.value).coerceIn(0f, 1f) * 4f
            shape = androidx.compose.foundation.shape.RoundedCornerShape(topStart = r.coerceAtMost(radius), topEnd = r.coerceAtMost(radius))
            clip = true
        }
            // The player is a page, not a pane of glass. Only its buttons and gestures listened, so a
            // tap anywhere else on it - between the lyrics, beside the artwork - fell through to what
            // is underneath: the list the player was opened from, playing a song nobody could see.
            // Being hit at all is enough to keep a touch here; nothing is consumed, so every control
            // inside works as before.
            .pointerInput(Unit) { awaitPointerEventScope { while (true) awaitPointerEvent() } },
    ) {
        CompositionLocalProvider(LocalChromeInset provides 0.dp, LocalPlayerShown provides shown) { content() }
    }
}

/**
 * Whether the phone is charging, kept up to date from the system's power broadcasts while [watch] is true;
 * false, and nothing registered, while it is not.
 */
@Composable
private fun rememberCharging(watch: Boolean): Boolean {
    val context = androidx.compose.ui.platform.LocalContext.current
    var charging by remember { mutableStateOf(false) }
    androidx.compose.runtime.DisposableEffect(watch) {
        if (!watch) { charging = false; return@DisposableEffect onDispose {} }
        // The battery's last state is sticky, so it is read on registering; the plug's two broadcasts then move it.
        val now = context.registerReceiver(null, android.content.IntentFilter(android.content.Intent.ACTION_BATTERY_CHANGED))
        charging = (now?.getIntExtra(android.os.BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
        val receiver = object : android.content.BroadcastReceiver() {
            override fun onReceive(c: android.content.Context, intent: android.content.Intent) {
                charging = intent.action == android.content.Intent.ACTION_POWER_CONNECTED
            }
        }
        val filter = android.content.IntentFilter().apply {
            addAction(android.content.Intent.ACTION_POWER_CONNECTED)
            addAction(android.content.Intent.ACTION_POWER_DISCONNECTED)
        }
        androidx.core.content.ContextCompat.registerReceiver(context, receiver, filter, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED)
        onDispose { context.unregisterReceiver(receiver) }
    }
    return charging
}

/**
 * Burn-in protection over the whole app: what is drawn moves by [drift] and is enlarged by its reach on each
 * side, so no edge comes into view; [dim] darkens it, and every touch or lift is told to [touches] without
 * being taken from what it lands on.
 */
private fun Modifier.burnInProtection(
    touches: kotlinx.coroutines.flow.MutableSharedFlow<Unit>,
    drift: () -> dev.nori.music.ffi.look.Drift?,
    dim: () -> Float,
): Modifier = this
    .pointerInput(touches) {
        awaitPointerEventScope {
            while (true) {
                val e = awaitPointerEvent(androidx.compose.ui.input.pointer.PointerEventPass.Initial)
                if (e.type == androidx.compose.ui.input.pointer.PointerEventType.Press || e.type == androidx.compose.ui.input.pointer.PointerEventType.Release) touches.tryEmit(Unit)
            }
        }
    }
    .drawWithContent {
        drawContent()
        val d = dim()
        if (d > 0f) drawRect(Color.Black, alpha = d)
    }
    .graphicsLayer {
        val d = drift() ?: return@graphicsLayer
        val reach = d.reachDp.dp.toPx()
        scaleX = 1f + 2f * reach / size.width
        scaleY = 1f + 2f * reach / size.height
        translationX = d.xDp.dp.toPx()
        translationY = d.yDp.dp.toPx()
    }

/** Songs asked for while on mobile data download only after a yes. */
@Composable
private fun MobileDownloadQuestion(actions: ActionsViewModel) {
    val songs by actions.mobileDownload.collectAsStateWithLifecycle()
    NoriDialog(songs, { actions.downloadOnMobileData(false) }) {
        AlertCard(
            title = { androidx.compose.material3.Text(words(dev.nori.music.app.R.string.download_mobile_title)) },
            text = { androidx.compose.material3.Text(words(dev.nori.music.app.R.string.download_mobile_text)) },
            confirmButton = { TextAction(words(dev.nori.music.app.R.string.download_mobile_yes), { actions.downloadOnMobileData(true) }) },
            dismissButton = { TextAction(words(dev.nori.music.app.R.string.cancel), { actions.downloadOnMobileData(false) }) },
        )
    }
}
