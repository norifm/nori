package dev.nori.music.app.ui

import androidx.compose.ui.draw.drawBehind
import kotlinx.coroutines.async
import androidx.compose.runtime.withFrameNanos
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionOnScreen
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.stripe.android.PaymentConfiguration
import com.stripe.android.paymentsheet.PaymentSheet
import com.stripe.android.paymentsheet.PaymentSheetResult
import com.stripe.android.paymentsheet.rememberPaymentSheet
import dev.nori.music.app.R
import dev.nori.music.app.vm.Paying
import dev.nori.music.app.vm.SupportViewModel
import dev.nori.music.ffi.Cadence
import dev.nori.music.ffi.Payment
import dev.nori.music.ffi.Stamp
import dev.nori.music.ffi.SupportChoice
import dev.nori.music.ffi.SupportPick
import dev.nori.music.ffi.SupportCurrency
import dev.nori.music.ffi.SupportPage
import dev.nori.music.ffi.TypedAmount
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Supporting nori: the stamp book, a seal for every payment in the accent of the season it was paid in, then
 * a card of rows saying what to give (an offered amount or one typed for Other, every month or once, the
 * currency) and Stripe's payment sheet, dressed in the season's paper and ink.
 */
@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
fun SupportScreen(vm: SupportViewModel) {
    val service = vm.service ?: return
    val context = LocalContext.current
    val tell = LocalMessages.current
    val stamps by vm.stamps.collectAsStateWithLifecycle()
    val monthly by vm.monthly.collectAsStateWithLifecycle()
    val choice by vm.choice.collectAsStateWithLifecycle()
    val paying by vm.paying.collectAsStateWithLifecycle()
    // Each time the page comes back (from the sheet, the browser, another app), it asks what was paid.
    LifecycleResumeEffect(Unit) { vm.refresh(); onPauseOrDispose {} }

    val sheet = rememberPaymentSheet { r ->
        when (r) {
            is PaymentSheetResult.Completed -> vm.taken(paid = true)
            is PaymentSheetResult.Canceled -> vm.taken(paid = false)
            is PaymentSheetResult.Failed -> vm.taken(paid = false, failure = r.error.localizedMessage ?: say.supportFailed)
        }
    }
    val appearance = sheetAppearance()
    LaunchedEffect(paying) {
        when (val p = paying) {
            is Paying.Take -> when (val m = p.payment) {
                is Payment.InApp -> {
                    PaymentConfiguration.init(context, service.publishableKey)
                    val config = PaymentSheet.Configuration.Builder("nori")
                        .googlePay(PaymentSheet.GooglePayConfiguration(
                            if (service.test) PaymentSheet.GooglePayConfiguration.Environment.Test else PaymentSheet.GooglePayConfiguration.Environment.Production,
                            service.merchantCountry, p.currency,
                        ))
                        .billingDetailsCollectionConfiguration(PaymentSheet.BillingDetailsCollectionConfiguration(email = PaymentSheet.BillingDetailsCollectionConfiguration.CollectionMode.Always))
                        .allowsDelayedPaymentMethods(true)
                        .appearance(appearance)
                        .build()
                    sheet.presentWithPaymentIntent(m.clientSecret, config)
                }
                is Payment.InBrowser -> {
                    runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(m.url))) }
                    vm.taken(paid = false)
                }
            }
            is Paying.Failed -> tell(if (p.otherCurrency) say.supportOtherCurrency else p.message)
            else -> {}
        }
    }

    val focus = LocalFocusManager.current
    val density = LocalDensity.current
    val ime = WindowInsets.ime
    val scope = rememberCoroutineScope()
    var pickingCurrency by remember { mutableStateOf(false) }
    // Each tap on Other asks for the keyboard again, also when Other is already chosen.
    var askedOther by remember { mutableIntStateOf(0) }
    // The currency sheet rises once the keyboard is down. A sheet is a window of its own: put up while the
    // keyboard leaves, it took the focus from the field mid-way and was laid out in the room the keyboard
    // was giving back, so it came up cut short or jumped.
    val pickCurrency: () -> Unit = {
        focus.clearFocus()
        scope.launch {
            snapshotFlow { ime.getBottom(density) }.first { it == 0 }
            pickingCurrency = true
        }
    }

    val scroll = rememberScrollState()
    // A seal that comes while the page is open is thanked for, the newest when several come at once. The
    // book keeps its empty place while [thanked]; the thanks shows while [thanking], a little longer, so the
    // book has drawn the seal in its place before the thanks' window goes.
    var thanked by remember { mutableStateOf<Int?>(null) }
    var thanking by remember { mutableStateOf<Int?>(null) }
    var seen by remember { mutableIntStateOf(-1) }
    var landed by remember { mutableStateOf(false) }
    val slot = remember { Slot() }
    var window by remember { mutableStateOf<LayoutCoordinates?>(null) }
    LaunchedEffect(stamps.size) {
        if (seen >= 0 && stamps.size > seen) { landed = false; thanked = stamps.lastIndex; thanking = stamps.lastIndex }
        seen = stamps.size
    }
    // The page ends over the chrome, or over the keyboard while it is up, whichever is higher. What to give
    // stands at its foot, where the thumb is, always whole and riding up with the keyboard; the book takes
    // the room above it and scrolls there, its edges fading as the lyrics' do.
    Column(
        Modifier.fillMaxSize().windowInsetsPadding(ime.union(WindowInsets(bottom = LocalChromeInset.current)))
            // A tap on the page itself puts the keyboard away.
            .clickable(interactionSource = null, indication = null) { focus.clearFocus() },
    ) {
        LargeTitle(say.supportTitle)
        Text(
            say.supportLine, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp),
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val page = MaterialTheme.colorScheme.background
        Box(Modifier.weight(1f).fillMaxWidth().onGloballyPositioned { window = it }.fadesUnderTitle(scroll, page).fadesAtFoot(scroll, page)) {
            // The seal that just came keeps its empty place from the frame it comes in, before the effect
            // above starts its thanks.
            val arriving = if (seen >= 0 && stamps.size > seen) stamps.lastIndex else null
            StampBook(stamps, vm.nextSeason, thanked ?: arriving, landed, slot, Modifier.verticalScroll(scroll).padding(top = 24.dp, bottom = Space.section))
        }
        Column(Modifier.fillMaxWidth().padding(bottom = Space.row)) {
            GroupCard {
                AmountRow(
                    choice, askedOther,
                    pickOffered = { i -> focus.clearFocus(); vm.pickOffered(i) },
                    pickOther = { vm.pickOther(); askedOther++ },
                    type = vm::type, done = { focus.clearFocus() },
                )
                GroupRow(
                    say.supportEveryMonth,
                    { vm.pickCadence(if (choice.cadence == Cadence.MONTHLY) Cadence.ONCE else Cadence.MONTHLY) },
                    trailing = { NoriSwitch(choice.cadence == Cadence.MONTHLY, null) },
                )
                GroupRow(say.supportCurrency, pickCurrency, value = currencyName(choice.currency), chevron = true, divider = false)
            }
            val amount = choice.amount?.toLong()
            PillButton(
                when {
                    amount == null -> say.supportGiveNothing
                    choice.cadence == Cadence.MONTHLY -> say.supportGiveMonthly(money(amount, choice.currency))
                    else -> say.supportGive(money(amount, choice.currency))
                },
                null, { focus.clearFocus(); vm.pay() }, Modifier.fillMaxWidth().padding(start = GROUP_MARGIN, end = GROUP_MARGIN, top = Space.row),
                prominent = true, enabled = amount != null && paying != Paying.Starting,
            )
            if (monthly) Row(Modifier.align(Alignment.CenterHorizontally).padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                val open = { page: SupportPage ->
                    vm.page(page) { url -> runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url))) } }
                }
                TextAction(say.supportManage, { open(SupportPage.MANAGE) })
                TextAction(say.supportStop, { open(SupportPage.STOP) })
            }
        }
    }
    CurrencySheet(pickingCurrency, choice.currency, vm.allCurrencies, { pickingCurrency = false }) { c -> vm.pickCurrency(c); pickingCurrency = false }
    thanking?.let { i ->
        val s = stamps.getOrNull(i) ?: return@let
        // Its place glides into the book's window, centered there, if it is not whole in it.
        val bring: suspend () -> Unit = bring@{
            val at = slot.at?.takeIf { it.isAttached }?.boundsInWindow() ?: return@bring
            val room = window?.takeIf { it.isAttached }?.boundsInWindow() ?: return@bring
            if (at.top < room.top || at.bottom > room.bottom) scroll.animateScrollBy(at.center.y - room.center.y, tween(600, easing = CubicBezierEasing(0.2f, 0.7f, 0.3f, 1f)))
        }
        // Two frames of the page's own window: the seal it was given is drawn.
        val drawn: suspend () -> Unit = { scope.async { withFrameNanos {}; withFrameNanos {} }.await() }
        Thanks(s, sealAccent(s.season), slot, bring, placed = { landed = true; thanked = null }, drawn = drawn) { carried ->
            if (!carried) { landed = false; thanked = null }
            thanking = null
        }
    }
}

