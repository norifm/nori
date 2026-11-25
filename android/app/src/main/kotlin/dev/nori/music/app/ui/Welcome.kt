package dev.nori.music.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingRow
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.app.vm.SyncViewModel
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.SetupUi
import dev.nori.music.app.vm.backupWords
import dev.nori.music.ffi.WelcomeDo
import dev.nori.music.ffi.WelcomeEvent
import dev.nori.music.ffi.WelcomeFlow
import dev.nori.music.ffi.WelcomePage
import dev.nori.music.ffi.WelcomeSource
import dev.nori.music.ffi.settings.SetupAnswer
import dev.nori.music.ffi.settings.SetupStep
import kotlin.math.abs
import kotlinx.coroutines.launch
import kotlin.math.cos
import kotlin.math.sin

/**
 * The first run, while no profile is in, and over the app once one is until the last source is signed in:
 * where the music is, the picked plugins installing, then the sign-ins (the core's `WelcomeFlow`). One source
 * picked signs in on its own page; several are listed.
 */
@Composable
fun WelcomePages(vm: SettingsViewModel) {
    val plugins: dev.nori.music.app.vm.PluginsViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val flow by vm.welcome.collectAsStateWithLifecycle()
    if (flow.page == WelcomePage.SigningIn) {
        val one = remember(flow) { dev.nori.music.ffi.welcomeOne(flow) }
        if (one != null) LoginScreen(vm, onClose = { vm.welcomeOn(WelcomeEvent.Back) }, kind = one) else WelcomeSignIns(vm, plugins, flow)
    } else WelcomeSources(vm, plugins, flow)
}

/**
 * Where the music is: the welcome, then nori's own and the plugins' sources and those a repo offers, all on an
 * equal footing, any number picked. Continue installs the plugins picked (each row says while it installs)
 * and goes on to the sign-ins.
 */
@Composable
private fun WelcomeSources(vm: SettingsViewModel, plugins: dev.nori.music.app.vm.PluginsViewModel, flow: WelcomeFlow) {
    val providers by vm.providers.collectAsStateWithLifecycle()
    val offers by plugins.providerOffers.collectAsStateWithLifecycle()
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    androidx.compose.runtime.LaunchedEffect(Unit) { plugins.loadOffers() }
    val installing = (flow.page as? WelcomePage.Installing)?.left.orEmpty()
    val sources = remember(providers, offers) { dev.nori.music.ffi.welcomeSources(providers.map { it.kind }, offers.map { it.id }) }
    val next = {
        val then = vm.welcomeOn(WelcomeEvent.Continue)
        if (then is WelcomeDo.Install) scope.launch {
            then.plugins.forEach { id ->
                val on = offers.firstOrNull { it.id == id }?.let { plugins.installPicked(it) } == true
                vm.welcomeOn(if (on) WelcomeEvent.Installed(id) else WelcomeEvent.InstallRefused(id))
            }
        }
        Unit
    }
    Surface(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().systemBarsPadding()) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 96.dp)) {
                WelcomeHead()
                GroupLabel(say.welcomeWhere)
                GroupCard {
                    sources.forEachIndexed { i, source ->
                        val waiting = (source as? WelcomeSource.Offered)?.plugin in installing
                        SourceLine(vm, plugins, source, offers, i < sources.lastIndex, trailing = { if (waiting) LoadingDots(dot = 5.dp) else PickMark(source in flow.picked) }) {
                            vm.welcomeOn(WelcomeEvent.Toggled(source))
                        }
                    }
                }
                Refused(flow, offers)
                WelcomeRestore(vm)
                WelcomePair()
            }
            PillButton(
                if (installing.isEmpty()) say.setupContinue else say.pluginInstalling, null, next,
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(horizontal = GROUP_MARGIN, vertical = 16.dp).testTag("welcome-continue"),
                prominent = true, enabled = flow.page == WelcomePage.Picking && flow.picked.isNotEmpty(),
            )
        }
    }
}

