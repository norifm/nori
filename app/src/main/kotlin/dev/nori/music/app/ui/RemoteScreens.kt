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
import androidx.compose.material.icons.Icons
import androidx.compose.material3.IconButton
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Groups
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material.icons.outlined.Speaker
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Terminal
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
import dev.nori.music.ffi.remote.DeviceState
import dev.nori.music.ffi.remote.Entry
import dev.nori.music.ffi.remote.Refusal
import dev.nori.music.ffi.remote.Role

/** A device's glyph: filled for the one playing, outlined for the rest. */
private fun kindIcon(kind: DeviceKind, filled: Boolean): ImageVector = when (kind) {
    DeviceKind.PHONE -> if (filled) Icons.Filled.PhoneAndroid else Icons.Outlined.PhoneAndroid
    DeviceKind.DESKTOP -> if (filled) Icons.Filled.Computer else Icons.Outlined.Computer
    DeviceKind.TERMINAL -> if (filled) Icons.Filled.Terminal else Icons.Outlined.Terminal
    DeviceKind.GUEST -> if (filled) Icons.Filled.Person else Icons.Outlined.Person
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
    },
)

/** The song a state is on, if it lists it. */
private fun DeviceState.current(): Entry? = entries.firstOrNull { it.index == index }

/**
 * Where the music plays: this phone, or one of the account's other devices with nori (one tap moves the
 * playback there, and this phone then shows and controls it), the phone's own audio output (Android's
 * picker), and the jam: "Start a Jam", or "Your Jam" with End while one is on. Up only while remote control
 * or jams are on.
 */
private enum class RoomPage { MAIN, INVITE, SETTINGS, PAIR }

@Composable
fun DevicesSheet(open: Boolean, onDismiss: () -> Unit, onOutput: () -> Unit, jams: Boolean) {
    val vm: RemoteViewModel = viewModel()
    val nav = LocalNav.current
    NoriSheet(open, onDismiss) {
        LifecycleResumeEffect(Unit) { vm.watch(true); onPauseOrDispose { vm.watch(false) } }
        val devices by vm.devices.collectAsStateWithLifecycle()
        val names by vm.names.collectAsStateWithLifecycle()
        val jam by vm.jam.collectAsStateWithLifecycle()
        val relay by vm.relay.collectAsStateWithLifecycle()
        val mirror by vm.mirror.collectAsStateWithLifecycle()
        val pairing by vm.pairingLink.collectAsStateWithLifecycle()
        val unsupported = relay == dev.nori.music.ffi.RelaySupport.UNSUPPORTED
        val pick = { device: String? -> vm.pick(device); onDismiss() }
        val hosted = jam?.takeIf { it.hosting }
        var page by remember(hosted?.link) { mutableStateOf(RoomPage.MAIN) }
        BackHandler(page != RoomPage.MAIN) { page = RoomPage.MAIN }
        Column(Modifier.verticalScroll(rememberScrollState()).windowInsetsPadding(gestureBarRoom)) {
            if (page == RoomPage.PAIR && pairing != null) {
                SheetBack(words(R.string.pair_devices)) { page = RoomPage.MAIN }
                PairingBody(pairing!!, vm) { page = RoomPage.MAIN }
            } else if (hosted != null && page != RoomPage.MAIN) {
                SheetBack(words(if (page == RoomPage.INVITE) R.string.jam_invite_title else R.string.settings)) { page = RoomPage.MAIN }
                if (page == RoomPage.INVITE) hosted.link?.let { InviteBody(it) } else JamSettings(hosted)
            } else {
                Text(
                    words(R.string.devices_title), Modifier.padding(horizontal = Space.gutter).padding(top = 4.dp, bottom = 12.dp),
                    style = MaterialTheme.typography.titleMedium.copy(fontSize = 17.sp, fontWeight = FontWeight.SemiBold),
                )
                GroupCard {
                    DeviceRow(words(R.string.devices_this), kindIcon(DeviceKind.PHONE, mirror == null), null, mirror == null, divider = hosted == null && devices.isNotEmpty()) { pick(null) }
                    if (hosted == null) devices.forEachIndexed { k, d ->
                        val now = d.state?.current()
                        DeviceRow(
                            names.getOrNull(k) ?: d.name, kindIcon(d.kind, mirror?.id == d.id),
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
                    SheetRow(words(R.string.devices_output), Icons.Outlined.Speaker, divider = pairing != null || offered || (jams && hosted != null), onClick = onOutput)
                    if (pairing != null) SheetRow(words(R.string.pair_devices), Icons.Outlined.PhoneAndroid, divider = offered || (jams && hosted != null), onClick = { page = RoomPage.PAIR })
                    if (offered) SheetRow(words(R.string.jam_start), Icons.Filled.Groups, divider = false, onClick = vm::jamStart)
                    if (jams && hosted != null) YourJam(hosted.members.count { it.role != Role.HOST }, { onDismiss(); nav.player(Panel.QUEUE) }, vm::jamEnd)
                }
                if (jams && hosted == null) {
                    if (unsupported && start == dev.nori.music.ffi.JamStart.UNAVAILABLE) Text(
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
        PillButton(words(R.string.pair_action), Icons.Filled.PhoneAndroid, {
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
    if (prefs.remoteControl && LocalRules.current.remote) DevicesSheet(open, onDismiss, { onDismiss(); openOutputPicker(context, output, tell) }, prefs.remoteControl)
}

/** The jam this phone hosts, as the devices list it: a tap opens its queue, End ends it. */
@Composable
private fun YourJam(listening: Int, onOpen: () -> Unit, onEnd: () -> Unit) {
    GroupRow(
        words(R.string.jam_yours), onOpen, subtitle = words(R.string.jam_strip, jamListening(listening)), icon = Icons.Filled.Groups, divider = false,
        trailing = {
            IconButton(onEnd) { Icon(Icons.AutoMirrored.Filled.Logout, words(R.string.jam_end_short), Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurface) }
        },
    )
}

/** A row of the sheet that is not a device. */
@Composable
private fun SheetRow(name: String, icon: ImageVector, divider: Boolean, onClick: () -> Unit) = GroupRow(name, onClick, icon = icon, divider = divider)

/** A sub-page's head: a bare back glyph and its title. */
@Composable
private fun SheetBack(title: String, onBack: () -> Unit) {
    Row(Modifier.padding(start = 4.dp, end = Space.gutter), verticalAlignment = Alignment.CenterVertically) {
        GlyphButton(Icons.AutoMirrored.Filled.ArrowBack, words(R.string.back), onClick = onBack)
        Text(title, Modifier.padding(start = 4.dp), style = MaterialTheme.typography.titleMedium.copy(fontSize = 17.sp, fontWeight = FontWeight.SemiBold), maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** A place the music can play: the one playing has its name in the accent and a filled glyph, the rest are ink. */
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
