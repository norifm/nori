package dev.nori.music.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.togetherWith
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.semantics.contentDescription
import kotlinx.coroutines.flow.collectLatest
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
import dev.nori.music.look.CoverLook

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
/**
 * The devices sheet's page: the devices, a hosted jam's invite or settings, the paired devices, pairing a new
 * one (the camera on its code, or its link pasted), this phone's code, or a controlled device's page.
 */
private sealed interface RoomPage {
    data object Main : RoomPage
    data object Invite : RoomPage
    data object Settings : RoomPage
    data object Pair : RoomPage
    data object Code : RoomPage
    data object New : RoomPage
    /** The page of the controlled device [id]. */
    data class Device(val id: String) : RoomPage
}

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
        val destination by vm.destination.collectAsStateWithLifecycle()
        val pairing by vm.pairingLink.collectAsStateWithLifecycle()
        val paired by vm.paired.collectAsStateWithLifecycle()
        val unsupported = relay == dev.nori.music.ffi.RelaySupport.UNSUPPORTED
        val cast by sound.pluginCast.collectAsStateWithLifecycle()
        val targets by sound.pluginTargets.collectAsStateWithLifecycle()
        val reached by vm.outputs.collectAsStateWithLifecycle()
        val pick = { device: String? ->
            // The music goes to one place: wherever it goes now, a cast's device stops.
            if (cast != null) sound.endCast()
            vm.pick(device)
            onDismiss()
        }
        val hosted = jam?.takeIf { it.hosting }
        var page by remember(hosted?.link) { mutableStateOf<RoomPage>(RoomPage.Main) }
        val controlled by plugins.controlled.collectAsStateWithLifecycle()
        val output by sound.currentOutput.collectAsStateWithLifecycle()
        // The headphones playing now, when a plugin controls them; their page while it is open.
        val here = remember(controlled, output) { dev.nori.music.ffi.plugins.controlledOnOutput(controlled, output) }
        val pageDevice = (page as? RoomPage.Device)?.let { p -> controlled.firstOrNull { it.id == p.id } }
        // What is showing: a page whose reason for being is gone is the main one.
        val showing = when {
            pageDevice != null -> page
            (page == RoomPage.Pair || page == RoomPage.Code || page == RoomPage.New) && pairing != null -> page
            hosted != null && (page == RoomPage.Invite || page == RoomPage.Settings) -> page
            else -> RoomPage.Main
        }
        BackHandler(showing != RoomPage.Main) { page = if (showing == RoomPage.Code || showing == RoomPage.New) RoomPage.Pair else RoomPage.Main }
        // One page fades into the next, as the sheet takes the new one's height.
        androidx.compose.animation.AnimatedContent(
            showing,
            transitionSpec = {
                (AppMotion.appear() togetherWith AppMotion.vanish()).using(
                    androidx.compose.animation.SizeTransform(clip = false) { _, _ -> AppMotion.settle() },
                )
            },
            label = "devices page",
        ) { p ->
        // Leaving because its device went, the page keeps drawing the device while it fades.
        val heldDevice = remember { arrayOfNulls<dev.nori.music.ffi.plugins.ControlledDevice>(1) }
        val shownDevice = (p as? RoomPage.Device)?.let { d -> controlled.firstOrNull { it.id == d.id } ?: heldDevice[0] }
        heldDevice[0] = shownDevice
        // Kept below the status bar: a sheet reaching under it has its content pushed back down as it arrives.
        val density = androidx.compose.ui.platform.LocalDensity.current
        val window = androidx.compose.ui.platform.LocalWindowInfo.current.containerSize.height
        val tallest = with(density) { (window - statusBarRoom.getTop(density) - gestureBarRoom.getBottom(density)).toDp() } - SHEET_ROOM
        val scroll = rememberScrollState()
        val paper = MaterialTheme.colorScheme.surfaceContainer
        Column(Modifier.heightIn(max = tallest).fadesUnderTitle(scroll, paper).fadesAtFoot(scroll, paper).verticalScroll(scroll).windowInsetsPadding(gestureBarRoom)) {
            if (shownDevice != null) {
                SheetTitle(shownDevice.name)
                ControlledDevicePage(plugins, shownDevice)
            } else if (p == RoomPage.Pair && pairing != null) {
                SheetTitle(words(R.string.pair_devices))
                PairedPage(vm, paired, { page = RoomPage.New }, { page = RoomPage.Code })
            } else if (p == RoomPage.Code && pairing != null) {
                SheetTitle(words(R.string.pair_show))
                LinkCode(pairing!!, words(R.string.pair_hint))
            } else if (p == RoomPage.New && pairing != null) {
                SheetTitle(words(R.string.pair_new))
                NewDevicePage(vm) { page = RoomPage.Pair }
            } else if (hosted != null && (p == RoomPage.Invite || p == RoomPage.Settings)) {
                SheetTitle(words(if (p == RoomPage.Invite) R.string.jam_invite_title else R.string.settings))
                if (p == RoomPage.Invite) hosted.link?.let { InviteBody(it) } else JamSettings(hosted)
            } else {
                SheetTitle(words(R.string.devices_title))
                here?.let { d -> DeviceControls(plugins, d, Modifier.padding(bottom = 12.dp)) { page = RoomPage.Device(d.id) } }
                val more = hosted == null && (devices.isNotEmpty() || reached.isNotEmpty())
                GroupCard {
                    DeviceRow(words(R.string.devices_this), kindIcon(DeviceKind.PHONE), null, destination is dev.nori.music.ffi.Destination.Here, divider = cast != null || targets.isNotEmpty() || more) { pick(null) }
                    // A plugin's device, playing: picking this phone above ends the cast.
                    cast?.let { c ->
                        val what = words(if (c.kind == dev.nori.music.ffi.plugins.CastKind.SINK) R.string.devices_casting else R.string.devices_cast_target)
                        DeviceRow(c.device, castIcon(c.look), what, destination is dev.nori.music.ffi.Destination.Cast, divider = targets.isNotEmpty() || more, trailing = c.volume?.let { "$it %" }) {}
                    }
                    // What plugins found on the network, as any other place the music can go.
                    targets.forEachIndexed { k, t ->
                        DeviceRow(t.name, castIcon(t.look), null, false, divider = k < targets.lastIndex || more) {
                            sound.castTo(t)
                            vm.pick(null)
                            onDismiss()
                        }
                    }
                    if (hosted == null) devices.forEachIndexed { k, d ->
                        val now = d.now
                        DeviceRow(
                            names.getOrNull(k) ?: d.name, kindIcon(d.kind),
                            now?.let { "${it.title} · ${it.artist}" } ?: words(R.string.devices_idle),
                            mirror?.id == d.id, k < devices.lastIndex || reached.isNotEmpty(), if (d.nearby) words(R.string.devices_nearby) else null,
                        ) { pick(d.id) }
                    }
                    // A speaker another device casts to: the music goes through that device.
                    if (hosted == null) reached.forEachIndexed { k, r ->
                        DeviceRow(r.output.name, outputIcon(r.output.look), words(R.string.devices_via, r.via), false, k < reached.lastIndex) {
                            if (cast != null) sound.endCast()
                            vm.castOn(r)
                            onDismiss()
                        }
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
                GroupCard {
                    SheetRow(words(R.string.devices_output), Iconoir.SoundHigh, divider = pairing != null || offered || (jams && hosted != null), onClick = onOutput)
                    if (pairing != null) GroupRow(
                        words(R.string.pair_devices), { page = RoomPage.Pair }, icon = Iconoir.SmartphoneDevice, chevron = true,
                        subtitle = paired.joinToString(", ") { it.name }.ifEmpty { words(R.string.pair_none) }, divider = offered || (jams && hosted != null),
                    )
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
                AnimatedVisibility(jams && hosted != null, enter = AppMotion.grow(), exit = AppMotion.shrink()) {
                    hosted?.let { JamControls(it, { page = RoomPage.Invite }, { page = RoomPage.Settings }) }
                }
                // Plugins' rows for this phone's devices, after nori's own.
                PluginDeviceSections(viewModel(), sound)
            }
        }
        }
    }
}

/** What a sheet's handle and foot take, kept out of how tall a page of the devices sheet may be. */
private val SHEET_ROOM = 48.dp

/** An incoming pairing code (a link opened, the system camera's scan) waits for an explicit choice. */
@Composable
internal fun PairingRequest(vm: RemoteViewModel = viewModel()) {
    val request by vm.pairRequest.collectAsStateWithLifecycle()
    val tell = LocalMessages.current
    val r = LocalContext.current.resources
    NoriDialog(request, vm::dismissPairing) { link ->
        val name = remember(link) { dev.nori.music.ffi.pairingName(link) }
        AlertCard(
            title = { Text(if (name != null) words(R.string.pair_confirm, name) else words(R.string.pair_confirm_unnamed)) },
            text = { Text(words(R.string.pair_confirm_hint)) },
            confirmButton = {
                TextAction(words(R.string.pair_action), {
                    vm.pair(link) { result -> tell(result.fold({ r.getString(R.string.pair_done, it.name) }, { say.pairFailed(it, name) })) }
                    vm.dismissPairing()
                })
            },
            dismissButton = { TextAction(words(R.string.cancel), vm::dismissPairing) },
        )
    }
}

/**
 * The devices paired with this phone, each with Forget (asked first); then pairing another, and this phone's
 * own code for another to scan.
 */
@Composable
private fun PairedPage(vm: RemoteViewModel, paired: List<dev.nori.music.ffi.PairedDevice>, onNew: () -> Unit, onCode: () -> Unit) {
    var forgetting by remember { mutableStateOf<dev.nori.music.ffi.PairedDevice?>(null) }
    val scheme = MaterialTheme.colorScheme
    if (paired.isNotEmpty()) GroupCard {
        paired.forEachIndexed { k, d ->
            GroupRow(
                d.name, subtitle = say.pairSeen(d), icon = kindIcon(d.kind), divider = k < paired.lastIndex,
                trailing = {
                    IconButton({ forgetting = d }) { Icon(Iconoir.Xmark, words(R.string.pair_forget), Modifier.size(20.dp), tint = scheme.onSurfaceVariant) }
                },
            )
        }
    }
    GroupCard {
        SheetRow(words(R.string.pair_new), Iconoir.ScanQrCode, divider = true, onClick = onNew)
        SheetRow(words(R.string.pair_show), Iconoir.SmartphoneDevice, divider = false, onClick = onCode)
    }
    Spacer(Modifier.height(24.dp))
    NoriDialog(forgetting, { forgetting = null }) { d ->
        AlertCard(
            title = { Text(words(R.string.pair_forget_title, d.name)) }, text = { Text(words(R.string.pair_forget_hint)) },
            confirmButton = { TextAction(words(R.string.pair_forget), { vm.forget(d.id); forgetting = null }, destructive = true) },
            dismissButton = { TextAction(words(R.string.cancel), { forgetting = null }) },
        )
    }
}

/** Why this page opens on the pasted link: no camera, the camera refused, or asked for. */
private enum class Pasting { NO_CAMERA, DENIED, ASKED }

/**
 * Pairing a new device: the camera on its code, its permission asked for here, at the tap that opened it;
 * "Paste a link instead" turns it into a field for the other device's link. Without a camera, or with the
 * camera refused, the field shows at once. Paired, [onPaired] goes back to the list.
 */
@Composable
private fun NewDevicePage(vm: RemoteViewModel, onPaired: () -> Unit) {
    val context = LocalContext.current
    val scheme = MaterialTheme.colorScheme
    val tell = LocalMessages.current
    val camera = remember { context.packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY) }
    var allowed by remember { mutableStateOf(androidx.core.content.ContextCompat.checkSelfPermission(context, android.Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED) }
    var pasting by remember { mutableStateOf(if (camera) null else Pasting.NO_CAMERA) }
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted ->
        allowed = granted
        if (!granted) pasting = Pasting.DENIED
    }
    LaunchedEffect(Unit) { if (camera && !allowed) ask.launch(android.Manifest.permission.CAMERA) }
    val paired = { device: dev.nori.music.ffi.PairedDevice -> tell(context.getString(R.string.pair_done, device.name)); onPaired() }
    val way = pasting
    if (way != null) {
        PastePage(vm, way, paired)
        return
    }
    // What to say of what the camera read last, the code being paired with now, and the last one tried: a
    // code turned down is not tried again while it stays in sight.
    var said by remember { mutableStateOf<String?>(null) }
    var pairing by remember { mutableStateOf<String?>(null) }
    var tried by remember { mutableStateOf<String?>(null) }
    val other = words(R.string.pair_scan_other)
    Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter), horizontalAlignment = Alignment.CenterHorizontally) {
        if (allowed) QrScanner(
            onFound = { text ->
                if (pairing != null || text == tried) return@QrScanner
                if (!dev.nori.music.ffi.pairingValid(text)) { said = other; return@QrScanner }
                pairing = text
                tried = text
                said = context.getString(R.string.pair_busy)
                vm.pair(text) { result ->
                    result.onSuccess(paired)
                    result.onFailure { said = say.pairFailed(it, dev.nori.music.ffi.pairingName(text)); pairing = null }
                }
            },
            noCamera = { LaunchedEffect(Unit) { pasting = Pasting.NO_CAMERA } },
        )
        Text(
            said ?: words(R.string.pair_scan_hint), Modifier.padding(top = 16.dp),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center, style = MaterialTheme.typography.bodyMedium,
            color = if (said != null && pairing == null) scheme.error else scheme.onSurfaceVariant,
        )
        TextAction(words(R.string.pair_paste_instead), { pasting = Pasting.ASKED }, Modifier.padding(top = 8.dp, bottom = 16.dp))
    }
}