/** Several sources picked: each with Connect, or a check once its profile is in, its form on a page of its own; Continue once one is. */
@Composable
private fun WelcomeSignIns(vm: SettingsViewModel, plugins: dev.nori.music.app.vm.PluginsViewModel, flow: WelcomeFlow) {
    val scheme = MaterialTheme.colorScheme
    val offers by plugins.providerOffers.collectAsStateWithLifecycle()
    var connecting by remember { mutableStateOf<dev.nori.music.ffi.settings.ServerKind?>(null) }
    NoriDialog(connecting, { connecting = null }, DialogStyle.Page) { k -> LoginScreen(vm, onClose = { connecting = null }, kind = k) }
    androidx.activity.compose.BackHandler(flow.added.isEmpty()) { vm.welcomeOn(WelcomeEvent.Back) }
    Surface(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().systemBarsPadding()) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 96.dp)) {
                Text(say.welcomeSignIn, Modifier.padding(start = Space.gutter, end = Space.gutter, top = 72.dp, bottom = 22.dp), style = titled(MaterialTheme.typography.displaySmall.copy(fontSize = 38.sp, lineHeight = 44.sp)))
                GroupCard {
                    flow.picked.forEachIndexed { i, source ->
                        val kind = (source as? WelcomeSource.Kind)?.kind
                        val added = kind in flow.added
                        SourceLine(vm, plugins, source, offers, i < flow.picked.lastIndex, trailing = {
                            if (added) Icon(Iconoir.Check, null, Modifier.size(20.dp), tint = scheme.primary)
                            else Text(say.welcomeConnect, style = MaterialTheme.typography.labelLarge.copy(fontSize = 14.sp))
                        }) { if (!added) connecting = kind }
                    }
                }
                Refused(flow, offers)
            }
            PillButton(
                say.setupContinue, null, { vm.welcomeOn(WelcomeEvent.Continue) },
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(horizontal = GROUP_MARGIN, vertical = 16.dp).testTag("welcome-done"),
                prominent = true, enabled = flow.added.isNotEmpty(),
            )
        }
    }
}

/** One source as a row: nori's server, a plugin's provider, or a plugin a repo offers, [trailing] at its end. */
@Composable
private fun SourceLine(
    vm: SettingsViewModel, plugins: dev.nori.music.app.vm.PluginsViewModel, source: WelcomeSource, offers: List<dev.nori.music.ffi.plugins.Offer>,
    divider: Boolean, trailing: @Composable () -> Unit, onClick: () -> Unit,
) {
    when (source) {
        is WelcomeSource.Kind -> {
            val words = providerWords(vm, source.kind)
            val tag = Modifier.testTag("source-${kindTag(source.kind)}")
            when (val kind = source.kind) {
                is dev.nori.music.ffi.settings.ServerKind.Plugin -> ProviderLine(plugins, kind.id, offers.firstOrNull { it.id == kind.id }, words.name, null, divider, tag, trailing, onClick)
                dev.nori.music.ffi.settings.ServerKind.Subsonic -> ProviderLine(Iconoir.Server, words.name, say.providerSubsonicAbout, divider, tag, trailing, onClick)
            }
        }
        is WelcomeSource.Offered -> offers.firstOrNull { it.id == source.plugin }?.let { o ->
            ProviderLine(plugins, o.id, o, o.name, null, divider, Modifier.testTag("source-offered-${o.id}"), trailing, onClick)
        }
    }
}

/** Whether a source is picked: an empty ring, or filled in the accent with a check. */
@Composable
private fun PickMark(picked: Boolean) {
    val scheme = MaterialTheme.colorScheme
    Box(
        Modifier.size(24.dp).then(
            if (picked) Modifier.background(scheme.primary, androidx.compose.foundation.shape.CircleShape)
            else Modifier.border(1.5.dp, scheme.onSurface.copy(alpha = 0.3f), androidx.compose.foundation.shape.CircleShape),
        ),
        Alignment.Center,
    ) { if (picked) Icon(Iconoir.Check, null, Modifier.size(16.dp), tint = scheme.onPrimary) }
}

