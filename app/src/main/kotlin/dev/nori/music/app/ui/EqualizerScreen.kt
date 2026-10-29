package dev.nori.music.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Surface
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.drawText
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.layout.height
import kotlin.math.roundToInt
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.compose.runtime.collectAsState
import dev.nori.music.ffi.settings.EqLevel
import dev.nori.music.ffi.settings.EqMode
import dev.nori.music.ffi.settings.SoundBand
import dev.nori.music.ffi.settings.BandField
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import dev.nori.music.ffi.settings.BandChannel
import dev.nori.music.ffi.model.EqKind
import dev.nori.music.settings.EqBands
import dev.nori.music.settings.EQ
import dev.nori.music.settings.effectivePreampDb
import dev.nori.music.settings.usesGain
import dev.nori.music.settings.slope

/** A meter's reading while this screen is [seen]; nothing is read while it is not. */
@Composable
internal fun meterReading(meter: kotlinx.coroutines.flow.Flow<Float>): Float {
    val seen = seen()
    val flow = remember(meter, seen) { if (seen) meter else kotlinx.coroutines.flow.emptyFlow() }
    return flow.collectAsState(0f).value
}

/** A band's label, its frequency and a mark for its channel or kind (nori-core's `settings::band_label`). */
private fun bandLabel(b: SoundBand): String = say.band(b.freq, dev.nori.music.ffi.settings.BandMark.entries[EqBands.mark(b.kind.ordinal, b.channel.ordinal)])

/** How far each control goes: the core's, the same ranges it holds every edit in. */
private val ranges get() = EQ.eqRanges

