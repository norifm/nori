package dev.nori.music.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.PairStep
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.app.vm.SyncViewModel
import dev.nori.music.app.vm.PairRole
import dev.nori.music.app.vm.takesCode
import dev.nori.music.app.vm.settingKey
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.sync.Came
import dev.nori.music.ffi.sync.Confirmation
import kotlinx.coroutines.launch

/**
 * Your devices: this device and its group's others with when each was last seen, Pair a device, the sync
 * server's settings and the kept copy, and Reset pairing. Not paired, one line says so over Pair a device.
 */
@Composable
fun YourDevices(settings: SettingsViewModel) {
    val vm: SyncViewModel = viewModel()
    val nav = LocalNav.current
    val v by vm.view.collectAsStateWithLifecycle()
    LifecycleResumeEffect(vm) { vm.read(); onPauseOrDispose {} }
    if (!v.read) return
    if (v.on && !v.paired) {
        Text(say.syncNotPaired, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        PillButton(say.syncPair, Iconoir.Plus, { nav.go("pair-device") }, Modifier.padding(horizontal = GROUP_MARGIN, vertical = 12.dp).fillMaxWidth().testTag("pair-device"), prominent = true)
    } else {
        GroupLabel(say.syncSectionDevices)
        GroupCard(Modifier.testTag("your-devices")) {
            GroupRow(vm.name, subtitle = say.syncThisDevice, icon = Iconoir.SmartphoneDevice)
            val now = remember(v.devices) { System.currentTimeMillis() }
            v.devices.forEach { d -> GroupRow(d.name, subtitle = say.syncSeen(d, now), icon = Iconoir.Computer) }
            GroupRow(say.syncPair, { nav.go("pair-device") }, Modifier.testTag("pair-device"), icon = Iconoir.Plus, divider = false)
        }
    }
    NeededPlugins(vm)
    GroupLabel(say.syncSectionServer)
    GroupCard {
        Toggle(say.syncOn, say.syncState(v.state), v.on, key = settingKey(say.syncOn)) { vm.setOn(it) }
        var address by remember(v.server) { mutableStateOf(v.server) }
        // Kept on Done, and when the page is left.
        val typed by androidx.compose.runtime.rememberUpdatedState(address)
        androidx.compose.runtime.DisposableEffect(vm) { onDispose { vm.setServer(typed) } }
        GroupField(
            address, { address = it }, Modifier.testTag("sync-server").spotlight(settingKey(say.syncServer)), label = say.syncServer, stacked = true,
            placeholder = say.syncServerPlaceholder, enabled = v.on, divider = v.paired,
            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Uri),
            onDone = { vm.setServer(address) },
        )
        if (v.paired) Toggle(say.syncKeepCopy, say.syncKeepCopyDetail, v.keepsCopy, divider = false) { vm.keepCopy(it) }
    }
    if (v.paired) {
        val scope = rememberCoroutineScope()
        var resetting by remember { mutableStateOf<String?>(null) }
        GroupCard(Modifier.padding(top = 8.dp)) {
            GroupRow(say.syncReset, { scope.launch { resetting = vm.resetNotice()?.let { (_, names) -> say.syncResetText(names) } } }, Modifier.testTag("reset-pairing"), divider = false)
        }
        NoriDialog(resetting, { resetting = null }) { text ->
            AlertCard(
                title = { Text(say.syncResetTitle) }, text = { Text(text) },
                confirmButton = { TextAction(say.syncResetConfirm, { vm.reset(); resetting = null }, Modifier.testTag("reset-confirm"), destructive = true) },
                dismissButton = { TextAction(say.cancel, { resetting = null }) },
            )
        }
    }
}

/**
 * Plugins the profiles sync brought play from and this device lacks: each opens its page to install it
 * there, its repo added first; one installed from a file on the other device only says so.
 */
