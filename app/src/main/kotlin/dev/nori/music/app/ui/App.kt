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
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.constrainHeight
import androidx.compose.ui.unit.constrainWidth
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.drawWithContent
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
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
private fun Inset(content: @Composable () -> Unit) {
    val room = statusBarRoom
    val density = androidx.compose.ui.platform.LocalDensity.current
    val top = with(density) { room.getTop(this).toDp() }
    val page = MaterialTheme.colorScheme.background
    val outStart = LocalPageStart.current
    val outEnd = LocalPageEnd.current
    // What scrolls up under the status bar ([underStatusBar]) fades out into the page's colour across the bar's
    // room, as a sideways row fades out under the strip beside it; below the room nothing is touched.
    androidx.compose.foundation.layout.Box(
        Modifier.drawWithCache {
            val h = top.toPx()
            val l = outStart.toPx()
            val fade = androidx.compose.ui.graphics.Brush.verticalGradient(
                0f to page, 0.4f to page.copy(alpha = 0.85f), 0.75f to page.copy(alpha = 0.35f), 1f to page.copy(alpha = 0f),
                startY = 0f, endY = h,
            )
            val area = androidx.compose.ui.geometry.Size(size.width + l + outEnd.toPx(), h)
            onDrawWithContent {
                drawContent()
                if (h > 0f) drawRect(fade, topLeft = androidx.compose.ui.geometry.Offset(-l, 0f), size = area)
            }
        }.windowInsetsPadding(room),
    ) { CompositionLocalProvider(LocalStatusRoom provides top) { content() } }
}

/**
 * Navigation as the screens see it; they never touch the NavController. Going anywhere puts the player
 * away first, the way an artist tapped on Apple's player drops it and opens the artist underneath.
 */
class Nav(private val c: NavHostController, private val sheet: PlayerSheet, private val pages: Pages) {
    fun go(route: String) { if (sheet.isOpen) sheet.close(); c.navigate(route) }

    /**
     * What the row that opened an album already knew about it - name, artist, cover - kept for the page
     * to draw its header from while the album itself is on the way. The page slides in (Pages)
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
    /** One shelf of an artist's page in full: [group] is its place among the page's shelves. */
    fun artistReleases(id: String, group: Int) = go("artist/${Uri.encode(id)}/releases/$group")
    fun playlist(id: String, hint: dev.nori.music.ffi.model.Playlist? = null) {
        if (hint != null) playlists[id] = hint
        go("playlist/${Uri.encode(id)}")
    }
    fun genre(name: String) = go("genre/${Uri.encode(name)}")
    fun folder(id: String) = go("folder/${Uri.encode(id)}")
    fun decade(year: Int) = go("decade/$year")
    fun smart(id: String) = go("smart/${Uri.encode(id)}")
    fun mix(id: String) = go("mix/${Uri.encode(id)}")
    /** The bigger view of a home row: a [HomeRow] name, `feed-<place>-<kind>` or the mixes. */
    fun homeRow(key: String) = go("home/${Uri.encode(key)}")
    fun smartEdit(id: String) = go("smartEdit/${Uri.encode(id.ifEmpty { "new" })}")
    fun stats() = go("stats")
    /** The download queue. Asked for again while it is showing (the notification tapped), it only puts the player away. */
    fun downloads() { if (c.currentDestination?.route == "downloads") { if (sheet.isOpen) sheet.close() } else go("downloads") }
    /** A settings group, optionally landing on one row of it (from the settings search). */
    fun settingsGroup(id: String, key: String = "") = go("settings/$id?key=${Uri.encode(key)}")
    /** Opens the player, on [panel] if one is asked for. */
    fun player(panel: Panel? = null) { sheet.panelAsked = panel; sheet.open() }
    fun equalizer() = go("equalizer")
    fun plugins() = go("plugins")
    fun plugin(id: String) = go("plugins/${Uri.encode(id)}")
    fun homeRows() = go("home-rows")
    fun librarySections() = go("library-sections")
    fun librarySection(s: dev.nori.music.ffi.settings.LibrarySection) = go("library/${s.name}")
    fun autoEq() = go("autoeq")
    fun back() { if (sheet.isOpen) sheet.close() else pages.back() }
    /**
     * A tab always lands on that tab's own page: nothing above the tab roots survives a tab tap. A saved
     * and restored stack would put an album straight back after Home popped it, and the tab would look dead.
     */
    fun tab(route: String) {
        if (sheet.isOpen) sheet.close()
        // Already there: nothing to go to. Navigating anyway ran the page's arrival again on every tap,
        // so a tab pressed twice redrew the page it was already on.
        if (c.currentBackStackEntry?.destination?.route == route) {
            // Search tapped again on its own page asks for the keyboard; the first tap only shows the page.
            // With launchSingleTop the page is not recomposed, so nothing else would notice the tap.
            if (route == "search") searchTaps.intValue++
            return
        }
        pages.tab()
        c.navigate(route) {
            popUpTo(c.graph.startDestinationId)
            launchSingleTop = true
        }
    }
}

