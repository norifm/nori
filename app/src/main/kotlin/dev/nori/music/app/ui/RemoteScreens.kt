package dev.nori.music.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.IconButton
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.sp
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.R
import dev.nori.music.app.vm.RemoteViewModel
import dev.nori.music.ffi.remote.DeviceKind
import dev.nori.music.ffi.remote.Refusal
import dev.nori.music.ffi.remote.Role

/** A device's glyph. */
internal fun kindIcon(kind: DeviceKind): ImageVector = when (kind) {
    DeviceKind.PHONE -> Iconoir.SmartphoneDevice
    DeviceKind.DESKTOP -> Iconoir.Computer
    DeviceKind.TERMINAL -> Iconoir.Terminal
    DeviceKind.GUEST -> Iconoir.User
}

@Composable
internal fun words(id: Int, vararg args: Any): String {
    val r = LocalContext.current.resources
    return remember(id, *args) { r.getString(id, *args) }
}

@Composable
internal fun refusal(r: Refusal): String = words(
    when (r) {
        Refusal.STALE -> R.string.devices_stale
        Refusal.NOT_ALLOWED -> R.string.devices_not_allowed
        Refusal.UNKNOWN -> R.string.devices_unknown
        Refusal.TOO_MANY -> R.string.jam_too_many
        Refusal.OTHER_SOURCE -> R.string.devices_other_source
    },
)

/** The song a state is on, if it lists it. */

/**
 * Where the music plays: this phone, or one of the account's other devices with nori (one tap moves the
 * playback there, and this phone then shows and controls it), the phone's own audio output (Android's
 * picker), and the jam: "Start a Jam", or "Your Jam" with End while one is on. Up only while remote control
 * or jams are on.
 */
private enum class RoomPage { MAIN, INVITE, SETTINGS, PAIR, DEVICE }