@Composable
private fun NeededPlugins(vm: SyncViewModel) {
    val plugins: dev.nori.music.app.vm.PluginsViewModel = viewModel()
    val needed by vm.installs.collectAsStateWithLifecycle()
    val cards by plugins.cards.collectAsStateWithLifecycle()
    val offers by plugins.offers.collectAsStateWithLifecycle()
    val missing = needed.filter { n -> cards.none { it.id == n.id } }
    if (missing.isEmpty()) return
    val nav = LocalNav.current
    LaunchedEffect(Unit) { plugins.loadOffers() }
    GroupLabel(say.syncSectionInstall)
    GroupCard(Modifier.testTag("needed-plugins")) {
        missing.forEachIndexed { i, p ->
            val name = remember(p.id, cards, offers) { dev.nori.music.ffi.plugins.pluginDisplayName(p.id, cards, offers.offers) }
            val install = if (p.repo != null) ({ plugins.bring(p.id) { nav.plugin(p.id) } }) else null
            GroupRow(
                say.syncInstall(name), install, subtitle = if (p.repo == null) say.syncInstallFile else null,
                icon = Iconoir.Download, chevron = p.repo != null, divider = i < missing.lastIndex,
            )
        }
    }
}

/**
 * Pair a device, with the two roles a device can have: in no group, its code shows at once (the QR code,
 * Copy link, how long it works, a new one once it ran out); with profiles, it scans another's code (the
 * camera asked for only then), takes a pasted link, or a device waiting nearby. The other device's yes
 * comes as a confirmation here. Leaving stops a pairing that did not finish.
 */