/**
 * The amount: its title over the offered amounts and Other as one row of chips; Other chosen opens the
 * typed amount under them, keyboard up, with the limit beside it when what is typed is past one.
 */
@Composable
private fun AmountRow(choice: SupportChoice, askedOther: Int, pickOffered: (Int) -> Unit, pickOther: () -> Unit, type: (String) -> Unit, done: () -> Unit) {
    val other = choice.pick == SupportPick.Other
    Column(Modifier.fillMaxWidth().padding(horizontal = GROUP_INSET, vertical = 12.dp)) {
        Text(say.supportAmount, style = MaterialTheme.typography.bodyLarge)
        // Other currencies and cadences offer other amounts: the chips change with a fade where they stand.
        AnimatedContent(
            choice.offered.map { number(it.toLong(), choice.currency) }, label = "offered",
            transitionSpec = { AppMotion.appear() togetherWith AppMotion.vanish() using SizeTransform(clip = false) },
        ) { labels ->
            val chosen = (choice.pick as? SupportPick.Offered)?.index?.toInt()
            ChipRow(labels + say.supportOther, { i -> if (i == labels.size) other else i == chosen }) { i ->
                if (i == labels.size) pickOther() else pickOffered(i)
            }
        }
    }
    AnimatedVisibility(other, enter = AppMotion.grow(), exit = AppMotion.shrink()) {
        Column {
            Hairline(startIndent = GROUP_INSET)
            OtherAmount(choice, askedOther, type, done)
        }
    }
    Hairline(startIndent = GROUP_INSET)
}

