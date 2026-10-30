package dev.nori.music.app.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.app.vm.SettingsViewModel
import androidx.compose.runtime.DisposableEffect
import androidx.lifecycle.compose.LifecycleStartEffect
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.PluginRow
import dev.nori.music.ffi.plugins.RowKind
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.plugins.RowRef
import dev.nori.music.text.Fmt

private const val DIMMED = 0.38f

/** A repo as its rows name it: nori's own, else its host. */
private fun repoName(url: String, official: Boolean): String = if (official) say.pluginRepoOfficial else android.net.Uri.parse(url).host ?: url

/** What a card is, in words: its extension points and whether it changes nori's values. */
private fun PluginCard.kinds() = say.pluginKinds(points, overrides.isNotEmpty())

/**
 * Settings › Plugins and its plugins' pages, for where there is no navigation of the app's yet (the first
 * profile's form): one page at a time, back going from a plugin's page to the list.
 */
@Composable
fun PluginsPages(vm: PluginsViewModel, sound: SettingsViewModel) {
    var page by remember { mutableStateOf<String?>(null) }
    BackHandler(page != null) { page = null }
    when (val id = page) {
        null -> PluginsScreen(vm, sound) { page = it }
        else -> PluginPage(vm, sound, id) { page = null }
    }
}

/**
 * Settings › Plugins: the installed plugins in their order (on or off, moved, removed, each opening its
 * page with [open]), installing one from a file, what the repos offer that isn't here, and the repos.
 */
@Composable
fun PluginsScreen(vm: PluginsViewModel, sound: SettingsViewModel, open: (String) -> Unit) {
    SoundPage(sound)
    val cards by vm.cards.collectAsStateWithLifecycle()
    val repos by vm.repos.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    val refused by vm.refused.collectAsStateWithLifecycle()
    var adding by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf<PluginCard?>(null) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) vm.previewFile(uri) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    // Offered and not here, or here in an older version.
    val wanted = offers.offers.filter { o -> o.installed == null || o.installed != o.versions.firstOrNull()?.version }
    Column(Modifier.verticalScroll(rememberScrollState())) {
        LargeTitle(say.plugins)
        if (cards.isEmpty()) Quiet(say.pluginsEmpty, Modifier.padding(horizontal = Space.gutter, vertical = 8.dp))
        else {
            GroupLabel(say.pluginsInstalled)
            GroupCard {
                cards.forEachIndexed { i, c ->
                    InstalledRow(
                        c, i < cards.lastIndex, { open(c.id) }, { on -> vm.turn(c.id, on) },
                        listOfNotNull(
                            (say.pluginMoveUp to { vm.move(c.id, -1) }).takeIf { i > 0 },
                            (say.pluginMoveDown to { vm.move(c.id, 1) }).takeIf { i < cards.lastIndex },
                            say.remove to { removing = c },
                        ),
                    )
                }
            }
            val prefs by sound.prefs.collectAsStateWithLifecycle()
            Spacer(Modifier.height(8.dp))
            GroupCard {
                Toggle(say.pluginsOff, say.pluginsOffDetail, prefs.pluginsOff, key = "pluginsOff", divider = false) { off -> vm.allOff(sound, off) }
            }
        }
        Column(Modifier.padding(horizontal = Space.gutter, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            PillButton(say.pluginsInstallFile, null, { pick.launch(arrayOf("*/*")) }, Modifier.fillMaxWidth().testTag("plugin-install-file"))
            refused?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("plugin-refused")) }
        }

        if (wanted.isNotEmpty() || offers.loading) {
            GroupLabel(say.pluginsGet)
            GroupCard {
                if (wanted.isEmpty()) GroupRow(say.pluginsAsking, divider = false)
                wanted.forEachIndexed { i, o ->
                    OfferRow(o, i < wanted.lastIndex) { vm.previewOffer(o) }
                }
            }
        }
        offers.failed.forEach { f ->
            Quiet(say.pluginRepoUnreachable(android.net.Uri.parse(f.repo.url).host ?: f.repo.url), Modifier.padding(horizontal = Space.gutter, vertical = 4.dp))
        }

        GroupLabel(say.pluginRepos)
        GroupCard {
            repos.forEach { r ->
                GroupRow(
                    repoName(r.url, r.official), subtitle = r.url.takeIf { !r.official },
                    trailing = if (r.official) null else ({ TextAction(say.remove, { vm.removeRepo(r.url) }, destructive = true) }),
                )
            }
            GroupRow(say.pluginAddRepo, { adding = true }, Modifier.testTag("plugin-add-repo"), divider = false)
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
    AddRepoSheet(adding, { adding = false }) { url -> vm.addRepo(url); adding = false }
    InstallSheet(vm)
    BluetoothAsk(vm, row = false)
    RemoveSheet(removing, { removing = null }) { c -> vm.uninstall(c.id); removing = null }
}