/** The picked plugins that would not install, under the list. */
@Composable
private fun Refused(flow: WelcomeFlow, offers: List<dev.nori.music.ffi.plugins.Offer>) {
    if (flow.refused.isEmpty()) return
    val names = flow.refused.map { id -> offers.firstOrNull { it.id == id }?.name ?: id }
    Text(say.welcomeRefused(names), Modifier.padding(start = Space.gutter, end = Space.gutter, top = 10.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
}

/** The first run's head over where the music is. */
@Composable
private fun WelcomeHead() {
    Text(
        say.welcomeTitle, Modifier.fillMaxWidth().padding(start = Space.gutter, end = Space.gutter, top = 72.dp, bottom = 10.dp),
        style = titled(MaterialTheme.typography.displaySmall.copy(fontSize = 38.sp, lineHeight = 44.sp)),
    )
}

/** Under where the music is: restoring a backup brings back its profiles, which open the app. */
/**
 * Pair a device on the welcome, before any profile: it brings another device's profiles, and the app opens
 * on the one in use once sync brought them.
 */
@Composable
private fun WelcomePair() {
    var pairing by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
    NoriDialog(pairing, { pairing = false }, DialogStyle.Page) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().systemBarsPadding()) {
                Box(Modifier.padding(start = 4.dp, top = 4.dp)) { GlyphButton(Iconoir.NavArrowLeft, say.setupBack, Modifier.testTag("pair-back")) { pairing = false } }
                PairDevice(done = { pairing = false })
            }
        }
    }
    Column(Modifier.padding(start = Space.gutter - 12.dp, top = 0.dp, bottom = 24.dp)) {
        TextAction(say.syncPair, { pairing = true }, Modifier.testTag("welcome-pair"))
    }
}