/** The other device's pairing link pasted, and Pair; why pairing failed shows under the field. */
@Composable
private fun PastePage(vm: RemoteViewModel, way: Pasting, onPaired: (dev.nori.music.ffi.PairedDevice) -> Unit) {
    var entered by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    val scheme = MaterialTheme.colorScheme
    Text(
        words(when (way) { Pasting.NO_CAMERA -> R.string.pair_no_camera; Pasting.DENIED -> R.string.pair_camera_denied; Pasting.ASKED -> R.string.pair_paste_hint }),
        Modifier.padding(horizontal = Space.gutter), style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant,
    )
    GroupCard { GroupField(entered, { entered = it; failure = null }, placeholder = words(R.string.pair_link), divider = false, autofocus = way == Pasting.ASKED) }
    Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 16.dp), Arrangement.spacedBy(16.dp), Alignment.CenterHorizontally) {
        failure?.let { Text(it, color = scheme.error, style = MaterialTheme.typography.bodyMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center) }
        PillButton(words(if (busy) R.string.pair_busy else R.string.pair_action), Iconoir.SmartphoneDevice, {
            busy = true
            failure = null
            val link = entered.trim()
            vm.pair(link) { result ->
                busy = false
                result.onSuccess(onPaired)
                result.onFailure { failure = say.pairFailed(it, dev.nori.music.ffi.pairingName(link)) }
            }
        }, prominent = true, enabled = !busy && dev.nori.music.ffi.pairingValid(entered.trim()))
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
    val finds by settings.pluginsFindOutputs.collectAsStateWithLifecycle()
    val remote = prefs.remoteControl && LocalRules.current.remote
    if (remote || pluginDevices.isNotEmpty() || controlled.isNotEmpty() || cast != null || finds) DevicesSheet(open, onDismiss, { onDismiss(); openOutputPicker(context, output, tell) }, prefs.remoteControl, remote, settings)
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

/** The icon of what a plugin casts to. */
internal fun castIcon(look: dev.nori.music.ffi.plugins.CastLook) = when (look) {
    dev.nori.music.ffi.plugins.CastLook.SPEAKER -> Iconoir.SoundHigh
    dev.nori.music.ffi.plugins.CastLook.HEADPHONES -> Iconoir.Headset
    dev.nori.music.ffi.plugins.CastLook.PHONE -> Iconoir.SmartphoneDevice
    dev.nori.music.ffi.plugins.CastLook.TV -> Iconoir.Airplay
}

/** The icon of what another device casts to. */
private fun outputIcon(look: dev.nori.music.ffi.remote.OutputLook) = when (look) {
    dev.nori.music.ffi.remote.OutputLook.SPEAKER -> Iconoir.SoundHigh
    dev.nori.music.ffi.remote.OutputLook.HEADPHONES -> Iconoir.Headset
    dev.nori.music.ffi.remote.OutputLook.PHONE -> Iconoir.SmartphoneDevice
    dev.nori.music.ffi.remote.OutputLook.TV -> Iconoir.Airplay
}

/** A row of the sheet that is not a device. */
@Composable
private fun SheetRow(name: String, icon: ImageVector, divider: Boolean, onClick: () -> Unit) = GroupRow(name, onClick, icon = icon, divider = divider)

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

/** How long the other device's volume stays up after the last press of a volume key. */
private const val VOLUME_SHOWN_MS = 1_500L

/**
 * The volume of the device playing elsewhere, as the volume keys move it (RemoteViewModel.volumeKey): a pill
 * with the device's glyph and name over its level, shown on a press and gone a moment after the last.
 */
@Composable
fun RemoteVolume(remote: RemoteViewModel, look: Look, modifier: Modifier = Modifier) {
    var pressed by remember { mutableStateOf(false) }
    LaunchedEffect(remote) {
        remote.volumeKeys.collectLatest {
            pressed = true
            kotlinx.coroutines.delay(VOLUME_SHOWN_MS)
            pressed = false
        }
    }
    val mirror by remote.mirror.collectAsStateWithLifecycle()
    // Kept through the fade out, which may outlast the device.
    var shown by remember { mutableStateOf<dev.nori.music.ffi.Mirror?>(null) }
    mirror?.takeIf { it.volume != null }?.let { shown = it }
    AnimatedVisibility(pressed && mirror?.volume != null, modifier, enter = AppMotion.appear(), exit = AppMotion.vanish()) {
        val m = shown ?: return@AnimatedVisibility
        val level = (m.volume?.toInt() ?: 0) / 100f
        val ink = androidx.compose.ui.graphics.ColorProducer { look.color(CoverLook.CHROME_CONTENT) }
        val description = words(R.string.remote_volume, m.name, m.volume?.toInt() ?: 0)
        Row(
            Modifier.widthIn(max = 360.dp).fillMaxWidth()
                .semantics { contentDescription = description }
                .drawBehind {
                    val round = androidx.compose.ui.geometry.CornerRadius(size.height / 2)
                    drawRoundRect(look.color(CoverLook.SURFACE_CONTAINER), cornerRadius = round)
                    drawRoundRect(look.color(CoverLook.OUTLINE_VARIANT), cornerRadius = round, style = androidx.compose.ui.graphics.drawscope.Stroke(HAIRLINE.toPx()))
                }
                .padding(start = 18.dp, end = 22.dp, top = 10.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val glyph = when {
                level <= 0f -> Iconoir.SoundOff
                level < 0.5f -> Iconoir.SoundLow
                else -> Iconoir.SoundHigh
            }
            LookIcon(glyph, null, Modifier.size(22.dp), ink)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                LookText(m.name, { look.color(CoverLook.CHROME_CONTENT_65) }, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(6.dp))
                androidx.compose.foundation.layout.Box(
                    Modifier.fillMaxWidth().height(4.dp).drawBehind {
                        val color = look.color(CoverLook.CHROME_CONTENT)
                        val round = androidx.compose.ui.geometry.CornerRadius(size.height / 2)
                        drawRoundRect(color.copy(alpha = color.alpha * 0.22f), cornerRadius = round)
                        drawRoundRect(color, size = size.copy(width = size.width * level), cornerRadius = round)
                    },
                )
            }
        }
    }
}