/** Counts taps on the search tab; the search field takes focus again on each one. */
/** How long the chrome's glyphs take to stand upright after a turn of the phone: about as long as the system's dissolve. */
private const val TURN_MS = 400

private val searchTaps = androidx.compose.runtime.mutableIntStateOf(0)

@Composable
internal fun searchFocusKey(): Int = searchTaps.intValue

val LocalNav = staticCompositionLocalOf<Nav> { error("no nav") }
val LocalSongMenu = staticCompositionLocalOf<(Song) -> Unit> { {} }

/** The album or artist page [entry] shows, as the song menu is told it. */
private fun pageOf(entry: androidx.navigation.NavBackStackEntry?): dev.nori.music.ffi.model.PageOrigin? {
    val id = entry?.arguments?.getString("id") ?: return null
    return when (entry.destination.route) {
        "album/{id}" -> dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.ALBUM, id)
        "artist/{id}" -> dev.nori.music.ffi.model.PageOrigin(dev.nori.music.ffi.model.OriginKind.ARTIST, id)
        else -> null
    }
}

/** Opens an album's or playlist's card menu: play it, next or at the end of the queue. */
val LocalCardMenu = staticCompositionLocalOf<(dev.nori.music.app.vm.CardItem) -> Unit> { {} }

/** The player's own ⋯: the same song menu, with the playback-wide entries the player needs. */
val LocalPlayerMenu = staticCompositionLocalOf<(Song) -> Unit> { {} }

/** Opens the devices sheet (RemoteScreens.DevicesHost): where the music plays. */
val LocalDevices = staticCompositionLocalOf<() -> Unit> { {} }

/** A song's seal tapped: the like flipped, or, signed out where an account would keep it, the sign-in offer. */
val LocalSealTap = staticCompositionLocalOf<(dev.nori.music.ffi.model.Song, Boolean) -> Unit> { { _, _ -> } }