@Composable
private fun WelcomeRestore(vm: SettingsViewModel) {
    val backup by vm.backup.collectAsStateWithLifecycle()
    var file by remember { mutableStateOf<android.net.Uri?>(null) }
    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) file = uri }
    NoriDialog(file, { file = null }) { uri ->
        BackupPasswordDialog(true, { file = null }) { password -> vm.restoreBackup(uri, password); file = null }
    }
    val said = backupWords(LocalContext.current.resources, backup, true)
    Column(Modifier.padding(start = Space.gutter - 12.dp, top = 10.dp)) {
        TextAction(say.restoreBackup, { open.launch(arrayOf("*/*")) }, Modifier.testTag("welcome-restore"), enabled = backup !is dev.nori.music.app.vm.BackupUi.Working)
        if (said != null) Text(
            said, Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodySmall,
            color = if (backup is dev.nori.music.app.vm.BackupUi.Failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * The first run's questions once its first profile is in, over the app opening under them, one to a page:
 * an answer is taken as it is picked and shown on the page, Continue goes on; the look too.
 */
@Composable
fun SetupPages(vm: SettingsViewModel, setup: SetupUi) {
    androidx.activity.compose.BackHandler(setup.at > 0) { vm.setupBack() }
    Surface(Modifier.fillMaxSize().testTag("setup")) {
        Column(Modifier.systemBarsPadding()) {
            SetupBar(setup, vm::setupBack, vm::setupNext)
            AnimatedContent(
                setup.at,
                transitionSpec = {
                    val way = if (targetState > initialState) 1 else -1
                    if (AppMotion.reduce) AppMotion.appear() togetherWith AppMotion.vanish()
                    else (slideInHorizontally(AppMotion.move()) { it / 5 * way } + AppMotion.appear()) togetherWith
                        (slideOutHorizontally(AppMotion.move()) { -it / 5 * way } + AppMotion.vanish())
                },
                label = "setup",
            ) { at ->
                val step = setup.steps[at]
                when (step) {
                    SetupStep.LOOK -> LookStep(vm, last = at == setup.steps.lastIndex)
                    SetupStep.PAIR -> PairStep(vm)
                    else -> AnswerStep(vm, step, setup.answers[step])
                }
            }
        }
    }
}

/** Back while there is a question before, where the run is as dots (the one shown in the accent), and Skip. */
@Composable
private fun SetupBar(setup: SetupUi, back: () -> Unit, skip: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Row(Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.width(96.dp)) { if (setup.at > 0) GlyphButton(Iconoir.NavArrowLeft, say.setupBack,Modifier.testTag("setup-back"), onClick = back) }
        Row(Modifier.weight(1f), Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally), Alignment.CenterVertically) {
            setup.steps.indices.forEach { i ->
                Box(Modifier.size(width = if (i == setup.at) 18.dp else 6.dp, height = 6.dp).background(if (i == setup.at) scheme.primary else scheme.onSurface.copy(alpha = 0.22f), PillShape))
            }
        }
        Box(Modifier.width(96.dp), Alignment.CenterEnd) {
            if (setup.steps[setup.at] !in setOf(SetupStep.LOOK, SetupStep.PAIR)) TextAction(say.setupSkip, skip, Modifier.testTag("setup-skip"))
        }
    }
}

/** A question's title in the page's type. */
@Composable
private fun StepTitle(step: SetupStep) {
    Text(say.setupStep(step), Modifier.padding(start = Space.gutter, end = Space.gutter, top = 20.dp, bottom = 22.dp), style = titled(MaterialTheme.typography.headlineMedium.copy(fontSize = 31.sp)))
}

/**
 * A question: what the answer chosen does, shown on the page's own music (the core's `setup_answers`), the
 * answers as rows, the one chosen checked, and Continue once one is. Picking one takes it at once and plays it
 * across the picture.
 */
@Composable
private fun AnswerStep(vm: SettingsViewModel, step: SetupStep, chosen: SetupAnswer?) {
    val scheme = MaterialTheme.colorScheme
    val answers = remember(step) { dev.nori.music.ffi.settings.setupAnswers(step) }
    val playhead = remember { androidx.compose.animation.core.Animatable(0f) }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val pick = { a: SetupAnswer ->
        vm.setupAnswer(a)
        if (!AppMotion.reduce) scope.launch {
            playhead.snapTo(0f)
            playhead.animateTo(1f, androidx.compose.animation.core.tween(PLAYHEAD_MS, easing = androidx.compose.animation.core.LinearEasing))
            playhead.snapTo(0f)
        }
        Unit
    }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 96.dp)) {
            StepTitle(step)
            Surface(
                Modifier.fillMaxWidth().padding(horizontal = GROUP_MARGIN), shape = TileShape, color = scheme.surfaceContainer,
                contentColor = scheme.onSurface, border = hairlineBorder(),
            ) {
                val covers by vm.setupCovers.collectAsStateWithLifecycle()
                when (step) {
                    SetupStep.LISTENING -> ListeningPreview(covers, chosen ?: SetupAnswer.NO_MIXING) { playhead.value }
                    else -> DataPreview(chosen ?: SetupAnswer.ORIGINAL) { playhead.value }
                }
            }
            GroupCard(Modifier.padding(top = 14.dp)) {
                answers.forEachIndexed { i, a ->
                    val (name, line) = say.setupAnswer(a)
                    GroupRow(
                        name, { if (a != chosen) pick(a) }, Modifier.testTag("setup-${a.name.lowercase()}").semantics { selected = a == chosen },
                        subtitle = line, divider = i < answers.lastIndex, minHeight = 64.dp,
                    ) { if (a == chosen) Icon(Iconoir.Check, null, Modifier.padding(start = 8.dp).size(20.dp), tint = scheme.primary) }
                }
            }
        }
        PillButton(
            say.setupContinue, null, vm::setupNext,
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().navigationBarsPadding().padding(horizontal = GROUP_MARGIN, vertical = 16.dp).testTag("setup-continue"),
            prominent = true, enabled = chosen != null,
        )
    }
}

/** How long the playhead takes across a question's picture once an answer is picked. */
private const val PLAYHEAD_MS = 900

/**
 * Where each of [style]'s songs starts after the last, as a share of a cover's side, and how much of each
 * cover fades in over the one before it: unmixed apart with a cut between them; AutoMix overlapping, blended;
 * its DJ drawn close, blended longer.
 */
private fun spacing(style: SetupAnswer): Pair<Float, Float> = when (style) {
    SetupAnswer.AUTO_MIX -> 0.72f to 0.28f
    SetupAnswer.AUTO_MIX_DJ -> 0.5f to 0.5f
    else -> 1.12f to 0f
}

/**
 * The question's covers in a row, played as [style] plays songs ([spacing]), gliding to their places when
 * another is picked; under them, for the DJ, the beat. [playhead], 0 to 1 (read while drawing), is a line in
 * the accent across them while a pick plays; 0 is none.
 */