@Composable
fun PairDevice() {
    val vm: SyncViewModel = viewModel()
    val settings: SettingsViewModel = viewModel()
    val nav = LocalNav.current
    val tell = LocalMessages.current
    val step by vm.step.collectAsStateWithLifecycle()
    val role by vm.role.collectAsStateWithLifecycle()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    val nearby by vm.nearby.collectAsStateWithLifecycle()
    val takes = takesCode(prefs.servers.size)
    var taking by rememberSaveable { mutableStateOf<Came?>(null) }
    LaunchedEffect(vm) { vm.opened() }
    LaunchedEffect(step) {
        when (step) {
            PairStep.Paired -> { tell(say.syncPaired); nav.back() }
            // The group's notice says the new device came.
            PairStep.Sent -> nav.back()
            else -> {}
        }
    }
    (step as? PairStep.Confirm)?.let { c -> PairConfirmation(c.confirmation, vm::confirm, vm::cancelPairing) }
    val refused = (step as? PairStep.Refused)?.reason?.takeIf { role == PairRole.TAKES }
    Column(Modifier.imePadding().verticalScroll(rememberScrollState())) {
        LargeTitle(say.syncPair)
        if (role == PairRole.SHOWS) {
            GroupLabel(say.syncSectionCode)
            CodeCard(step) { vm.showCode() }
        } else {
            GroupCard(Modifier.padding(top = 8.dp)) {
                GroupRow(say.syncShowCode, { taking = null; vm.showCode() }, Modifier.testTag("show-code"), icon = Iconoir.SmartphoneDevice, divider = false)
            }
        }
        if (takes) {
            if (nearby.isNotEmpty()) {
                GroupLabel(say.syncSectionNearby)
                GroupCard(Modifier.testTag("nearby")) {
                    nearby.forEachIndexed { i, d -> GroupRow(d.name, { vm.take(d) }, icon = Iconoir.AntennaSignal, chevron = true, divider = i < nearby.lastIndex) }
                }
            }
            GroupLabel(say.syncSectionTake)
            when (taking) {
                Came.CAMERA -> Scan(onFound = { vm.take(it, Came.CAMERA) }, onPaste = { taking = Came.LINK })
                Came.LINK -> Paste { vm.take(it, Came.LINK) }
                else -> GroupCard {
                    GroupRow(say.syncScan, { taking = Came.CAMERA }, Modifier.testTag("scan-code"), icon = Iconoir.ScanQrCode)
                    GroupRow(say.syncPaste, { taking = Came.LINK }, Modifier.testTag("paste-link"), icon = Iconoir.Copy, divider = false)
                }
            }
            if (step == PairStep.Starting && role == PairRole.TAKES) Text(say.syncStarting, Modifier.padding(horizontal = Space.gutter, vertical = 12.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        refused?.let { r ->
            Text(say.pairingRefused(r), Modifier.padding(horizontal = Space.gutter, vertical = 12.dp).testTag("pair-refused"), color = MaterialTheme.colorScheme.error)
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
}

/** This device's code: the QR code on white, its three words, how long it works and Copy link; ran out, a new one. */
@Composable
private fun CodeCard(step: PairStep, onNew: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter), horizontalAlignment = Alignment.CenterHorizontally) {
        when (step) {
            is PairStep.Code -> {
                QrPicture(step.link, 220.dp, Modifier.testTag("pair-code"))
                Words(step.words, Modifier.padding(top = 16.dp))
                Text(say.syncCodeWords, Modifier.padding(top = 6.dp), textAlign = TextAlign.Center, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                step.note?.let { Text(say.pairingRefused(it), Modifier.padding(top = 8.dp), textAlign = TextAlign.Center, style = MaterialTheme.typography.bodySmall, color = scheme.error) }
                Text(say.syncCodeLasts, Modifier.padding(top = 8.dp), textAlign = TextAlign.Center, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                CopyLink(step.link, Modifier.padding(top = 14.dp))
            }
            PairStep.RanOut -> {
                Text(say.syncCodeRanOut, Modifier.padding(vertical = 12.dp).testTag("code-ran-out"), style = MaterialTheme.typography.bodyMedium)
                PillButton(say.syncNewCode, Iconoir.Refresh, onNew, Modifier.fillMaxWidth().testTag("new-code"), prominent = true)
            }
            is PairStep.Refused -> {
                Text(say.pairingRefused(step.reason), Modifier.padding(vertical = 12.dp).testTag("pair-refused"), color = scheme.error, textAlign = TextAlign.Center)
                PillButton(say.syncNewCode, Iconoir.Refresh, onNew, Modifier.fillMaxWidth().testTag("new-code"), prominent = true)
            }
            else -> Text(say.syncStarting, Modifier.padding(vertical = 24.dp), color = scheme.onSurfaceVariant)
        }
    }
}

/** The pairing link copied, for a device without a camera. */
@Composable
private fun CopyLink(link: String, modifier: Modifier) {
    val context = LocalContext.current
    PillButton(say.syncCopyLink, Iconoir.Copy, {
        context.getSystemService(android.content.ClipboardManager::class.java)?.setPrimaryClip(android.content.ClipData.newPlainText(say.syncLink, link))
        // Android 13 and later show the copy themselves.
        if (android.os.Build.VERSION.SDK_INT < 33) android.widget.Toast.makeText(context, say.syncCopyLink, android.widget.Toast.LENGTH_SHORT).show()
    }, modifier.fillMaxWidth().testTag("copy-link"), prominent = true)
}

/** The three words both screens show while pairing. */
@Composable
private fun Words(words: List<String>, modifier: Modifier = Modifier) {
    Text(
        words.joinToString("  "), modifier.testTag("pair-words"), textAlign = TextAlign.Center,
        style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.SemiBold), color = MaterialTheme.colorScheme.onSurface,
    )
}

/** The camera on the other device's code, asked for at the tap that opened it; refused or missing, the link instead. */
@Composable
private fun Scan(onFound: (String) -> Unit, onPaste: () -> Unit) {
    val context = LocalContext.current
    val scheme = MaterialTheme.colorScheme
    val camera = remember { context.packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY) }
    val granted = { androidx.core.content.ContextCompat.checkSelfPermission(context, android.Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED }
    var allowed by remember { mutableStateOf(granted()) }
    var refused by remember { mutableStateOf(false) }
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { ok ->
        allowed = ok
        refused = !ok
    }
    LaunchedEffect(Unit) { if (camera && !allowed) ask.launch(android.Manifest.permission.CAMERA) }
    // A code read once is taken once, while it stays in sight.
    var taken by remember { mutableStateOf<String?>(null) }
    Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter), horizontalAlignment = Alignment.CenterHorizontally) {
        when {
            !camera -> Text(say.syncNoCamera, color = scheme.onSurfaceVariant, textAlign = TextAlign.Center)
            refused -> Text(say.syncCameraDenied, color = scheme.onSurfaceVariant, textAlign = TextAlign.Center)
            allowed -> {
                QrScanner(onFound = { text -> if (text != taken) { taken = text; onFound(text) } }, noCamera = { Text(say.syncNoCamera, color = scheme.onSurfaceVariant) })
                Text(say.syncScanHint, Modifier.padding(top = 12.dp), textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant)
            }
        }
        TextAction(say.syncPaste, onPaste, Modifier.padding(top = 4.dp))
    }
}

/** The other device's pairing link, pasted. */
@Composable
private fun Paste(onTake: (String) -> Unit) {
    var entered by remember { mutableStateOf("") }
    GroupCard {
        GroupField(
            entered, { entered = it }, Modifier.testTag("pair-link"), placeholder = say.syncLink, divider = false, autofocus = true,
            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Uri, autoCorrectEnabled = false),
            onDone = { if (entered.isNotBlank()) onTake(entered) },
        )
    }
    PillButton(say.syncPairAction, null, { onTake(entered) }, Modifier.padding(horizontal = GROUP_MARGIN, vertical = 12.dp).fillMaxWidth().testTag("pair-take"), prominent = true, enabled = entered.isNotBlank())
}

/**
 * On the device with the profiles: who the new device is and how many profiles' sign-ins it gets, the three
 * words to compare when the code came by link or from nearby, the group it leaves, and at the first pairing
 * whether to keep an encrypted copy (kept, unless switched off here).
 */
@Composable
private fun PairConfirmation(c: Confirmation, onPair: (Boolean) -> Unit, onCancel: () -> Unit) {
    var keep by remember(c) { mutableStateOf(true) }
    val scheme = MaterialTheme.colorScheme
    NoriDialog(c, onCancel) {
        AlertCard(
            title = { Text(say.syncConfirmTitle(c.name, c.profiles.toInt()), Modifier.testTag("pair-confirm-title")) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    c.words?.let { w ->
                        Words(w, Modifier.fillMaxWidth())
                        Text(say.syncConfirmWords, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                    }
                    c.leaves?.let { n -> Text(say.syncConfirmLeaves(c.name, n.toInt()), style = MaterialTheme.typography.bodyMedium) }
                    if (c.asksKeepCopy) Row(Modifier.fillMaxWidth().testTag("keep-copy"), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(say.syncKeepCopyAsk, style = MaterialTheme.typography.bodyMedium)
                            Text(say.syncKeepCopyAskDetail, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                        }
                        NoriSwitch(keep, { keep = it })
                    }
                }
            },
            confirmButton = { TextAction(say.syncPairAction, { onPair(keep) }, Modifier.testTag("pair-confirm")) },
            dismissButton = { TextAction(say.cancel, onCancel) },
        )
    }
}

/** Two profiles that may be one account, which the core cannot tell: both shown, Merge keeping the one in use, or Keep both. */
@Composable
fun MergeOffers(settings: SettingsViewModel) {
    val vm: SyncViewModel = viewModel()
    val v by vm.view.collectAsStateWithLifecycle()
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    v.offers.forEach { offer ->
        val a = prefs.servers.firstOrNull { it.id == offer.a } ?: return@forEach
        val b = prefs.servers.firstOrNull { it.id == offer.b } ?: return@forEach
        GroupLabel(say.syncMergeTitle)
        GroupCard(Modifier.testTag("merge-offer")) {
            ProfileLine(settings, a, false, divider = true, onClick = null)
            ProfileLine(settings, b, false, divider = true, onClick = null)
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), Arrangement.End) {
                TextAction(say.syncKeepBoth, { vm.keepApart(offer) })
                TextAction(say.syncMerge, { vm.merge(offer) }, Modifier.testTag("merge"))
            }
        }
    }
}