/** What is typed for Other: a large plain number (its currency is the row under it), the limit beside it when it is past one. */
@Composable
private fun OtherAmount(choice: SupportChoice, asked: Int, type: (String) -> Unit, done: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val big = MaterialTheme.typography.headlineSmall.copy(fontSize = 28.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.3).sp, color = scheme.onSurface)
    Row(Modifier.fillMaxWidth().heightIn(min = 60.dp).padding(horizontal = GROUP_INSET), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (choice.other.isEmpty()) Text("0", style = big.copy(color = scheme.onSurfaceVariant.copy(alpha = 0.5f)), maxLines = 1)
            BasicTextField(
                choice.other, type, Modifier.fillMaxWidth().takesFocus(asked),
                textStyle = big, singleLine = true, cursorBrush = SolidColor(scheme.primary),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { done() }),
            )
        }
        val limit = when (val t = choice.typed) {
            is TypedAmount.TooLittle -> say.supportLeast(money(t.least.toLong(), choice.currency))
            is TypedAmount.TooMuch -> say.supportMost(money(t.most.toLong(), choice.currency))
            else -> null
        }
        // The last limit stays while it fades, the amount already back in range.
        val last = remember { arrayOf("") }
        if (limit != null) last[0] = limit
        AnimatedVisibility(limit != null, enter = AppMotion.appear(), exit = AppMotion.vanish()) {
            Text(last[0], Modifier.padding(start = 12.dp), style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1)
        }
    }
}

/** Every currency offered, the current one checked; a tap picks it and closes the sheet. */
@Composable
private fun CurrencySheet(open: Boolean, current: SupportCurrency, all: List<SupportCurrency>, close: () -> Unit, pick: (SupportCurrency) -> Unit) {
    NoriSheet(open, close) {
        Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
            SheetTitle(say.supportCurrency)
            GroupCard {
                all.forEachIndexed { i, c ->
                    GroupRow(
                        currencyName(c), { pick(c) }, value = c.code, divider = i < all.lastIndex, minHeight = SHEET_ROW,
                        trailing = {
                            if (c.code == current.code) Icon(Iconoir.Check, say.chosen, Modifier.padding(start = 10.dp).size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        },
                    )
                }
            }
        }
    }
}