@Composable
fun EqualizerScreen(vm: SettingsViewModel) {
    val p by vm.prefs.collectAsStateWithLifecycle()
    LifecycleStartEffect(vm) { vm.soundPageShown(true); onStopOrDispose { vm.soundPageShown(false) } }
    val nav = LocalNav.current
    var importing by remember { mutableStateOf(false) }

    NoriDialog(importing, { importing = false }) { ImportDialog(vm) { importing = false } }

    Column(Modifier.underStatusBar().verticalScroll(rememberScrollState()).padding(top = LocalStatusRoom.current, bottom = LocalChromeInset.current)) {
        LargeTitle(say.equalizer, Modifier.padding(end = Space.gutter - Space.tight)) {
            NoriSwitch(p.eqEnabled, { on -> vm.update { it.copy(eqEnabled = on) } })
        }
        val graphic = p.eqMode == EqMode.GRAPHIC
        // Two equalizers, each with its own settings: the one picked here is the one that plays.
        Row(Modifier.padding(horizontal = Space.gutter, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Chip(say.eqGraphic, graphic) { if (!graphic) vm.setEqMode(EqMode.GRAPHIC) }
            Chip(say.eqParametric, !graphic) { if (graphic) vm.setEqMode(EqMode.PARAMETRIC) }
        }
        // Two settings switch the whole sample chain off. Without this the screen looks broken: bands
        // move, the limiter says it is on, and nothing whatsoever happens to the sound.
        val dac by vm.dac.collectAsStateWithLifecycle()
        val bypass = remember(dac.bitPerfect, p.soundBypass) { dev.nori.music.ffi.settings.eqBypass(dac.bitPerfect, p.soundBypass)?.let(say::eqBypass) }
        if (bypass != null) Surface(
            shape = CardShape, color = MaterialTheme.colorScheme.errorContainer,
            modifier = Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 8.dp),
        ) {
            Text(bypass, Modifier.padding(12.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer)
        }

        if (graphic) GraphicBands(vm, p.eqGraphic, p.eqGraphicTarget, p.eqEnabled)
        else ParametricBands(vm, p.eqBands, p.eqEnabled)

        GroupLabel(say.presets)
        LazyRow(Modifier.bleedsToEdges(), contentPadding = edgePadding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(vm.presets) { preset -> Chip(say.preset(preset.kind), false) { vm.applyPreset(preset) } }
        }
        Spacer(Modifier.height(10.dp))
        GroupCard {
            // On the graphic equalizer a headphone correction is fitted to its sliders.
            GroupRow(say.headphonePresets, nav::autoEq, icon = Iconoir.Headset, chevron = true)
            GroupRow(say.pastePreset, { importing = true }, icon = Iconoir.Copy)
            GroupRow(say.reset, vm::resetBands, icon = Iconoir.Refresh, divider = false)
        }
        Spacer(Modifier.height(10.dp))
        GroupCard {
            val automatic = p.eqPreampDb == null
            GroupRow(
                remember(p.effectivePreampDb, automatic) { say.preamp(p.effectivePreampDb, automatic) }, { vm.setAutoPreamp(!automatic) },
                divider = !automatic, trailing = { NoriSwitch(automatic, vm::setAutoPreamp) },
            )
            p.eqPreampDb?.let { v -> CardSlider(v, ranges.preamp.min..ranges.preamp.max, { x -> vm.setLevel(EqLevel.PREAMP, x) }, enabled = p.eqEnabled, divider = false) }
        }

        GroupLabel(say.output)
        GroupCard {
            GroupRow(say.balance, value = remember(p.balance) { say.balance(p.balance) }, divider = false)
            CardSlider(p.balance, ranges.balance.min..ranges.balance.max, { v -> vm.setLevel(EqLevel.BALANCE, v) }, centred = true)
            Toggle(say.mono, say.monoDetail, p.mono) { on -> vm.update { it.copy(mono = on) } }
            Toggle(say.limiter, say.limiterDetail, p.limiter, divider = p.limiter) { on -> vm.update { it.copy(limiter = on) } }
            if (p.limiter) {
                GroupRow(remember(p.limiterThresholdDb) { say.ceiling(p.limiterThresholdDb) }, divider = false, trailing = { LimiterReduction() })
                CardSlider(p.limiterThresholdDb, ranges.limiter.min..ranges.limiter.max, { v -> vm.setLevel(EqLevel.LIMITER, v) }, divider = false)
            }
        }

        DevicesSection(vm)

        GroupLabel(say.profiles)
        val profiles by vm.profiles.collectAsStateWithLifecycle()
        var naming by remember { mutableStateOf(false) }
        var newName by remember { mutableStateOf("") }
        NoriDialog(naming, { naming = false }) {
            AlertCard(
                title = { Text(say.saveTheseSettings) },
                text = { FormField(newName, { newName = it }, label = { Text(say.name) }, singleLine = true) },
                confirmButton = { TextAction(say.save, { vm.saveProfile(newName); naming = false }, enabled = newName.isNotBlank()) },
                dismissButton = { TextAction(say.cancel, { naming = false }) },
            )
        }
        val rows by vm.deviceRows.collectAsStateWithLifecycle()
        GroupCard {
            AnimatedRows(profiles, { it.name }) { profile ->
                val used = remember(rows, profile) { say.profileUse(rows.filter { it.output in profile.outputs }.map { it.name }) }
                GroupRow(
                    profile.name, { vm.applyProfile(profile) }, subtitle = used,
                    trailing = { IconButton({ vm.deleteProfile(profile.name) }) { Icon(Iconoir.Trash, remember(profile.name) { say.deleteNamed(profile.name) }, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) } },
                )
            }
            GroupRow(say.saveAsProfile, { newName = ""; naming = true }, icon = Iconoir.Plus, divider = false)
        }
    }
}

/** A slider inside a [GroupCard], under the row it sets. */
@Composable
private fun CardSlider(
    value: Float, range: ClosedFloatingPointRange<Float>, onChange: (Float) -> Unit,
    enabled: Boolean = true, centred: Boolean = false, divider: Boolean = true,
) {
    NoriSlider(value, range, onChange, Modifier.padding(horizontal = GROUP_INSET).padding(bottom = 6.dp), enabled = enabled, centred = centred)
    if (divider) Hairline(startIndent = GROUP_INSET)
}

/** The graphic equalizer: the curve it plays with a point per band to drag, and its layouts. */
@Composable
private fun GraphicBands(vm: SettingsViewModel, sliders: List<Float>, target: List<Float>, enabled: Boolean) {
    val count = sliders.size
    // The bands' labels and exact centres are the core's, asked once per layout.
    val bands = remember(count) { dev.nori.music.ffi.settings.graphicBands(count.toUInt()) }
    val response = remember(sliders) { dev.nori.music.ffi.settings.graphicResponse(sliders, CURVE_FREQS) }
    // A headphone correction's curve, drawn faintly at the level the sliders play it: both run 20 Hz to
    // 20 kHz on a log scale, so their means line them up.
    val aim = remember(target, response) {
        if (target.isEmpty()) emptyList() else { val shift = response.average().toFloat() - target.average().toFloat(); target.map { it + shift } }
    }
    val points = remember(bands, sliders) { sliders.mapIndexed { i, v -> CurvePoint(bands.getOrNull(i)?.let { EqBands.position(it.freq) } ?: 0f, v) } }
    val gain = ranges.gain
    var chosen by androidx.compose.runtime.saveable.rememberSaveable(count) { mutableStateOf(-1) }
    val label = { i: Int -> bands.getOrNull(i)?.let { dev.nori.music.text.Fmt.isoBand(it.labelHz) }.orEmpty() }
    val set = { i: Int, db: Float -> vm.setGraphic(i, ((db * 2f).roundToInt() / 2f).coerceIn(gain.min, gain.max)) }
    CurveCard(
        response, aim, points, enabled, label,
        nearestAcross = true, chosen = chosen,
        onTap = { chosen = if (chosen == it) -1 else it },
        onDrag = { i, _, db -> chosen = i; set(i, db) },
    )
    val v = sliders.getOrNull(chosen)
    if (v == null) Hint(say.eqTapGraphic) else {
        Spacer(Modifier.height(10.dp))
        GroupCard {
            BandHeader(remember(chosen, count) { label(chosen) }, chosen, count, { chosen = it }) {}
            Row(Modifier.padding(start = 4.dp, end = GROUP_INSET, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton({ set(chosen, v - 0.5f) }, enabled = enabled && v > gain.min) { Icon(Iconoir.MinusCircle, say.eqLower, Modifier.size(22.dp)) }
                NoriSlider(v, gain.min..gain.max, { x -> set(chosen, x) }, Modifier.weight(1f), enabled = enabled, centred = true)
                IconButton({ set(chosen, v + 0.5f) }, enabled = enabled && v < gain.max) { Icon(Iconoir.PlusCircle, say.eqRaise, Modifier.size(22.dp)) }
                ValueField(remember(v) { dev.nori.music.text.Fmt.signedDb(v) }, enabled, Modifier.width(NUMBER_W)) { typed ->
                    dev.nori.music.ffi.settings.bandTyped(BandField.GAIN, typed)?.let { set(chosen, it) }
                }
            }
        }
    }
    Row(Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        dev.nori.music.ffi.settings.graphicLayouts().forEach { n -> Chip(say.eqBandCount(n.toInt()), n.toInt() == count) { if (n.toInt() != count) vm.setEqLayout(n.toInt()) } }
    }
    // After a headphone correction: how closely the sliders follow it, asked of the core once per change.
    if (target.isNotEmpty()) {
        val follow = remember(sliders, target) { dev.nori.music.ffi.settings.graphicFollow(sliders, target) }
        follow?.let {
            Text(
                remember(it.maxDb, count) { say.eqFollows(it.maxDb, count) },
                Modifier.padding(horizontal = Space.gutter, vertical = 2.dp),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * The parametric equalizer, as many bands as wanted anywhere: a tap on the curve adds one there, a drag
 * moves one in frequency and gain, and the chosen one is edited in full under the curve.
 */
@Composable
private fun ParametricBands(vm: SettingsViewModel, bands: List<SoundBand>, enabled: Boolean) {
    var chosen by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(-1) }
    val response = remember(bands) { dev.nori.music.ffi.settings.bandsResponse(bands, CURVE_FREQS) }
    val points = remember(bands) { bands.map { CurvePoint(EqBands.position(it.freq), if (it.kind.usesGain) it.gainDb else 0f) } }
    val gain = ranges.gain
    CurveCard(
        response.left, response.right.orEmpty(), points, enabled,
        label = { i -> bands.getOrNull(i)?.let(::bandLabel).orEmpty() },
        nearestAcross = false,
        shapes = response.each, chosen = chosen,
        onTap = { chosen = it },
        onEmpty = { x, db ->
            chosen = bands.size
            vm.addBand(EqBands.freqAt(x), (db * 2f).roundToInt() / 2f)
        },
        onDrag = { i, x, db ->
            chosen = i
            val b = bands.getOrNull(i) ?: return@CurveCard
            val freq = EqBands.moved(b.freq, x)
            vm.setBand(i, if (b.kind.usesGain) b.copy(freq = freq, gainDb = ((db * 2f).roundToInt() / 2f).coerceIn(gain.min, gain.max)) else b.copy(freq = freq))
        },
    )
    val band = bands.getOrNull(chosen)
    if (band == null) Hint(if (bands.isEmpty()) say.eqTapToAdd else say.eqTapABand) else {
        Spacer(Modifier.height(10.dp))
        BandEditor(band, chosen, bands.size, { chosen = it }, enabled, { b -> vm.setBand(chosen, b) }) { vm.removeBand(chosen); chosen = -1 }
    }
}

/** A quiet line under the curve saying what a touch does there. */
@Composable
private fun Hint(text: String) {
    Text(text, Modifier.padding(horizontal = Space.gutter + 4.dp, vertical = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/**
 * The top of a band's editor: its name between steps to the band before and after (bands too close to
 * tell apart by touch are reached this way), then [trailing].
 */
@Composable
private fun BandHeader(name: String, at: Int, count: Int, onGo: (Int) -> Unit, trailing: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = GROUP_INSET), verticalAlignment = Alignment.CenterVertically) {
        IconButton({ onGo(at - 1) }, enabled = at > 0) { Icon(Iconoir.NavArrowLeft, say.previous, Modifier.size(20.dp)) }
        Text(name, Modifier.widthIn(min = 44.dp), style = MaterialTheme.typography.titleMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        IconButton({ onGo(at + 1) }, enabled = at < count - 1) { Icon(Iconoir.NavArrowRight, say.next, Modifier.size(20.dp)) }
        Spacer(Modifier.weight(1f))
        trailing()
    }
}

/**
 * The chosen band in full: its kind (a list) and side, then its frequency, gain and width as numbers to
 * type, each with a slider under them for a coarse move, and a way to remove it.
 */
@Composable
private fun BandEditor(band: SoundBand, at: Int, count: Int, onGo: (Int) -> Unit, enabled: Boolean, onChange: (SoundBand) -> Unit, onRemove: () -> Unit) {
    val gain = ranges.gain
    val typed = { field: BandField, text: String ->
        dev.nori.music.ffi.settings.bandTyped(field, text)?.let { v ->
            onChange(when (field) { BandField.FREQUENCY -> band.copy(freq = v); BandField.GAIN -> band.copy(gainDb = v); BandField.Q -> band.copy(q = v) })
        }
    }
    GroupCard {
        BandHeader(remember(band.freq, band.channel, band.kind) { bandLabel(band) }, at, count, onGo) {
            IconButton(onRemove) { Icon(Iconoir.Trash, say.removeBand, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        Row(Modifier.padding(horizontal = GROUP_INSET - 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            KindPicker(band.kind, Modifier.weight(1f, fill = false)) { k -> onChange(band.copy(kind = k)) }
            Spacer(Modifier.weight(0.01f))
            BandChannel.entries.forEach { c -> Chip(say.bandChannel(c), band.channel == c) { onChange(band.copy(channel = c)) } }
        }
        Spacer(Modifier.height(6.dp))
        ValueRow(say.frequency, remember(band.freq) { dev.nori.music.text.Fmt.hz(band.freq) }, EqBands.position(band.freq), 0f..1f, enabled,
            onSlide = { x -> onChange(band.copy(freq = EqBands.freqAt(x))) }) { typed(BandField.FREQUENCY, it) }
        if (band.kind.usesGain) ValueRow(say.gain, remember(band.gainDb) { dev.nori.music.text.Fmt.signedDb(band.gainDb) }, band.gainDb, gain.min..gain.max, enabled, centred = true,
            onSlide = { v -> onChange(band.copy(gainDb = (v * 2f).roundToInt() / 2f)) }) { typed(BandField.GAIN, it) }
        ValueRow(say.widthName(band.kind.slope), remember(band.q) { dev.nori.music.text.Fmt.fixed(band.q.toDouble(), 2) }, band.q, ranges.q.min..ranges.q.max, enabled,
            onSlide = { q -> onChange(band.copy(q = (q * 100f).roundToInt() / 100f)) }) { typed(BandField.Q, it) }
        Spacer(Modifier.height(8.dp))
    }
}

/** The band's kind, and the list of kinds it opens. */
@Composable
private fun KindPicker(kind: EqKind, modifier: Modifier, onPick: (EqKind) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val scheme = MaterialTheme.colorScheme
    androidx.compose.foundation.layout.Box(modifier) {
        androidx.compose.runtime.CompositionLocalProvider(androidx.compose.material3.LocalMinimumInteractiveComponentSize provides androidx.compose.ui.unit.Dp.Unspecified) {
            Surface(onClick = { open = true }, shape = PillShape, color = scheme.surface, contentColor = scheme.onSurface, border = hairlineBorder()) {
                Row(Modifier.height(34.dp).padding(start = 14.dp, end = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(say.bandKind(kind), Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelLarge.copy(fontSize = 14.sp), maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    Icon(Iconoir.NavArrowDown, null, Modifier.padding(start = 4.dp).size(16.dp), tint = scheme.onSurfaceVariant)
                }
            }
        }
        NoriPopover(open, { open = false }) {
            EqKind.entries.forEach { k -> PopoverItem(say.bandKind(k), k == kind) { open = false; if (k != kind) onPick(k) } }
        }
    }
}

/**
 * A band's value as a number at the end of its row. A tap makes it typed, with the number keys; Done or leaving it hands the
 * words to [onTyped], which reads them through the core (`settings::band_typed`): words it cannot read
 * leave the value as it was.
 */
@Composable
private fun ValueField(shown: String, enabled: Boolean, modifier: Modifier = Modifier, onTyped: (String) -> Unit) {
    val scheme = MaterialTheme.colorScheme
    var editing by remember { mutableStateOf(false) }
    var text by remember { mutableStateOf(androidx.compose.ui.text.input.TextFieldValue()) }
    var focused by remember { mutableStateOf(false) }
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    val done = {
        if (editing) {
            editing = false
            focused = false
            onTyped(text.text)
        }
    }
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(10.dp)
    Column(
        modifier
            .clip(shape)
            .background(scheme.surface)
            .border(1.dp, if (editing) scheme.primary else scheme.outlineVariant, shape)
            .clickable(enabled = enabled && !editing) {
                text = androidx.compose.ui.text.input.TextFieldValue(shown, androidx.compose.ui.text.TextRange(0, shown.length))
                editing = true
            }
            .padding(horizontal = 10.dp, vertical = 7.dp),
    ) {
        val style = MaterialTheme.typography.bodyMedium.copy(
            color = if (enabled) scheme.onSurface else scheme.onSurfaceVariant,
            textAlign = androidx.compose.ui.text.style.TextAlign.End,
            fontFeatureSettings = "tnum",
        )
        if (editing) {
            androidx.compose.foundation.text.BasicTextField(
                text, { text = it },
                Modifier.fillMaxWidth().focusRequester(focus).onFocusChanged { f ->
                    if (f.isFocused) focused = true else if (focused) done()
                },
                singleLine = true, textStyle = style,
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Decimal,
                    imeAction = androidx.compose.ui.text.input.ImeAction.Done,
                ),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { done() }),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(scheme.primary),
            )
            androidx.compose.runtime.LaunchedEffect(Unit) { focus.requestFocus() }
        } else Text(shown, Modifier.fillMaxWidth(), style = style, maxLines = 1)
    }
}

/** How wide a row's number is: room for "+12.5" or "20.0k" and the box round it. */
private val NUMBER_W = 76.dp

/** One of a band's values: its name, a slider for a coarse move, and its number at the end, typed on a tap. */
@Composable
private fun ValueRow(
    name: String, shown: String, at: Float, range: ClosedFloatingPointRange<Float>, enabled: Boolean,
    centred: Boolean = false, onSlide: (Float) -> Unit, onTyped: (String) -> Unit,
) {
    Row(Modifier.fillMaxWidth().padding(start = GROUP_INSET, end = GROUP_INSET, top = 2.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(name, Modifier.width(84.dp), style = MaterialTheme.typography.bodyMedium, maxLines = 1)
        NoriSlider(at, range, onSlide, Modifier.weight(1f), enabled = enabled, centred = centred)
        Spacer(Modifier.width(10.dp))
        ValueField(shown, enabled, Modifier.width(NUMBER_W), onTyped)
    }
}

/** Where the curve is read, 20 Hz to 20 kHz on a log scale. */
private val CURVE_FREQS: List<Float> = List(97) { 20f * Math.pow(1000.0, it / 96.0).toFloat() }

/** How far the curve's card reaches up and down, dB. */
private const val CURVE_DB = 15f

/** A point to drag on the curve: across, 0 to 1 over 20 Hz-20 kHz on a log scale, and its level in dB. */
private class CurvePoint(val x: Float, val db: Float)

/**
 * The curve an equalizer plays, on a card with its grid, and [points] to drag. A press takes the point
 * nearest it ([nearestAcross]: by frequency alone, for bands too close to tell apart by touch), and a drag
 * moves it through [onDrag] (across, dB); a press that does not move is [onTap]. [faint] is a second curve
 * drawn quietly: a headphone correction, or the right channel.
 */
@Composable
private fun CurveCard(
    curve: List<Float>, faint: List<Float>, points: List<CurvePoint>, enabled: Boolean,
    label: (Int) -> String, nearestAcross: Boolean,
    onDrag: (Int, Float, Float) -> Unit, onTap: ((Int) -> Unit)? = null,
    onEmpty: ((Float, Float) -> Unit)? = null, shapes: List<List<Float>> = emptyList(), chosen: Int = -1,
) {
    val scheme = MaterialTheme.colorScheme
    val ink = if (enabled) scheme.onSurface else scheme.onSurfaceVariant
    val accent = scheme.primary
    val surface = scheme.surfaceContainer
    val measurer = androidx.compose.ui.text.rememberTextMeasurer()
    val small = MaterialTheme.typography.labelSmall.copy(fontSize = 10.sp, color = scheme.onSurfaceVariant)
    val bubbleStyle = MaterialTheme.typography.labelMedium.copy(color = scheme.onPrimary, fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold)
    var held by remember { mutableStateOf(-1) }
    val latest = androidx.compose.runtime.rememberUpdatedState(points)
    val drag = androidx.compose.runtime.rememberUpdatedState(onDrag)
    val tap = androidx.compose.runtime.rememberUpdatedState(onTap)
    val empty = androidx.compose.runtime.rememberUpdatedState(onEmpty)
    val axis = remember { listOf(100f, 1000f, 10000f).map { EqBands.position(it) to dev.nori.music.text.Fmt.isoBand(it) } }
    GroupCard {
        androidx.compose.foundation.Canvas(
            Modifier.fillMaxWidth().height(216.dp).padding(horizontal = 14.dp, vertical = 12.dp)
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    val freqRoom = 16.dp.toPx()
                    fun plotHeight() = size.height - freqRoom
                    fun xOf(px: Float) = px / size.width
                    fun dbOf(py: Float) = (1f - 2f * py / plotHeight()) * CURVE_DB
                    fun nearest(at: androidx.compose.ui.geometry.Offset): Int {
                        val ps = latest.value
                        val reach = 40.dp.toPx()
                        val dy = { i: Int -> (1f - ps[i].db / CURVE_DB) * plotHeight() / 2f - at.y }
                        return ps.indices.minByOrNull { i ->
                            val dx = ps[i].x * size.width - at.x
                            if (nearestAcross) kotlin.math.abs(dx) else kotlin.math.hypot(dx, dy(i))
                        }?.takeIf { i ->
                            // Graphic bands stand closer than a finger: the nearest across is taken, but only
                            // near its height, so a swipe elsewhere on the curve still scrolls the page.
                            if (nearestAcross) kotlin.math.abs(dy(i)) < reach else kotlin.math.hypot(ps[i].x * size.width - at.x, dy(i)) < reach
                        } ?: -1
                    }
                    awaitEachGesture {
                        val down = awaitFirstDown()
                        val i = nearest(down.position)
                        if (i < 0) {
                            // Away from every point: a tap there is a new band, a drag is the page's scroll.
                            val up = waitForUpOrCancellation() ?: return@awaitEachGesture
                            if ((up.position - down.position).getDistance() < viewConfiguration.touchSlop && up.position.y < plotHeight()) {
                                empty.value?.invoke(xOf(up.position.x).coerceIn(0f, 1f), dbOf(up.position.y))
                            }
                            return@awaitEachGesture
                        }
                        held = i
                        val start = latest.value[i]
                        val grabbed = androidx.compose.ui.geometry.Offset(start.x * size.width, (1f - start.db / CURVE_DB) * plotHeight() / 2f) - down.position
                        var moved = false
                        val slop = viewConfiguration.touchSlop
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) break
                            if (!moved && (change.position - down.position).getDistance() > slop) moved = true
                            if (moved) {
                                change.consume()
                                val at = change.position + grabbed
                                // A finger moving up or down wavers sideways: under the slop the band stays
                                // where it was across, keeping its frequency.
                                val across = if (kotlin.math.abs(change.position.x - down.position.x) < slop) start.x else xOf(at.x)
                                drag.value(i, across, dbOf(at.y))
                            }
                        }
                        if (!moved) tap.value?.invoke(i)
                        held = -1
                    }
                },
        ) {
            val freqRoom = 16.dp.toPx()
            val h = size.height - freqRoom
            val mid = h / 2f
            fun y(db: Float) = mid - db.coerceIn(-CURVE_DB, CURVE_DB) / CURVE_DB * mid
            val grid = ink.copy(alpha = 0.08f)
            for (db in listOf(-12f, -6f, 6f, 12f)) drawLine(grid, androidx.compose.ui.geometry.Offset(0f, y(db)), androidx.compose.ui.geometry.Offset(size.width, y(db)), 1f)
            drawLine(ink.copy(alpha = 0.2f), androidx.compose.ui.geometry.Offset(0f, mid), androidx.compose.ui.geometry.Offset(size.width, mid), 1f)
            for ((x, text) in axis) {
                drawLine(grid, androidx.compose.ui.geometry.Offset(x * size.width, 0f), androidx.compose.ui.geometry.Offset(x * size.width, h), 1f)
                val t = measurer.measure(text, small)
                drawText(t, topLeft = androidx.compose.ui.geometry.Offset(x * size.width - t.size.width / 2f, size.height - t.size.height))
            }
            for (db in listOf(12f, -12f)) {
                val t = measurer.measure(dev.nori.music.text.Fmt.signedDb(db).substringBefore('.'), small)
                drawText(t, topLeft = androidx.compose.ui.geometry.Offset(2f, y(db) - t.size.height - 1f))
            }
            fun trace(values: List<Float>): androidx.compose.ui.graphics.Path {
                val path = androidx.compose.ui.graphics.Path()
                values.forEachIndexed { i, db ->
                    val x = size.width * i / (values.size - 1).coerceAtLeast(1)
                    if (i == 0) path.moveTo(x, y(db)) else path.lineTo(x, y(db))
                }
                return path
            }
            val stroke = { w: Float -> androidx.compose.ui.graphics.drawscope.Stroke(width = w, cap = androidx.compose.ui.graphics.StrokeCap.Round, join = androidx.compose.ui.graphics.StrokeJoin.Round) }
            if (faint.isNotEmpty()) drawPath(trace(faint), ink.copy(alpha = 0.35f), style = stroke(1.5.dp.toPx()))
            // Each band's own shape, quietly; the chosen one in the accent.
            shapes.forEachIndexed { i, shape ->
                if (i == chosen || i == held) return@forEachIndexed
                drawPath(trace(shape), ink.copy(alpha = 0.22f), style = stroke(1.dp.toPx()))
            }
            shapes.getOrNull(if (held >= 0) held else chosen)?.let { shape ->
                val line = trace(shape)
                val area = androidx.compose.ui.graphics.Path().apply { addPath(line); lineTo(size.width, mid); lineTo(0f, mid); close() }
                drawPath(area, accent.copy(alpha = 0.2f))
            }
            if (curve.isNotEmpty()) {
                val line = trace(curve)
                val area = androidx.compose.ui.graphics.Path().apply {
                    addPath(line); lineTo(size.width, mid); lineTo(0f, mid); close()
                }
                drawPath(area, ink.copy(alpha = 0.07f))
                drawPath(line, ink, style = stroke(2.dp.toPx()))
            }
            val r = if (points.size > 16) 3.5.dp.toPx() else 5.dp.toPx()
            points.forEachIndexed { i, pt ->
                if (i == held) return@forEachIndexed
                val c = androidx.compose.ui.geometry.Offset(pt.x * size.width, y(pt.db))
                drawCircle(surface, r + 1.5.dp.toPx(), c)
                drawCircle(if (i == chosen) accent else ink, r, c)
            }
            points.getOrNull(held)?.let { pt ->
                val c = androidx.compose.ui.geometry.Offset(pt.x * size.width, y(pt.db))
                drawLine(accent.copy(alpha = 0.5f), androidx.compose.ui.geometry.Offset(c.x, 0f), androidx.compose.ui.geometry.Offset(c.x, h), 1.dp.toPx())
                drawCircle(surface, 9.dp.toPx(), c)
                drawCircle(accent, 7.5.dp.toPx(), c)
                // What the held point is and where it sits, along the top of the curve above it: anywhere
                // nearer, the finger holding the point covers it.
                val t = measurer.measure("${label(held)}  ${dev.nori.music.text.Fmt.signedDb(pt.db)}", bubbleStyle)
                val pad = 8.dp.toPx()
                val w = t.size.width + pad * 2
                val bh = t.size.height + pad
                val left = (c.x - w / 2f).coerceIn(0f, size.width - w)
                val top = 0f
                drawRoundRect(accent, androidx.compose.ui.geometry.Offset(left, top), androidx.compose.ui.geometry.Size(w, bh), androidx.compose.ui.geometry.CornerRadius(bh / 2f))
                drawText(t, topLeft = androidx.compose.ui.geometry.Offset(left + pad, top + pad / 2f))
            }
        }
    }
}

/**
 * Proof that the limiter is working: what it is pulling back, right now. Polled only while this screen
 * is on top, so it costs nothing the rest of the time. Its own scope, so each reading redraws this one
 * line rather than recomposing the whole screen every 120 ms.
 */
@Composable
private fun LimiterReduction() {
    val reduction = meterReading(viewModel<SettingsViewModel>().limiterDb)
    // The words change only with the tenth of a dB they show; the meter moves far more finely.
    Text(
        remember(kotlin.math.round(reduction * 10f)) { say.reduction(reduction) },
        style = MaterialTheme.typography.labelMedium,
        color = if (reduction > 0.05f) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun ImportDialog(vm: SettingsViewModel, onDone: () -> Unit) {
    var text by remember { mutableStateOf("") }
    var error by remember { mutableStateOf(false) }
    AlertCard(
        title = { Text(say.importPreset) },
        text = {
            Column {
                Text(say.importPresetHint, style = MaterialTheme.typography.bodySmall)
                FormField(text, { text = it; error = false }, Modifier.fillMaxWidth().padding(top = 8.dp), minLines = 5, placeholder = { Text(say.importPresetExample) })
                if (error) Text(say.noFiltersFound, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = { TextAction(say.import, { if (vm.importPreset(text) > 0) onDone() else error = true }) },
        dismissButton = { TextAction(say.cancel, onDone) },
    )
}