@Composable
fun DevicesSheet(open: Boolean, onDismiss: () -> Unit, onOutput: () -> Unit, jams: Boolean, remote: Boolean, sound: dev.nori.music.app.vm.SettingsViewModel) {
    val vm: RemoteViewModel = viewModel()
    val plugins: dev.nori.music.app.vm.PluginsViewModel = viewModel()
    val nav = LocalNav.current
    NoriSheet(open, onDismiss) {
        // Other devices are looked for only with remote control on; a plugin's rows alone need none of it.
        if (remote) LifecycleResumeEffect(Unit) { vm.watch(true); onPauseOrDispose { vm.watch(false) } }
        LifecycleResumeEffect(Unit) { plugins.outputPicker(true); onPauseOrDispose { plugins.outputPicker(false) } }
        val devices by vm.devices.collectAsStateWithLifecycle()
        val names by vm.names.collectAsStateWithLifecycle()
        val jam by vm.jam.collectAsStateWithLifecycle()
        val relay by vm.relay.collectAsStateWithLifecycle()
        val mirror by vm.mirror.collectAsStateWithLifecycle()
        val pairing by vm.pairingLink.collectAsStateWithLifecycle()
        val unsupported = relay == dev.nori.music.ffi.RelaySupport.UNSUPPORTED
        val cast by sound.pluginCast.collectAsStateWithLifecycle()
        val pick = { device: String? ->
            if (device == null && cast != null) sound.endCast()
            vm.pick(device)
            onDismiss()
        }
        val hosted = jam?.takeIf { it.hosting }
        var page by remember(hosted?.link) { mutableStateOf(RoomPage.MAIN) }
        BackHandler(page != RoomPage.MAIN) { page = RoomPage.MAIN }
        val controlled by plugins.controlled.collectAsStateWithLifecycle()
        val output by sound.currentOutput.collectAsStateWithLifecycle()
        // The headphones playing now, when a plugin controls them; their page while it is open.
        val here = remember(controlled, output) { dev.nori.music.ffi.plugins.controlledOutput(controlled, dev.nori.music.ffi.devices.outputLook(output).name) }
        var shownDevice by remember { mutableStateOf<String?>(null) }
        val pageDevice = controlled.firstOrNull { it.id == shownDevice }
        // Its page goes with the device.
        LaunchedEffect(pageDevice == null) { if (pageDevice == null && page == RoomPage.DEVICE) page = RoomPage.MAIN }
        Column(Modifier.verticalScroll(rememberScrollState()).windowInsetsPadding(gestureBarRoom)) {
            if (page == RoomPage.DEVICE && pageDevice != null) {
                SheetTitle(pageDevice.name)
                ControlledDevicePage(plugins, pageDevice)
            } else if (page == RoomPage.PAIR && pairing != null) {
                SheetTitle(words(R.string.pair_devices))
                PairingBody(pairing!!, vm) { page = RoomPage.MAIN }
            } else if (hosted != null && page != RoomPage.MAIN) {
                SheetTitle(words(if (page == RoomPage.INVITE) R.string.jam_invite_title else R.string.settings))
                if (page == RoomPage.INVITE) hosted.link?.let { InviteBody(it) } else JamSettings(hosted)
            } else {
                SheetTitle(words(R.string.devices_title))
                here?.let { d -> DeviceControls(plugins, d, Modifier.padding(bottom = 12.dp)) { shownDevice = d.id; page = RoomPage.DEVICE } }
                GroupCard {
                    DeviceRow(words(R.string.devices_this), kindIcon(DeviceKind.PHONE), null, mirror == null && cast == null, divider = cast != null || hosted == null && devices.isNotEmpty()) { pick(null) }
                    // A plugin's device, playing: picking this phone above ends the cast.
                    cast?.let { c ->
                        val what = words(if (c.kind == dev.nori.music.ffi.plugins.CastKind.SINK) R.string.devices_casting else R.string.devices_cast_target)
                        DeviceRow(c.device, Iconoir.SoundHigh, what, true, divider = hosted == null && devices.isNotEmpty(), trailing = c.volume?.let { "$it %" }) {}
                    }
                    if (hosted == null) devices.forEachIndexed { k, d ->
                        val now = d.now
                        DeviceRow(
                            names.getOrNull(k) ?: d.name, kindIcon(d.kind),
                            now?.let { "${it.title} · ${it.artist}" } ?: words(R.string.devices_idle),
                            mirror?.id == d.id, k < devices.lastIndex, if (d.nearby) words(R.string.devices_nearby) else null,
                        ) { pick(d.id) }
                    }
                }
                if (hosted == null) {
                    mirror?.refused?.let { Text(refusal(it), Modifier.padding(horizontal = Space.gutter, vertical = 6.dp), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
                    if (unsupported) Text(
                        words(R.string.devices_nearby_only), Modifier.padding(horizontal = Space.gutter, vertical = 12.dp),
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val start by vm.jamStarting.collectAsStateWithLifecycle()
                val offered = jams && hosted == null && start == dev.nori.music.ffi.JamStart.OFFERED
                // The phone's own speaker, headphones or Bluetooth: Android's picker. The speaker's outline, as the
                // player's button has it idle: a filled glyph is one lit in the accent.
                GroupCard(Modifier.padding(top = 12.dp)) {
                    SheetRow(words(R.string.devices_output), Iconoir.SoundHigh, divider = pairing != null || offered || (jams && hosted != null), onClick = onOutput)
                    if (pairing != null) SheetRow(words(R.string.pair_devices), Iconoir.SmartphoneDevice, divider = offered || (jams && hosted != null), onClick = { page = RoomPage.PAIR })
                    if (offered) SheetRow(words(R.string.jam_start), Iconoir.Community, divider = false, onClick = vm::jamStart)
                    if (jams && hosted != null) YourJam(hosted.members.count { it.role != Role.HOST }, { onDismiss(); nav.player(Panel.QUEUE) }, vm::jamEnd)
                }
                if (jams && hosted == null) {
                    if (start == dev.nori.music.ffi.JamStart.UNSUPPORTED) Text(
                        words(R.string.jam_unsupported), Modifier.padding(horizontal = Space.gutter, vertical = 12.dp),
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    ) else if (start == dev.nori.music.ffi.JamStart.STARTING) Text(
                        words(R.string.jam_starting), Modifier.padding(horizontal = Space.gutter, vertical = 12.dp),
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val ms = if (reduceMotion()) 0 else 220
                AnimatedVisibility(jams && hosted != null, enter = fadeIn(tween(ms)) + expandVertically(tween(ms)), exit = fadeOut(tween(ms)) + shrinkVertically(tween(ms))) {
                    hosted?.let { JamControls(it, { page = RoomPage.INVITE }, { page = RoomPage.SETTINGS }) }
                }
                // Plugins' rows for this phone's devices, after nori's own.
                PluginDeviceSections(viewModel(), sound)
            }
        }
    }
}

/** An incoming pairing code waits for an explicit choice. */
@Composable
internal fun PairingRequest(vm: RemoteViewModel = viewModel()) {
    val request by vm.pairRequest.collectAsStateWithLifecycle()
    val tell = LocalMessages.current
    val paired = words(R.string.pair_done)
    val failed = words(R.string.pair_failed)
    NoriDialog(request, vm::dismissPairing) { link ->
        AlertCard(
            title = { Text(words(R.string.pair_confirm)) },
            text = { Text(words(R.string.pair_hint)) },
            confirmButton = { TextAction(words(R.string.pair_action), { vm.pair(link) { ok -> tell(if (ok) paired else failed); vm.dismissPairing() } }) },
            dismissButton = { TextAction(words(R.string.cancel), vm::dismissPairing) },
        )
    }
}

@Composable
private fun PairingBody(link: String, vm: RemoteViewModel, onPaired: () -> Unit) {
    var entered by remember { mutableStateOf("") }
    var resetting by remember { mutableStateOf(false) }
    val tell = LocalMessages.current
    val paired = words(R.string.pair_done)
    val failed = words(R.string.pair_failed)
    LinkCode(link, words(R.string.pair_hint))
    Column(Modifier.padding(horizontal = Space.gutter), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        FormField(entered, { entered = it }, Modifier.fillMaxWidth(), label = { Text(words(R.string.pair_link)) }, singleLine = true)
        PillButton(words(R.string.pair_action), Iconoir.SmartphoneDevice, {
            vm.pair(entered) { ok -> tell(if (ok) paired else failed); if (ok) onPaired() }
        }, enabled = dev.nori.music.ffi.pairingValid(entered))
        TextAction(words(R.string.pair_reset), { resetting = true })
    }
    NoriDialog(resetting, { resetting = false }) {
        AlertCard(
            title = { Text(words(R.string.pair_reset)) }, text = { Text(words(R.string.pair_reset_hint)) },
            confirmButton = { TextAction(words(R.string.pair_reset), { vm.resetPairing(); resetting = false }, destructive = true) },
            dismissButton = { TextAction(words(R.string.cancel), { resetting = false }) },
        )
    }
}

/** The devices sheet, opened from the player's output button and the bar's speaker ([LocalDevices]). */
@Composable
fun DevicesHost(open: Boolean, onDismiss: () -> Unit) {
    val settings: dev.nori.music.app.vm.SettingsViewModel = viewModel()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val output by settings.currentOutput.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val tell = LocalMessages.current
    val pluginDevices by settings.pluginDevices.collectAsStateWithLifecycle()
    val controlled by viewModel<dev.nori.music.app.vm.PluginsViewModel>().controlled.collectAsStateWithLifecycle()
    val cast by settings.pluginCast.collectAsStateWithLifecycle()
    val remote = prefs.remoteControl && LocalRules.current.remote
    if (remote || pluginDevices.isNotEmpty() || controlled.isNotEmpty() || cast != null) DevicesSheet(open, onDismiss, { onDismiss(); openOutputPicker(context, output, tell) }, prefs.remoteControl, remote, settings)
}

/** The jam this phone hosts, as the devices list it: a tap opens its queue, End ends it. */
@Composable
private fun YourJam(listening: Int, onOpen: () -> Unit, onEnd: () -> Unit) {
    GroupRow(
        words(R.string.jam_yours), onOpen, subtitle = words(R.string.jam_strip, jamListening(listening)), icon = Iconoir.Community, divider = false,
        trailing = {
            IconButton(onEnd) { Icon(Iconoir.LogOut, words(R.string.jam_end_short), Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurface) }
        },
    )
}

/** A row of the sheet that is not a device. */
@Composable
private fun SheetRow(name: String, icon: ImageVector, divider: Boolean, onClick: () -> Unit) = GroupRow(name, onClick, icon = icon, divider = divider)

@Composable
private fun SheetTitle(title: String) = Text(
    title, Modifier.padding(horizontal = Space.gutter).padding(top = 4.dp, bottom = 12.dp),
    style = MaterialTheme.typography.titleMedium.copy(fontSize = 17.sp, fontWeight = FontWeight.SemiBold), maxLines = 1, overflow = TextOverflow.Ellipsis,
)

/** A place the music can play: the one playing has its name and glyph in the accent, the rest are ink. */
@Composable
private fun DeviceRow(name: String, icon: ImageVector, subtitle: String?, active: Boolean, divider: Boolean, trailing: String? = null, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.semantics { selected = active }) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable(onClick = onClick).padding(horizontal = 14.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, Modifier.size(24.dp), tint = if (active) scheme.primary else scheme.onSurface)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(name, style = MaterialTheme.typography.bodyLarge, color = if (active) scheme.primary else scheme.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (!subtitle.isNullOrEmpty()) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (trailing != null) Text(trailing, Modifier.padding(start = 10.dp), style = MaterialTheme.typography.bodyMedium.copy(fontSize = 13.5.sp), color = scheme.onSurfaceVariant)
        }
        if (divider) Hairline(startIndent = 52.dp)
    }
}