/** [c]'s name as the phone says it: "Euro", "Polish Zloty". */
private fun currencyName(c: SupportCurrency): String =
    java.util.Currency.getInstance(c.code).getDisplayName(java.util.Locale.getDefault()).replaceFirstChar { it.titlecase() }

/** The seals, oldest first, five to a row from the start, and after them the next one's place: its outline and kanji, uncolored, as a seal not yet a favorite. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StampBook(
    stamps: List<Stamp>,
    next: dev.nori.music.ffi.look.Season,
    thanked: Int?,
    landed: Boolean,
    slot: Slot,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier.fillMaxWidth().padding(horizontal = Space.gutter)) {
        // Five fill the row from edge to edge.
        val side = (maxWidth - SEAL_GAP * (SEALS_IN_ROW - 1)) / SEALS_IN_ROW
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(SEAL_GAP),
            verticalArrangement = Arrangement.spacedBy(SEAL_GAP),
            maxItemsInEachRow = SEALS_IN_ROW,
        ) {
            stamps.forEachIndexed { i, s ->
                // While it is thanked for, a seal still has its empty place, which it is carried to.
                if (i == thanked) Blank(s.season, side, Modifier.onGloballyPositioned { slot.at = it })
                else Seal(s, sealAccent(s.season), side)
            }
            if (thanked == null) {
                // After a seal was carried in, the next place grows in beside it.
                val shown = remember { Animatable(if (landed && !AppMotion.reduce) 0f else 1f) }
                LaunchedEffect(shown) { shown.animateTo(1f, tween(320, delayMillis = 60, easing = CubicBezierEasing(0.2f, 0.9f, 0.3f, 1.2f))) }
                Blank(next, side, Modifier.graphicsLayer { alpha = shown.value.coerceIn(0f, 1f); scaleX = 0.6f + 0.4f * shown.value; scaleY = scaleX })
            }
        }
    }
}

/** A seal's place before it is paid for: [FRAME] with [season]'s kanji, uncolored. */
@Composable
private fun Blank(season: dev.nori.music.ffi.look.Season, side: Dp, modifier: Modifier = Modifier) {
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Canvas(modifier.size(side)) {
        scale(size.width / 24f, size.height / 24f, Offset.Zero) {
            drawPath(FRAME, quiet, style = Stroke(1.5f))
            drawPath(OUTLINED[season.ordinal], quiet, alpha = 0.85f)
        }
    }
}

/** [season]'s accent in the appearance worn: a seal's color. */
@Composable
private fun sealAccent(season: dev.nori.music.ffi.look.Season): Color {
    val dark = LocalPapers.current.darkTheme
    return remember(season, dark) { Color(dev.nori.music.ffi.look.palette(season, 1u, 1u, false, dark, false).accent.toInt()) }
}

/** Where the book's empty place for the seal being thanked for is, once laid out. */
private class Slot {
    var at: LayoutCoordinates? = null
}

/** A seal's pose on its way: moved by [x], [y] px, turned [deg], scaled [sx] by [sy]. */
private class Pose(val x: Float, val y: Float, val deg: Float, val sx: Float, val sy: Float = sx)

private fun lerp(a: Float, b: Float, t: Float) = a + (b - a) * t

/** The way to the book: lifted a little and turned to the seal's tilt, set down, pressed, settled. Each
 *  leg starts at its fraction of the way with its own easing, from its pose to the next. */