/** A line in the quiet ink. */
@Composable
private fun Quiet(text: String, modifier: Modifier = Modifier, lines: Int = Int.MAX_VALUE) = Text(
    text, modifier, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
    maxLines = lines, overflow = TextOverflow.Ellipsis,
)

/** An installed plugin: its name, its about on one line, what it is, its menu and its switch; a tap opens its page. */
@Composable
private fun InstalledRow(c: PluginCard, divider: Boolean, open: () -> Unit, turn: (Boolean) -> Unit, menu: List<Pair<String, () -> Unit>>) {
    Column(Modifier.fillMaxWidth().testTag("plugin-${c.id}")) {
        Row(Modifier.fillMaxWidth().clickable(onClick = open).padding(start = GROUP_INSET, end = GROUP_INSET, top = 10.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(c.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                c.about?.let { Quiet(it, lines = 1) }
                c.kinds().takeIf { it.isNotEmpty() }?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
            MoreButton(menu)
            NoriSwitch(c.enabled, turn, Modifier.testTag("plugin-on-${c.id}"))
        }
        if (divider) Hairline(startIndent = GROUP_INSET)
    }
}

/** An offered plugin: its name, about, version and repo, and Install, or Update for one here in an older version. */
@Composable
private fun OfferRow(o: Offer, divider: Boolean, install: () -> Unit) {
    GroupRow(
        o.name, install, Modifier.testTag("offer-${o.id}"),
        subtitle = listOfNotNull(o.about, o.versions.firstOrNull()?.let { say.pluginOfferLine(it.version, repoName(o.repo.url, o.repo.official)) }).joinToString("\n"),
        divider = divider,
        trailing = { TextAction(if (o.installed == null) say.pluginInstall else say.pluginUpdate, install) },
    )
}

/** A sheet's paper: a title, what it says, then its buttons at the end. */
@Composable
private fun SheetPage(title: String, buttons: @Composable () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.windowInsetsPadding(gestureBarRoom).padding(start = Space.gutter, end = Space.gutter, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(title, style = titled(MaterialTheme.typography.titleLarge))
        content()
        Row(Modifier.fillMaxWidth(), Arrangement.spacedBy(8.dp, Alignment.End)) { buttons() }
    }
}

/** A repo's address, asked under the warning that it isn't nori's: Add is the user agreeing. */
@Composable
private fun AddRepoSheet(visible: Boolean, close: () -> Unit, add: (String) -> Unit) {
    NoriSheet(visible, close) {
        var url by remember { mutableStateOf("") }
        SheetPage(say.pluginAddRepo, {
            TextAction(say.cancel, close)
            TextAction(say.add, { add(url) }, Modifier.testTag("repo-add"), enabled = url.isNotBlank())
        }) {
            Quiet(say.pluginRepoWarning)
            FormField(
                url, { url = it }, Modifier.fillMaxWidth().testTag("repo-url"), label = { Text(say.pluginRepoUrl) }, singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
        }
    }
}

/**
 * A plugin read but not installed, from a file or a repo: what it is and everything it may reach, before
 * Install. A new install lands off; turning it on is one tap on its row.
 */
@Composable
private fun InstallSheet(vm: PluginsViewModel) {
    val preview by vm.preview.collectAsStateWithLifecycle()
    NoriSheet(preview, vm::dropPreview) { p ->
        val c = p.card
        SheetPage(c.name, {
            TextAction(say.cancel, vm::dropPreview)
            TextAction(say.pluginInstall, vm::installPreview, Modifier.testTag("install-confirm"))
        }) {
            c.about?.let { Text(it) }
            Quiet(listOfNotNull(say.pluginVersion(c.version), c.kinds().takeIf { it.isNotEmpty() }, p.repo?.let { android.net.Uri.parse(it).host }, say.pluginUnsigned.takeIf { p.key == null }).joinToString(" · "))
            Text(say.pluginMayTitle, style = MaterialTheme.typography.titleSmall)
            say.pluginMay(c).forEach { Text("· $it") }
        }
    }
}

/**
 * Asks for the Bluetooth permission a plugin needs ([PluginsViewModel.needsBluetooth]): at once after installing one (the
 * view model says so), and as a row on its page while it is missing.
 */
@Composable
private fun BluetoothAsk(vm: PluginsViewModel, modifier: Modifier = Modifier, row: Boolean = true) {
    if (android.os.Build.VERSION.SDK_INT < 31) return
    var allowed by remember { mutableStateOf(vm.bluetoothAllowed()) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> allowed = granted; vm.bluetoothAnswered(granted) }
    val asked by vm.askBluetooth.collectAsStateWithLifecycle()
    LaunchedEffect(asked) { if (asked) ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT) }
    if (row && !allowed) GroupCard(modifier) { GroupRow(say.allowBluetooth, { ask.launch(android.Manifest.permission.BLUETOOTH_CONNECT) }, icon = Iconoir.Bluetooth, chevron = true, divider = false) }
}

/** Removing a plugin, asked first. */
@Composable
private fun RemoveSheet(card: PluginCard?, close: () -> Unit, remove: (PluginCard) -> Unit) {
    NoriSheet(card, close) { c ->
        SheetPage(say.pluginRemoveTitle(c.name), {
            TextAction(say.cancel, close)
            TextAction(say.remove, { remove(c) }, Modifier.testTag("remove-confirm"), destructive = true)
        }) { Quiet(say.pluginRemoveText) }
    }
}

/**
 * A plugin's page: what it is, on or off, the last thing it said, its rows drawn with the settings' own
 * components (a section row starts a new card), the versions its repo offers, and Remove.
 */
@Composable
fun PluginPage(vm: PluginsViewModel, sound: SettingsViewModel, id: String, back: () -> Unit) {
    SoundPage(sound)
    val cards by vm.cards.collectAsStateWithLifecycle()
    val notice by vm.notice.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<PluginCard?>(null) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    val card = cards.firstOrNull { it.id == id } ?: return
    // What the repo it came from offers of it; a plugin from a file updates from nowhere.
    val offer = offers.offers.firstOrNull { it.id == id && it.repo.url == card.repo }
    Column(Modifier.verticalScroll(rememberScrollState())) {
        LargeTitle(card.name)
        Column(Modifier.padding(horizontal = Space.gutter, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            card.about?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
            Quiet(listOfNotNull(say.pluginVersion(card.version), card.kinds().takeIf { it.isNotEmpty() }, say.pluginFromFile.takeIf { card.repo == null }).joinToString(" · "))
            notice?.takeIf { it.plugin == id }?.let { n ->
                Text(n.line, style = MaterialTheme.typography.bodySmall, color = if (n.problem) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.height(8.dp))
        GroupCard {
            Toggle(say.pluginOn, "", card.enabled, key = "plugin-on", divider = card.repo != null) { on -> vm.turn(id, on) }
            if (card.repo != null) Toggle(say.pluginKeepVersion, "", card.held, divider = false) { held -> vm.hold(id, held) }
        }
        val controlled by vm.controlled.collectAsStateWithLifecycle()
        controlled.filter { it.card.id == id }.forEach { d -> DeviceControls(vm, d, Modifier.padding(top = Space.section)) }
        if (vm.needsBluetooth(card) && card.enabled) BluetoothAsk(vm, Modifier.padding(top = Space.section))
        PluginRows(vm, card, null)
        PluginAssets(vm, id)
        if (offer != null) {
            val uri = androidx.compose.ui.platform.LocalUriHandler.current
            GroupLabel(say.pluginVersions)
            GroupCard {
                offer.versions.forEachIndexed { i, v ->
                    GroupRow(
                        v.version, minHeight = 44.dp, value = v.released, divider = i < offer.versions.lastIndex || offer.source != null,
                        trailing = {
                            if (v.version != card.version) TextAction(say.pluginInstall, { vm.previewVersion(v) }, Modifier.padding(start = 8.dp))
                        },
                    )
                }
                offer.source?.let { src -> GroupRow(say.pluginSource, { runCatching { uri.openUri(src) } }, minHeight = 44.dp, value = android.net.Uri.parse(src).host, chevron = true, divider = false) }
            }
        }
        Row(Modifier.padding(horizontal = 8.dp, vertical = 12.dp)) {
            TextAction(say.remove, { removing = card }, Modifier.testTag("plugin-remove"), destructive = true)
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
    RemoveSheet(removing, { removing = null }) { c -> vm.uninstall(c.id); removing = null; back() }
}

/** A plugin's assets as status lines, with the way to fetch them on mobile data while one waits for Wi-Fi. */
@Composable
private fun PluginAssets(vm: PluginsViewModel, id: String) {
    val all by vm.assets.collectAsStateWithLifecycle()
    val assets = all[id].orEmpty()
    if (assets.isEmpty()) return
    Spacer(Modifier.height(Space.section))
    GroupCard {
        assets.forEachIndexed { i, a -> GroupRow(say.pluginAsset(a), minHeight = 44.dp, divider = i < assets.lastIndex) }
        if (assets.any { it.state is dev.nori.music.ffi.plugins.AssetState.WaitingForWifi }) GroupRow(say.pluginAssetsOnMobile, { vm.allowMobileAssets(id) }, minHeight = 44.dp, divider = false)
    }
}

/**
 * A plugin turned on or a row changed is heard at once while this page is in sight: it is a sound page
 * tuned from its first look, so even the first change does not wait for the deep buffer to drain. [sound]
 * is the app's one SettingsViewModel, which counts every sound page in sight: a page coming in sight
 * starts before the one it covers stops.
 */
@Composable
private fun SoundPage(sound: SettingsViewModel) {
    LifecycleStartEffect(sound) { sound.soundPageShown(true, tuned = true); onStopOrDispose { sound.soundPageShown(false) } }
}

/** The rows of the plugins that show on the devices sheet too, each under its name; read before the sheet opened. */
@Composable
fun PluginDeviceSections(vm: PluginsViewModel, sound: SettingsViewModel) {
    val cards by vm.deviceCards.collectAsStateWithLifecycle()
    // With plugins' rows on it, the sheet is a sound page: what they change is heard at once.
    if (cards.isNotEmpty()) DisposableEffect(sound) { sound.soundPageShown(true, tuned = true); onDispose { sound.soundPageShown(false) } }
    cards.forEach { c -> PluginRows(vm, c, c.name) }
}

/**
 * [card]'s rows in cards as nori's own settings pages draw theirs: each section row the label over the card
 * after it. With [title] (a page of nori's own showing them), every label starts with it and the rows before
 * the first section are under it alone; without, those rows are under a gap. Rows its plugin hides are left out.
 */
@Composable
fun PluginRows(vm: PluginsViewModel, card: PluginCard, title: String?) {
    val status by vm.status.collectAsStateWithLifecycle()
    val shown by vm.shown.collectAsStateWithLifecycle()
    val groups = mutableListOf<Pair<String?, MutableList<PluginRow>>>(title to mutableListOf())
    card.rows.filter { shown[RowRef(card.id, it.id)]?.shown != false }.forEach { r ->
        if (r.kind == RowKind.SECTION) groups += (title?.let { say.pluginSection(it, r.label) } ?: r.label) to mutableListOf() else groups.last().second += r
    }
    groups.filter { it.second.isNotEmpty() }.forEach { (label, rows) ->
        if (label != null) GroupLabel(label) else Spacer(Modifier.height(Space.section))
        GroupCard {
            rows.forEachIndexed { i, r ->
                val enabled = shown[RowRef(card.id, r.id)]?.enabled != false
                PluginRowView(card, r, status[RowRef(card.id, r.id)], enabled, i < rows.lastIndex, vm)
            }
        }
    }
}

@Composable
private fun PluginRowView(card: PluginCard, r: PluginRow, status: String?, enabled: Boolean, divider: Boolean, vm: PluginsViewModel) {
    val dim = Modifier.alpha(if (enabled) 1f else DIMMED)
    when (r.kind) {
        RowKind.SWITCH -> Toggle(r.label, r.hint.orEmpty(), (r.value as? Value.Switch)?.v1 == true, enabled = enabled, divider = divider) { vm.set(card.id, r.id, Value.Switch(it)) }
        RowKind.CHOICE -> {
            var open by remember { mutableStateOf(false) }
            val chosen = (r.value as? Value.Choice)?.v1?.toInt()
            GroupRow(
                r.label, if (enabled) ({ open = true }) else null, dim, subtitle = r.hint, value = chosen?.let(r.options::getOrNull), chevron = true, divider = divider,
                trailing = {
                    NoriPopover(open, { open = false }) {
                        r.options.forEachIndexed { i, o -> PopoverItem(o, chosen = i == chosen) { vm.set(card.id, r.id, Value.Choice(i.toUInt())); open = false } }
                    }
                },
            )
        }
        RowKind.LEVEL -> {
            val min = (r.min ?: 0.0).toFloat()
            val max = (r.max ?: 1.0).toFloat()
            val step = r.step?.toFloat()?.takeIf { it > 0f }
            val value = ((r.value as? Value.Level)?.v1 ?: r.min ?: 0.0).toFloat()
            val shownValue = Fmt.fixed(value.toDouble(), r.decimals.toInt()).let { v -> r.unit?.let { say.pluginUnit(v, it) } ?: v }
            Text(say.pluginLevel(r.label, shownValue), dim.padding(start = GROUP_INSET, end = GROUP_INSET, top = 10.dp), style = MaterialTheme.typography.bodySmall)
            r.hint?.let { Quiet(it, dim.padding(horizontal = GROUP_INSET)) }
            NoriSlider(value, min..max, { v ->
                val snapped = step?.let { s -> (min + kotlin.math.round((v - min) / s) * s).coerceIn(min, max) } ?: v
                if (snapped != value) vm.set(card.id, r.id, Value.Level(snapped.toDouble()))
            }, Modifier.padding(horizontal = GROUP_INSET).testTag("level-${r.id}"), enabled = enabled)
            if (divider) Hairline(startIndent = GROUP_INSET)
        }
        RowKind.TEXT, RowKind.SECRET -> {
            val stored = (r.value as? Value.Text)?.v1.orEmpty()
            var text by remember(stored) { mutableStateOf(stored) }
            Column(dim.padding(horizontal = GROUP_INSET, vertical = 8.dp)) {
                FormField(
                    text, { text = it }, Modifier.fillMaxWidth(), label = { Text(r.label) }, singleLine = true, supportingText = r.hint?.let { h -> { Text(h) } },
                    visualTransformation = if (r.kind == RowKind.SECRET) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
                )
                if (text != stored) TextAction(say.save, { vm.set(card.id, r.id, Value.Text(text)) }, enabled = enabled)
            }
            if (divider) Hairline(startIndent = GROUP_INSET)
        }
        RowKind.ACTION -> GroupRow(r.label, if (enabled) ({ vm.act(card.id, r.id) }) else null, dim, subtitle = r.hint, divider = divider)
        RowKind.STATUS -> GroupRow(r.label, modifier = dim, subtitle = r.hint, value = status, divider = divider)
        RowKind.NOTE -> {
            Text(r.label, Modifier.padding(horizontal = GROUP_INSET, vertical = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (divider) Hairline(startIndent = GROUP_INSET)
        }
        RowKind.LINK -> {
            val uri = androidx.compose.ui.platform.LocalUriHandler.current
            GroupRow(
                r.label, r.url?.takeIf { enabled }?.let { u -> { runCatching { uri.openUri(u) } } }, dim,
                subtitle = r.hint, value = r.url?.let { android.net.Uri.parse(it).host }, chevron = true, divider = divider,
            )
        }
        RowKind.SECTION -> {}
    }
}
