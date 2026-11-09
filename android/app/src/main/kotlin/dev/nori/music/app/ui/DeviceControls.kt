package dev.nori.music.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorProducer
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.PlayerViewModel
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.plugins.ControlledDevice
import dev.nori.music.ffi.plugins.Noise
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

/**
 * A controlled device's known controls: its name with its batteries under it, its noise modes, and a way on to
 * its page. On that page ([onOpen] null) the page already names it, so only the batteries show.
 */
@Composable
fun DeviceControls(vm: PluginsViewModel, d: ControlledDevice, modifier: Modifier = Modifier, onOpen: (() -> Unit)? = null) {
    GroupCard(modifier) {
        val battery = say.battery(d.battery)
        if (onOpen != null) GroupRow(d.name, icon = Iconoir.Headset, subtitle = battery, divider = false)
        else if (battery.isNotEmpty()) Text(
            battery, Modifier.padding(horizontal = GROUP_INSET, vertical = 12.dp),
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (d.noiseModes.isNotEmpty()) {
            if (onOpen != null || battery.isNotEmpty()) Hairline(startIndent = GROUP_INSET)
            Row(Modifier.fillMaxWidth().padding(horizontal = GROUP_INSET, vertical = 10.dp), Arrangement.spacedBy(6.dp)) {
                d.noiseModes.forEach { m -> Chip(say.noise(m), m == d.noise, Modifier.weight(1f), sidePadding = 4.dp) { vm.setNoise(d.id, m) } }
            }
        }
        if (onOpen != null) {
            Hairline(startIndent = GROUP_INSET)
            GroupRow(say.deviceSettings, onOpen, chevron = true, divider = false)
        }
    }
}

/** Everything of a controlled device: its known controls, then its settings. */
@Composable
fun ControlledDevicePage(vm: PluginsViewModel, d: ControlledDevice) {
    DeviceControls(vm, d)
    PluginRows(vm, d.card, null) { it.onDevicePage() }
}

/** The headphones the music plays on, when a plugin controls them. */
@Composable
internal fun outputHeadphones(): ControlledDevice? {
    val settings: SettingsViewModel = viewModel()
    val output by settings.currentOutput.collectAsStateWithLifecycle()
    val controlled by viewModel<PluginsViewModel>().controlled.collectAsStateWithLifecycle()
    return remember(controlled, output) {
        if (controlled.isEmpty()) null
        else dev.nori.music.ffi.plugins.controlledOnOutput(controlled, output)
    }
}

/** [outputHeadphones], when they have noise control and the music plays on this phone ([playingOn]: another device). */
@Composable
internal fun noiseHeadphones(playingOn: String?): ControlledDevice? {
    val cast by viewModel<SettingsViewModel>().pluginCast.collectAsStateWithLifecycle()
    return outputHeadphones()?.takeIf { playingOn == null && cast == null && it.noiseModes.isNotEmpty() }
}

/** The next of the headphones' noise modes (the core's `next_noise`). */
private fun PluginsViewModel.nextNoise(d: ControlledDevice) {
    dev.nori.music.ffi.plugins.nextNoise(d.noiseCycle, d.noise)?.let { setNoise(d.id, it) }
}

/** The headphones' noise control as one glyph: a tap moves it to the next mode. */
@Composable
internal fun NoiseButton(d: ControlledDevice, size: Dp, idle: ColorProducer, lit: ColorProducer) {
    val vm: PluginsViewModel = viewModel()
    IconButton({ vm.nextNoise(d) }) {
        LookIcon(noiseGlyph(d.noise), d.noise?.let(say::noise), Modifier.size(size), if (d.noise == null || d.noise == Noise.OFF) idle else lit)
    }
}

/** The headphones' noise control in a title row, beside the seal. */
@Composable
internal fun NoiseTitleButton(player: PlayerViewModel) {
    val state by player.state.collectAsStateWithLifecycle()
    val jam by player.jamStrip.collectAsStateWithLifecycle()
    val vm: PluginsViewModel = viewModel()
    NoiseAppearing(if (jam == null && !state.jamGuest) noiseHeadphones(state.playingOn) else null) { d ->
        TitleButton(noiseGlyph(d.noise), d.noise?.let(say::noise).orEmpty(), false) { vm.nextNoise(d) }
    }
}

/** [content] for the headphones [d] while there are any: it opens its width and fades in, and goes the same way. */
@Composable
internal fun NoiseAppearing(d: ControlledDevice?, content: @Composable (ControlledDevice) -> Unit) {
    val last = remember { arrayOf(d) }
    if (d != null) last[0] = d
    androidx.compose.animation.AnimatedVisibility(d != null, enter = AppMotion.grow(across = true), exit = AppMotion.shrink(across = true)) {
        last[0]?.let { content(it) }
    }
}

private const val RING = 9f

/** A glyph on the icons' 24 grid: a ring, whole or in [dashes], with a disc of [disc] or a ring of [inner] inside. */
private fun ringGlyph(name: String, dashes: Int = 0, disc: Float = 0f, inner: Float = 0f) = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
    val line = SolidColor(Color.Black)
    fun at(degrees: Float) = (degrees * PI / 180).let { Offset(12 + RING * cos(it).toFloat(), 12 + RING * sin(it).toFloat()) }
    path(stroke = line, strokeLineWidth = 1.5f, strokeLineCap = StrokeCap.Round) {
        if (dashes == 0) {
            moveTo(3f, 12f)
            arcTo(RING, RING, 0f, true, true, 21f, 12f)
            arcTo(RING, RING, 0f, true, true, 3f, 12f)
            close()
        } else for (i in 0 until dashes) {
            val from = at(i * 360f / dashes)
            val to = at((i + 0.5f) * 360f / dashes)
            moveTo(from.x, from.y)
            arcTo(RING, RING, 0f, false, true, to.x, to.y)
        }
        if (inner > 0f) {
            moveTo(12 - inner, 12f)
            arcTo(inner, inner, 0f, true, true, 12 + inner, 12f)
            arcTo(inner, inner, 0f, true, true, 12 - inner, 12f)
            close()
        }
    }
    if (disc > 0f) path(fill = line) {
        moveTo(12 - disc, 12f)
        arcTo(disc, disc, 0f, true, true, 12 + disc, 12f)
        arcTo(disc, disc, 0f, true, true, 12 - disc, 12f)
        close()
    }
}.build()

private val NoiseOff by lazy { ringGlyph("noiseOff") }
private val NoiseCanceling by lazy { ringGlyph("noiseCanceling", disc = 4.6f) }
private val NoiseTransparency by lazy { ringGlyph("noiseTransparency", dashes = 8) }
private val NoiseAdaptive by lazy { ringGlyph("noiseAdaptive", inner = 4.6f) }

private fun noiseGlyph(n: Noise?): ImageVector = when (n) {
    null, Noise.OFF -> NoiseOff
    Noise.CANCELING -> NoiseCanceling
    Noise.TRANSPARENCY -> NoiseTransparency
    Noise.ADAPTIVE -> NoiseAdaptive
}