/**
 * Sync this profile, among a profile's advanced settings: on by default. Switched off on a profile that
 * syncs, it asks whether the other devices remove it too. [saved] is the profile as kept, null while it
 * is being added, when the switch only changes the form ([onForm]).
 */
@Composable
fun SyncProfileSwitch(saved: SavedServer?, on: Boolean, onForm: (Boolean) -> Unit) {
    val vm: SyncViewModel = viewModel()
    val v by vm.view.collectAsStateWithLifecycle()
    var asking by remember { mutableStateOf(false) }
    Toggle(say.syncProfile, "", on, key = "sync-profile", divider = false) { next ->
        when {
            saved == null -> onForm(next)
            !next && dev.nori.music.app.vm.syncOffAsks(saved.id, v) -> asking = true
            else -> { onForm(next); vm.syncProfile(saved, next) }
        }
    }
    if (saved != null) NoriDialog(asking, { asking = false }) {
        AlertCard(
            title = { Text(say.syncProfileOffTitle(saved.name.ifBlank { saved.url }), maxLines = 2, overflow = TextOverflow.Ellipsis) },
            text = { Text(say.syncProfileOffText) },
            confirmButton = { TextAction(say.syncProfileOffRemove, { asking = false; onForm(false); vm.syncProfile(saved, false, elsewhere = true) }, Modifier.testTag("sync-off-remove"), destructive = true) },
            dismissButton = { TextAction(say.syncProfileOffKeep, { asking = false; onForm(false); vm.syncProfile(saved, false) }, Modifier.testTag("sync-off-keep")) },
        )
    }
}