@Composable
private fun ListeningPreview(covers: List<android.graphics.Bitmap>, style: SetupAnswer, playhead: () -> Float) {
    val scheme = MaterialTheme.colorScheme
    val (step, fade) = spacing(style)
    val stepNow by androidx.compose.animation.core.animateFloatAsState(step, AppMotion.move(), label = "step")
    val fadeNow by androidx.compose.animation.core.animateFloatAsState(fade, AppMotion.move(), label = "fade")
    val beat by androidx.compose.animation.core.animateFloatAsState(if (style == SetupAnswer.AUTO_MIX_DJ) 1f else 0f, AppMotion.move(), label = "beat")
    androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 22.dp, bottom = 14.dp, start = 18.dp, end = 18.dp)) {
        // Sized so the widest arrangement fits.
        val shown = remember(covers) { covers.map { it.asImageBitmap() } }
        val side = maxWidth / (1 + spacing(SetupAnswer.NO_MIXING).first * (shown.size - 1).coerceAtLeast(1))
        val coverShape = androidx.compose.foundation.shape.RoundedCornerShape(Radius.cover)
        val density = androidx.compose.ui.platform.LocalDensity.current
        val sidePx = with(density) { side.toPx() }
        val width = with(density) { maxWidth.toPx() }
        fun left() = (width - sidePx * (1 + stepNow * (shown.size - 1))) / 2
        Box(
            Modifier.fillMaxWidth().height(side + 24.dp).drawWithContent {
                drawContent()
                // The beat under the mix, coming and going with it.
                if (beat > 0f) {
                    val tick = 1.5.dp.toPx()
                    val start = left()
                    val span = sidePx * (1 + stepNow * (shown.size - 1))
                    for (i in 0..16) drawRect(scheme.onSurface.copy(alpha = 0.32f * beat), Offset(start + span * i / 16f - tick / 2, sidePx + 12.dp.toPx()), Size(tick, 8.dp.toPx()))
                }
                val at = playhead()
                if (at > 0f) {
                    val x = left() + sidePx * (1 + stepNow * (shown.size - 1)) * at
                    drawRect(scheme.primary, Offset(x - 1.dp.toPx(), -6.dp.toPx()), Size(2.dp.toPx(), sidePx + 12.dp.toPx()))
                }
            },
        ) {
            shown.forEachIndexed { i, cover ->
                androidx.compose.foundation.Image(
                    cover, null,
                    Modifier.offset { androidx.compose.ui.unit.IntOffset((left() + i * stepNow * sidePx).toInt(), 0) }.size(side).clip(coverShape)
                        .graphicsLayer { compositingStrategy = androidx.compose.ui.graphics.CompositingStrategy.Offscreen }
                        .drawWithContent {
                            drawContent()
                            // Fading in over the song before it.
                            if (i > 0 && fadeNow > 0f) drawRect(
                                Brush.horizontalGradient(0f to Color.Transparent, fadeNow to Color.Black, endX = size.width),
                                blendMode = androidx.compose.ui.graphics.BlendMode.DstIn,
                            )
                        },
                    contentScale = androidx.compose.ui.layout.ContentScale.Crop,
                )
            }
        }
    }
}

/** The chosen quality as a waveform, finer the higher it is, changing over when another is picked. */
@Composable
private fun DataPreview(quality: SetupAnswer, playhead: () -> Float) {
    val ink = MaterialTheme.colorScheme.onSurface
    val accent = MaterialTheme.colorScheme.primary
    androidx.compose.animation.Crossfade(quality, Modifier.fillMaxWidth().height(120.dp), AppMotion.move(), label = "quality") { q ->
        Box(Modifier.fillMaxSize().waveformPicture(q, ink, accent, playhead))
    }
}

/**
 * The quality as one waveform in [ink], fine for the original file and in fewer, coarser steps the more it saves; [playhead],
 * 0 to 1 (read while drawing), is a line in [accent] across it while a pick plays; 0 is none.
 */