private class Way(dx: Float, dy: Float, tilt: Float, k: Float, lift: Float) {
    private val legs = listOf(
        Triple(0f, CubicBezierEasing(0.45f, 0f, 0.55f, 1f), Pose(0f, 0f, 0f, 1f)),
        Triple(0.42f, CubicBezierEasing(0.45f, 0f, 0.25f, 1f), Pose(dx * 0.35f, dy * 0.35f - lift, tilt / 2, (1 + k) / 2 + 0.06f)),
        Triple(0.8f, CubicBezierEasing(0.3f, 0f, 0.5f, 1f), Pose(dx, dy, tilt, k)),
        Triple(0.89f, CubicBezierEasing(0.4f, 0f, 0.3f, 1f), Pose(dx, dy, tilt, k * 1.06f, k * 0.93f)),
        Triple(1f, LinearEasing, Pose(dx, dy, tilt, k)),
    )

    fun at(t: Float): Pose {
        val i = legs.indexOfLast { it.first <= t }.coerceAtMost(legs.size - 2)
        val (start, easing, from) = legs[i]
        val (end, _, to) = legs[i + 1]
        val e = easing.transform(((t - start) / (end - start)).coerceIn(0f, 1f))
        return Pose(lerp(from.x, to.x, e), lerp(from.y, to.y, e), lerp(from.deg, to.deg, e), lerp(from.sx, to.sx, e), lerp(from.sy, to.sy, e))
    }
}

/**
 * The thanks for a seal paid while the page is open, over the whole app (its own window): the page under a
 * veil of its paper, the seal pressed down in the middle as a stamp is, "Thank you" under
 * it; then the seal carried to its empty place in the book ([slot], brought into view by [bring]) and
 * pressed there, all on this seal, so the book's takes over at rest: once this window has drawn the seal
 * at rest, [placed] puts it in the book, and the thanks goes once the page's window has [drawn] it there.
 * Neither waits on a guess of how fast a frame is. A touch or back ends it at once;
 * [done] says whether the seal was carried. Under Motion Reduced it is the thanks alone, still.
 */