/** Whether songs show a seal: a like, or one that asks to sign in (`ProfileRules`). */
val dev.nori.music.ffi.library.ProfileRules.seals: Boolean get() = stars || starsAfterSignIn

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
    Tab("home", say.home, Iconoir.HomeSimple),
    Tab("library", say.library, Iconoir.AlbumList),
    Tab("settings", say.settings, Iconoir.Settings),
    Tab("search", say.search, Iconoir.Search),
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
    androidx.compose.runtime.SideEffect {
        AppMotion.reduce = prefs.motion != dev.nori.music.ffi.settings.Motion.FULL
        AppMotion.still = prefs.motion == dev.nori.music.ffi.settings.Motion.EINK
    }
    NoriTheme(prefs) {
        // A jam guest's profile is the same app over its host's library, started afresh on joining and on leaving.
        val guest = remember(prefs.activeServerId, prefs.server?.apiKey) { prefs.server?.apiKey?.let { dev.nori.music.ffi.remote.isGuestKey(it) } == true }
        val kind = prefs.server?.kind ?: dev.nori.music.ffi.settings.ServerKind.Subsonic
        // Sign-in fades to the app: the screens are different enough that a direction would invent a
        // relationship they do not have.
        androidx.compose.animation.Crossfade(
            targetState = Triple(prefs.loggedIn, guest, kind),
            animationSpec = AppMotion.fadeIn(),
            label = "session",
        ) { (loggedIn, guest, kind) ->
        if (!loggedIn) {
            LoginScreen(settings)
            return@Crossfade
        }
        // A provider account's name (its `user`) brings its library and hearts.
        val account = prefs.server?.user.orEmpty()
        val reported by settings.reportedRules.collectAsStateWithLifecycle()
        val rules = remember(guest, kind, account, reported) { settings.rules() }
        val home = "home"
        // A guest follows the host while the app is in sight: what plays, the queue, its requests.
        val remote: dev.nori.music.app.vm.RemoteViewModel = viewModel()
        if (guest) androidx.lifecycle.compose.LifecycleResumeEffect(Unit) { remote.watch(true); onPauseOrDispose { remote.watch(false) } }
        val asked = remote.asked.collectAsStateWithLifecycle()

        val pageNavigator = remember { PageNavigator() }
        val controller = rememberNavController(pageNavigator)
        val sheetScope = androidx.compose.runtime.rememberCoroutineScope()
        // Whether the player was open, kept with the rest of the saved state, so an app Android put back
        // together (stopped in the background, or a change of configuration as the phone is unlocked)
        // opens on the player it was left on, from its first frame: opened a frame later, the page under
        // it showed for that frame.
        var playerWasOpen by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
        val sheet = remember { PlayerSheet(sheetScope, startOpen = playerWasOpen) }
        // The player on its way up or down, or a cover in flight between panels, moves what islands draw.
        val sheetMoving by remember(sheet) { androidx.compose.runtime.derivedStateOf { sheet.progress.value > 0f && sheet.progress.value < 1f || sheet.panelFlight } }
        HoldIslands(sheetMoving)
        LaunchedEffect(sheet) {
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
        val pages = rememberPages(controller, pageNavigator)
        val nav = remember(controller) { Nav(controller, sheet, pages) }
        // A jam just joined opens on it: the player, turned to the queue with the jam over it.
        if (guest) {
            var arrived by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
            LaunchedEffect(Unit) { if (!arrived) { arrived = true; nav.player(Panel.QUEUE) } }
        }
        val actions: ActionsViewModel = viewModel()
        val player: PlayerViewModel = viewModel()
        val snackbar = remember { SnackbarHostState() }
        var menuSong by remember { mutableStateOf<Song?>(null) }
        var menuCard by remember { mutableStateOf<dev.nori.music.app.vm.CardItem?>(null) }
        var menuFromPlayer by remember { mutableStateOf(false) }
        var menuPage by remember { mutableStateOf<dev.nori.music.ffi.model.PageOrigin?>(null) }
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
        val hiddenBars = remember { mutableStateOf(HiddenBars(status = false)) }
        CompositionLocalProvider(
            LocalNav provides nav,
            LocalLyricsKeepOn provides lyricsKeepOn,
            LocalStatusRoomKept provides dev.nori.music.ffi.settings.barRoomKept(prefs.hideStatusBar, wide),
            LocalPages provides pages,
            LocalPlayerSheet provides sheet,
            LocalDownloadMarks provides rememberDownloadMarks(actions),
            LocalSongMenu provides { menuSong = it; menuFromPlayer = false; menuPage = pageOf(controller.currentBackStackEntry) },
            LocalCardMenu provides { menuCard = it },
            LocalPlayerMenu provides { menuSong = it; menuFromPlayer = true },
            LocalDevices provides { devicesOpen = true },
            LocalSealTap provides { s, on -> if (rules.stars) actions.star(s, on) else settings.offerSignIn() },
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
            // On its side the page is kept clear of the camera's punch hole.
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
            // core's `burn_in_drift`) and after a while untouched the screen dims. The status bar can be put away
            // while it is at work (the core's `bar_hidden`).
            val protecting = prefs.burnInProtection && (awake || lyricsKeepOn.value)
            val hideBar = remember(prefs.hideStatusBar, wide, protecting) { dev.nori.music.ffi.settings.barHidden(prefs.hideStatusBar, wide, protecting) }
            val bars = remember(hideBar) { HiddenBars(hideBar) }
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
            // On a car's own screen the controls and the tabs go beside the driver (DriverSide), or on the side
            // chosen in Settings (the core's `controls_on_left`). A phone keeps its layout.
            val car = remember(context) { DriverSide.isCar(context) }
            val driverLeft = remember(context) { DriverSide.onLeft(context) }
            val coverAtEnd = wide && car && dev.nori.music.ffi.settings.controlsOnLeft(prefs.controlsSide, driverLeft)
            // On its side the chrome stands on the edge the phone's bottom turned to (SideChrome): the right one
            // way round, the left the other, or beside a car's driver. The camera is on the opposite side.
            val rotation = remember(config) { barsView.display?.rotation ?: android.view.Surface.ROTATION_0 }
            val chromeLeft = wide && (rotation == android.view.Surface.ROTATION_270 || coverAtEnd)
            androidx.compose.runtime.SideEffect {
                sheet.edge = if (!wide) SheetEdge.BOTTOM else if (chromeLeft) SheetEdge.LEFT else SheetEdge.RIGHT
            }
            var sideRoom by remember { mutableStateOf(0.dp) }
            val cutout = androidx.compose.foundation.layout.WindowInsets.displayCutout.asPaddingValues()
            val pageStart = if (!wide) 0.dp else if (chromeLeft) sideRoom else cutout.calculateStartPadding(androidx.compose.ui.unit.LayoutDirection.Ltr)
            val pageEnd = if (!wide) 0.dp else if (chromeLeft) cutout.calculateEndPadding(androidx.compose.ui.unit.LayoutDirection.Ltr) else sideRoom
            // A turn of the phone: the system dissolves the old layout into the new (MainActivity), the chrome is
            // laid out again where it already lay, and its glyphs turn upright from where they were.
            val turn = remember { androidx.compose.animation.core.Animatable(0f) }
            val lastRotation = remember { intArrayOf(rotation) }
            LaunchedEffect(rotation) {
                val steps = ((rotation - lastRotation[0]) % 4 + 4) % 4
                lastRotation[0] = rotation
                if (steps == 0) return@LaunchedEffect
                turn.snapTo(when (steps) { 1 -> -90f; 3 -> 90f; else -> 180f })
                if (AppMotion.reduce) turn.snapTo(0f) else turn.animateTo(0f, androidx.compose.animation.core.tween(TURN_MS, easing = androidx.compose.animation.core.FastOutSlowInEasing))
            }
            val density = androidx.compose.ui.platform.LocalDensity.current
            // One look for the chrome, cross-fading once when the page under it changes.
            val chromeLook = rememberChromeLook()
            val backdrop = rememberBackdrop()
            // This replaced a Scaffold when the chrome started floating, and with it went the two things
            // Scaffold quietly provided: something that paints the app's background (every screen was
            // showing the window's default grey, lighter than our own cards) and a content colour for
            // text that does not name one (which left titles rendering almost black).
            Surface(color = MaterialTheme.colorScheme.background, contentColor = MaterialTheme.colorScheme.onBackground) {
            Box(Modifier.fillMaxSize().onGloballyPositioned { sheet.rootSize = it.size }.then(if (protecting) Modifier.burnInProtection(touches, { drift }, { dim }) else Modifier)) {
              // Everything under the player. Once the player covers it completely it is not drawn at all (a
              // layer at zero alpha is skipped), and what ticks on the page stops (LocalPageCovered).
              val covered by remember(sheet) { androidx.compose.runtime.derivedStateOf { sheet.progress.value >= 1f } }
              CompositionLocalProvider(LocalWide provides wide, LocalCoverAtEnd provides coverAtEnd, LocalPageStart provides pageStart, LocalPageEnd provides pageEnd) {
              CompositionLocalProvider(LocalPageCovered provides covered) {
              Box(Modifier.fillMaxSize().graphicsLayer { alpha = if (covered) 0f else 1f }) {
              // The strips the camera and the chrome stand in are the app's own page; a tinted page paints them over
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
              // The pages span the whole width and keep their content off the strips themselves (Page): the stack
              // clips to its own bounds while some of its changes run, and a page's colour and rows run on under the strips.
              Box(
                  Modifier.fillMaxSize()
                      // What the chrome blurs behind itself (Pill).
                      .drawsBackdrop(backdrop),
              ) {
                PageStack(controller, pages, home) {
                    page("home") { HomeScreen(actions) }
                    page("home/{row}") { HomeRowScreen(it.arguments!!.getString("row")!!, actions) }
                    page("search") { SearchScreen(actions) }
                    page("library") { LibraryScreen(actions) }
                    page("settings") { SettingsScreen(settings) }
                    page("settings/{id}?key={key}") { e ->
                        SettingsGroupScreen(settings, e.arguments!!.getString("id")!!, e.arguments?.getString("key").orEmpty(), actions)
                    }
                    page("equalizer") { EqualizerScreen(settings) }
                    page("plugins") { PluginsScreen(viewModel(), settings, LocalNav.current::plugin) }
                    page("plugins/{id}") { PluginPage(viewModel(), settings, it.arguments!!.getString("id")!!, LocalNav.current::back) }
                    page("home-rows") { HomeRows(settings) }
                    page("library-sections") { LibrarySections(settings) }
                    page("library/{section}") {
                        // A section with a chip is shown on the Library page; the places and smart playlists have a page.
                        val section = dev.nori.music.ffi.settings.LibrarySection.valueOf(it.arguments!!.getString("section")!!)
                        if (section.onLibraryPage()) LibraryScreen(actions, initial = section) else LibrarySectionScreen(section, actions)
                    }
                    page("autoeq") { AutoEqScreen(settings) }
                    page("album/{id}", inset = false) { AlbumScreen(it.arguments!!.getString("id")!!, actions) }
                    page("downloaded/{id}", inset = false) {
                        AlbumScreen(it.arguments!!.getString("id")!!, actions, viewModel<dev.nori.music.app.vm.DownloadedAlbumViewModel>())
                    }
                    page("artist/{id}", inset = false) { ArtistScreen(it.arguments!!.getString("id")!!, actions) }
                    page("artist/{id}/releases/{group}") { ArtistReleasesScreen(it.arguments!!.getString("id")!!, it.arguments!!.getString("group")!!.toInt()) }
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
                UpdateBanner(settings, Modifier.align(Alignment.TopCenter).padding(start = pageStart, end = pageEnd).coversPage())
                ChangelogDialog(settings)
              }
              }
              // Upright, what runs on under the bar fades into the page's colour across the bar's room, as a row on its
              // side does under the strip the chrome stands in (bleedsToEdges); the page's colour is the chrome's look's.
              if (!wide) Box(
                  Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(chromeHeight + EDGE_REACH).coversPage().drawBehind {
                      val reach = EDGE_REACH.toPx()
                      val fade = edgeFade(chromeLook.color(CoverLook.CHROME_PAGE), size.height - reach, reach)
                      drawRect(androidx.compose.ui.graphics.Brush.verticalGradient(*fade, startY = 0f, endY = size.height))
                  },
              )
              Box(
                  Modifier.align(Alignment.BottomCenter).padding(start = pageStart, end = pageEnd).coversPage()
                      .onGloballyPositioned { chromeHeight = with(density) { it.size.height.toDp() } }
                      .then(if (wide) Modifier else Modifier.keepsTouches()),
              ) {
                  // The now playing bar has a heart now, and it reads the stars the user has just
                  // changed from here like every other one. Outside this, it saw only the server's
                  // answer, so a tap on it changed nothing until the song came round again.
                  CompositionLocalProvider(LocalStarMarks provides marks, LocalBackdrop provides backdrop, LocalGlyphTurn provides { turn.value }) {
                      BottomChrome(player, actions, nav::player, tabRoute, tabs, nav::tab, chromeLook)
                  }
              }
              if (wide) CompositionLocalProvider(LocalStarMarks provides marks, LocalGlyphTurn provides { turn.value }) {
                  SideChrome(
                      player, actions, nav::player, tabRoute, tabs, nav::tab, chromeLook, chromeLeft,
                      Modifier.align(if (chromeLeft) Alignment.CenterStart else Alignment.CenterEnd).coversPage()
                          .onGloballyPositioned { sideRoom = with(density) { it.size.width.toDp() } }.keepsTouches(),
                  )
              }
              }
              }
              PlayerLayer(sheet) { CompositionLocalProvider(LocalStarMarks provides marks, LocalDrifting provides protecting) { PlayerScreen(player, actions) } }
              }
              // Top: less in the way of the now-playing bar; swipe or the X dismisses.
              SnackbarHost(
                  snackbar,
                  Modifier.align(Alignment.TopCenter).windowInsetsPadding(statusBarRoom).padding(top = 8.dp, start = 12.dp, end = 12.dp).coversPage(),
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
            EdgeBack(pages, sheet)
            SheetBack(sheet)
            SelectionBack(actions, sheet, controller)
            }
            SongMenu(menuSong, actions, onDismiss = { menuSong = null }, player = player.takeIf { menuFromPlayer }, page = menuPage.takeUnless { menuFromPlayer })
            CardMenu(menuCard, actions, onDismiss = { menuCard = null })
            DevicesHost(devicesOpen) { devicesOpen = false }
            // A jam that would not start says so, whichever menu started it.
            if (prefs.remoteControl && rules.remote) JamFailures(actions)
        }
        }
    }
}

/**
 * Back puts the player away while it is up. Registered after the pages' own (EdgeBack), so it goes first; and
 * its own small scope, so the sheet opening and closing recomposes this and not the whole app.
 */
@Composable
private fun SheetBack(sheet: PlayerSheet) {
    // With the back gesture the sheet sinks a little with the finger, the way a page does, and goes
    // on down if the gesture is let go, or back up if it is called off.
    androidx.activity.compose.PredictiveBackHandler(sheet.isOpen) { events ->
        try {
            events.collect { sheet.backBy(it.progress) }
            sheet.close()
        } catch (e: kotlinx.coroutines.CancellationException) {
            sheet.open()
            throw e
        }
    }
}

/**
 * Back while songs are selected lets go of them and stays on the page; registered after the pages'
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
 * One route of the app. [inset] puts plain screens below the status bar; album, artist and playlist pages
 * draw under it.
 */
private fun androidx.navigation.NavGraphBuilder.page(
    route: String, inset: Boolean = true,
    content: @Composable (androidx.navigation.NavBackStackEntry) -> Unit,
) = addPage(route) { entry -> if (inset) Inset { content(entry) } else content(entry) }

/**
 * The player sheet over everything else, present only while it is open or moving. The page behind
 * darkens as it comes in; the sheet's leading corners round off while it is in flight and square up once it
 * fills the screen. All of it is read in the draw phase, so a drag redraws two layers and recomposes
 * nothing.
 */
@Composable
private fun PlayerLayer(sheet: PlayerSheet, content: @Composable () -> Unit) {
    val shown by remember { androidx.compose.runtime.derivedStateOf { sheet.progress.value > 0f || sheet.progress.targetValue > 0f } }
    // Built once the app's first frame is drawn and the main thread has nothing else to do, and kept:
    // opening the player then only moves it. An open before that builds it.
    val warm = rememberIdle(Unit)
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
            val at = sheet.offset()
            translationX = if (parked) 0f else at.x
            translationY = if (parked) sheet.rootSize.height * 2f + 1f else at.y
            // It comes up *through* the now playing bar rather than on top of it: for the first tenth of
            // the rise the player is still part transparent, so the bar shows through its own place in
            // it. At full strength on the first frame of a drag, a whole dark page would appear out of
            // nothing before it had moved anywhere.
            alpha = if (parked) 0f else (sheet.progress.value / 0.1f).coerceIn(0f, 1f)
            // Its leading corners: the top ones, or on its side the two facing the page.
            val r = (radius * (1f - sheet.progress.value).coerceIn(0f, 1f) * 4f).coerceAtMost(radius)
            shape = when (sheet.edge) {
                SheetEdge.BOTTOM -> androidx.compose.foundation.shape.RoundedCornerShape(topStart = r, topEnd = r)
                SheetEdge.RIGHT -> androidx.compose.foundation.shape.RoundedCornerShape(topStart = r, bottomStart = r)
                SheetEdge.LEFT -> androidx.compose.foundation.shape.RoundedCornerShape(topEnd = r, bottomEnd = r)
            }
            // Open, it has no corners to cut, and what bleeds past the screen's edges (Modifier.bleed) stays.
            clip = sheet.progress.value < 1f
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

/** Burn-in protection moves the screen ([burnInProtection]): what runs to its edges reaches past them ([bleed]). */
val LocalDrifting = androidx.compose.runtime.compositionLocalOf { false }

/**
 * Laid out the core's burn-in reach past every side while the screen drifts ([LocalDrifting]), taking the room
 * it would otherwise: something drawn to the screen's edges never leaves one bare as the screen moves.
 */
@Composable
fun Modifier.bleed(): Modifier {
    if (!LocalDrifting.current) return this
    return layout { measurable, constraints ->
        val r = stage.burnInReachDp.dp.roundToPx()
        val grown = constraints.copy(
            minWidth = constraints.minWidth + 2 * r, maxWidth = if (constraints.hasBoundedWidth) constraints.maxWidth + 2 * r else constraints.maxWidth,
            minHeight = constraints.minHeight + 2 * r, maxHeight = if (constraints.hasBoundedHeight) constraints.maxHeight + 2 * r else constraints.maxHeight,
        )
        val p = measurable.measure(grown)
        layout(constraints.constrainWidth(p.width - 2 * r), constraints.constrainHeight(p.height - 2 * r)) { p.place(-r, -r) }
    }
}

/**
 * Burn-in protection over the whole app: what is drawn moves by [drift], a few dp, the app's own background
 * showing at the edge it leaves; [dim] darkens it, and every touch or lift is told to [touches] without
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