private fun Modifier.waveformPicture(quality: SetupAnswer, ink: Color, accent: Color, playhead: () -> Float): Modifier = drawWithCache {
    val w = size.width
    val h = size.height
    val left = 22.dp.toPx()
    val span = w - 2 * left
    val (wave, share) = when (quality) {
        SetupAnswer.SAVE_DATA -> waveform(18, steps = 4) to 0.62f
        SetupAnswer.HIGH -> waveform(32, steps = 8) to 0.56f
        else -> waveform(56, steps = 0) to 0.5f
    }
    val slot = span / wave.size
    val thick = slot * share
    onDrawBehind {
        wave.forEachIndexed { i, v ->
            val tall = h * 0.62f * v
            drawRoundRect(ink.copy(alpha = 0.78f), Offset(left + slot * i + (slot - thick) / 2, (h - tall) / 2), Size(thick, tall), CornerRadius(thick / 2))
        }
        val at = playhead()
        if (at > 0f) drawRect(accent, Offset(left + span * at - 1.dp.toPx(), h * 0.12f), Size(2.dp.toPx(), h * 0.76f))
    }
}

/** [n] levels of one made-up waveform, 0 to 1; [steps] above 0 rounds each to that many steps, as saved data does. */
private fun waveform(n: Int, steps: Int): List<Float> = List(n) { i ->
    val t = i * 22f / n
    val v = 0.25f + 0.75f * abs(sin(t * 0.55f) * cos(t * 0.21f + 0.4f))
    if (steps > 0) (kotlin.math.ceil(v * steps) / steps).coerceAtMost(1f) else v
}

/** Ends the run, notifications asked for first: here, as the run ends, rather than over the welcome. */
@Composable
private fun rememberFinish(vm: SettingsViewModel): (() -> Unit) -> Unit {
    val context = LocalContext.current
    var then by remember { mutableStateOf({}) }
    val askNotifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { vm.setupNext(); then() }
    return { after ->
        val asks = android.os.Build.VERSION.SDK_INT >= 33 &&
            androidx.core.content.ContextCompat.checkSelfPermission(context, android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED
        then = after
        if (asks) askNotifications.launch(android.Manifest.permission.POST_NOTIFICATIONS) else { vm.setupNext(); after() }
    }
}

/**
 * The last page while this phone is in no group: Pair a device ends the run on Pair a device, Start listening
 * ends it; the sync server's switch and address under Advanced, as Your devices has them.
 */
@Composable
private fun PairStep(vm: SettingsViewModel) {
    val sync: SyncViewModel = viewModel()
    val v by sync.view.collectAsStateWithLifecycle()
    val nav = LocalNav.current
    val finish = rememberFinish(vm)
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 160.dp)) {
            StepTitle(SetupStep.PAIR)
            GroupLabel(say.advanced)
            SyncServerCard(sync, v)
        }
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().navigationBarsPadding().padding(horizontal = GROUP_MARGIN, vertical = 16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            PillButton(say.syncPair, Iconoir.Plus, { finish { nav.go("pair-device") } }, Modifier.fillMaxWidth().testTag("setup-pair"), prominent = true, enabled = v.on)
            TextAction(say.setupStart, { finish {} }, Modifier.padding(top = 8.dp).testTag("setup-start"))
        }
    }
}

/**
 * The look, taken as it is picked: Settings' own appearance and season rows; Continue, or Start listening
 * where it is [last], which ends the run.
 */
@Composable
private fun LookStep(vm: SettingsViewModel, last: Boolean) {
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    val facts by vm.settingsFacts.collectAsStateWithLifecycle()
    val res = LocalContext.current.resources
    // Settings' own appearance and season rows, as its Appearance page draws them.
    val sections = remember(prefs, facts, res) {
        vm.settingsPage("look", prefs, facts, res)?.sections.orEmpty()
            .map { s -> s.copy(rows = s.rows.filter { r -> r is SettingRow.Seasons || (r as? SettingRow.ChipChoice)?.name == "theme" || (r as? SettingRow.Choice)?.name == "theme" }) }
            .filter { it.rows.isNotEmpty() }
    }
    val finish = rememberFinish(vm)
    val start = { if (last) finish {} else vm.setupNext() }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 96.dp)) {
            StepTitle(SetupStep.LOOK)
            sections.forEach { SettingsSectionRows(vm, it, null) }
        }
        PillButton(
            if (last) say.setupStart else say.setupContinue, null, start,
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().navigationBarsPadding().padding(horizontal = GROUP_MARGIN, vertical = 16.dp).testTag("setup-start"), prominent = true,
        )
    }
}