@Composable
private fun Thanks(stamp: Stamp, accent: Color, slot: Slot, bring: suspend () -> Unit, placed: () -> Unit, drawn: suspend () -> Unit, done: (carried: Boolean) -> Unit) {
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current
    val page = MaterialTheme.colorScheme.background
    val veil = remember { Animatable(0f) }
    val drop = remember { Animatable(0f) }
    val press = remember { Animatable(0f) }
    val words = remember { Animatable(0f) }
    val flight = remember { Animatable(0f) }
    var big by remember { mutableStateOf<LayoutCoordinates?>(null) }
    var way by remember { mutableStateOf<Way?>(null) }
    val finish by rememberUpdatedState(done)
    var over by remember { mutableStateOf(false) }
    val end = { carried: Boolean -> if (!over) { over = true; finish(carried) } }
    Dialog({ end(false) }, DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false, dismissOnClickOutside = false)) {
        val view = LocalView.current
        // The veil is ours: no dimming or window animation of the platform's.
        SideEffect { (view.parent as? DialogWindowProvider)?.window?.let { it.setDimAmount(0f); it.setWindowAnimations(0) } }
        LaunchedEffect(Unit) {
            // The dialog's composition is the window's own; run on the app's clock whatever it inherited.
            withContext(AppMotion) {
                if (AppMotion.reduce) {
                    veil.snapTo(1f); drop.snapTo(1f); press.snapTo(1f); words.snapTo(1f)
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                    delay(1800)
                    return@withContext end(false)
                }
                coroutineScope {
                    launch { veil.animateTo(1f, tween(320, easing = LinearOutSlowInEasing)) }
                    drop.animateTo(1f, tween(440, delayMillis = 120, easing = CubicBezierEasing(0.55f, 0f, 0.9f, 0.45f)))
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                    launch { press.animateTo(1f, tween(220, easing = LinearEasing)) }
                    words.animateTo(1f, tween(420, delayMillis = 140, easing = LinearOutSlowInEasing))
                    launch { bring() }
                    delay(1300)
                }
                val from = big?.takeIf { it.isAttached }
                val to = slot.at?.takeIf { it.isAttached }
                if (from == null || to == null) return@withContext end(false)
                val a = from.positionOnScreen() + Offset(from.size.width / 2f, from.size.height / 2f)
                val b = to.positionOnScreen() + Offset(to.size.width / 2f, to.size.height / 2f)
                way = Way(b.x - a.x, b.y - a.y, stamp.tilt, to.size.width.toFloat() / from.size.width, with(density) { 36.dp.toPx() })
                coroutineScope {
                    launch { words.animateTo(0f, tween(220, easing = FastOutLinearInEasing)) }
                    launch { veil.animateTo(0f, tween(640, delayMillis = 180, easing = FastOutSlowInEasing)) }
                    launch { delay(784); haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove) }
                    flight.animateTo(1f, tween(980, easing = LinearEasing))
                }
                // This window has drawn the seal at rest, then the page's has drawn it in the book.
                withFrameNanos {}
                withFrameNanos {}
                placed()
                drawn()
                end(true)
            }
        }
        Box(
            Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { end(false) } },
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.fillMaxSize().graphicsLayer { alpha = veil.value }.background(page.copy(alpha = 0.92f)))
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Top) {
                Box(Modifier.size(168.dp), contentAlignment = Alignment.Center) {
                    Seal(stamp.copy(tilt = 0f), accent, 168.dp, Modifier.onGloballyPositioned { big = it }.graphicsLayer {
                        val w = way
                        if (w != null) {
                            val p = w.at(flight.value)
                            translationX = p.x; translationY = p.y; rotationZ = p.deg; scaleX = p.sx; scaleY = p.sy
                        } else {
                            val d = drop.value
                            val fall = lerp(1.6f, 1f, d)
                            // Pressed: a little wider and flatter at a third of the way, then itself.
                            val q = press.value
                            val squash = if (q < 0.35f) q / 0.35f else (1f - q) / 0.65f
                            scaleX = fall * (1f + 0.05f * squash); scaleY = fall * (1f - 0.05f * squash)
                            translationY = lerp(-32.dp.toPx(), 0f, d)
                            alpha = (d / 0.6f).coerceIn(0f, 1f)
                        }
                    }.drawBehind {
                        // The page under its cut kanji and border, as in the book: over the empty place it lands
                        // on, that place's frame does not show through.
                        val u = size.width / 24f
                        drawRoundRect(page, Offset(2f * u, 2f * u), Size(20f * u, 20f * u), CornerRadius(3.5f * u))
                    })
                }
                Text(
                    say.supportThanks,
                    // 10 dp under the seal to the top of its capitals: the seal is drawn 2 of 24 (14 dp) inside its
                    // box, and Literata's line has 14 dp above the capitals.
                    // A touch left of center: "Th" is heavy, "you" light, and centered it leaned right.
                    Modifier.layout { m, c -> val p = m.measure(c); val cut = (14.dp + 14.dp - 10.dp).roundToPx(); layout(p.width, p.height - cut) { p.place(-(1.5).dp.roundToPx(), -cut) } }
                        .graphicsLayer { alpha = words.value; translationY = (1f - words.value) * 10.dp.toPx() },
                    style = titled(MaterialTheme.typography.headlineMedium.copy(fontSize = 31.sp)), color = MaterialTheme.colorScheme.onBackground,
                )
            }
        }
    }
}

private val SEAL_GAP = 14.dp
private const val SEALS_IN_ROW = 5

/** The chop's body, its thin inner border cut out (the favorite seal's). */
private const val SEAL_BODY = "M5.5,2H18.5A3.5,3.5 0 0 1 22,5.5V18.5A3.5,3.5 0 0 1 18.5,22H5.5A3.5,3.5 0 0 1 2,18.5V5.5A3.5,3.5 0 0 1 5.5,2ZM5.85,3.75H18.15A2.1,2.1 0 0 1 20.25,5.85V18.15A2.1,2.1 0 0 1 18.15,20.25H5.85A2.1,2.1 0 0 1 3.75,18.15V5.85A2.1,2.1 0 0 1 5.85,3.75ZM5.95,4.35H18.05A1.6,1.6 0 0 1 19.65,5.95V18.05A1.6,1.6 0 0 1 18.05,19.65H5.95A1.6,1.6 0 0 1 4.35,18.05V5.95A1.6,1.6 0 0 1 5.95,4.35Z"

/** Each season's seal on a 24 by 24 grid: the body with the season's kanji cut out of it. */
private val SEALS: List<Path> by lazy {
    Kanji.seasonPaths.map { glyph ->
        val body = PathParser().parsePathString(SEAL_BODY).toPath()
        val cut = PathParser().parsePathString(glyph).toPath()
        // The kanji fills the border's inside: its 24 grid scaled about the middle.
        cut.transform(Matrix().apply { translate(12f, 12f); scale(0.62f, 0.62f); translate(-12f, -12f) })
        body.addPath(cut)
        body.fillType = PathFillType.EvenOdd
        body
    }
}

/** The outline seal's frame: the filled seal's square in a 1.5 line, centered on 3 to 21 (`seal_outline`). */
private val FRAME: Path by lazy { PathParser().parsePathString("M6.5,3H17.5A3.5,3.5 0 0 1 21,6.5V17.5A3.5,3.5 0 0 1 17.5,21H6.5A3.5,3.5 0 0 1 3,17.5V6.5A3.5,3.5 0 0 1 6.5,3Z").toPath() }

/** Each season's kanji inside [FRAME]. */
private val OUTLINED: List<Path> by lazy {
    Kanji.seasonPaths.map { glyph ->
        PathParser().parsePathString(glyph).toPath().apply { transform(Matrix().apply { translate(12f, 12f); scale(0.56f, 0.56f); translate(-12f, -12f) }) }
    }
}

@Composable
private fun Seal(stamp: Stamp, color: Color, side: Dp, modifier: Modifier = Modifier) {
    val path = SEALS[stamp.season.ordinal]
    Canvas(modifier.size(side)) {
        rotate(stamp.tilt) {
            scale(size.width / 24f, size.height / 24f, Offset.Zero) { drawPath(path, color) }
        }
    }
}

/** [minor] units of [c] as a plain number, for the offered chips: "40", "1,000". */
private fun number(minor: Long, c: SupportCurrency): String {
    val f = java.text.NumberFormat.getNumberInstance()
    val scale = Math.pow(10.0, c.digits.toDouble())
    val whole = minor % scale.toLong() == 0L
    f.minimumFractionDigits = if (whole) 0 else c.digits.toInt()
    f.maximumFractionDigits = if (whole) 0 else c.digits.toInt()
    return f.format(minor / scale)
}

/** [minor] units of [c] as the phone writes money: "40 zł", "€5", whole amounts without decimals. */
private fun money(minor: Long, c: SupportCurrency): String {
    val f = java.text.NumberFormat.getCurrencyInstance()
    f.currency = java.util.Currency.getInstance(c.code)
    val scale = Math.pow(10.0, c.digits.toDouble())
    val whole = minor % scale.toLong() == 0L
    f.minimumFractionDigits = if (whole) 0 else c.digits.toInt()
    f.maximumFractionDigits = if (whole) 0 else c.digits.toInt()
    return f.format(minor / scale)
}

/** Stripe's sheet in the theme worn now: its paper, ink, accent and corners, light or dark alike. */
@Composable
private fun sheetAppearance(): PaymentSheet.Appearance {
    val s = MaterialTheme.colorScheme
    val colors = PaymentSheet.Colors(
        primary = s.primary.toArgb(),
        surface = s.background.toArgb(),
        component = s.surfaceContainer.toArgb(),
        componentBorder = s.onSurface.copy(alpha = 0.14f).toArgb(),
        componentDivider = s.onSurface.copy(alpha = 0.10f).toArgb(),
        onComponent = s.onSurface.toArgb(),
        subtitle = s.onSurfaceVariant.toArgb(),
        placeholderText = s.onSurfaceVariant.toArgb(),
        onSurface = s.onSurface.toArgb(),
        appBarIcon = s.onSurface.toArgb(),
        error = s.error.toArgb(),
    )
    val button = PaymentSheet.PrimaryButtonColors(background = s.onSurface.toArgb(), onBackground = s.background.toArgb(), border = s.onSurface.toArgb())
    return PaymentSheet.Appearance(
        colorsLight = colors, colorsDark = colors,
        shapes = PaymentSheet.Shapes(cornerRadiusDp = 12f, borderStrokeWidthDp = 0.5f),
        typography = PaymentSheet.Typography(sizeScaleFactor = 1f, fontResId = R.font.inter),
        primaryButton = PaymentSheet.PrimaryButton(colorsLight = button, colorsDark = button, shape = PaymentSheet.PrimaryButtonShape(cornerRadiusDp = 100f)),
    )
}
