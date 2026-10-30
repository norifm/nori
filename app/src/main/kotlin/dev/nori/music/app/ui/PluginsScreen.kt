package dev.nori.music.app.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.fadeIn
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.app.vm.SettingsViewModel
import androidx.compose.runtime.DisposableEffect
import androidx.lifecycle.compose.LifecycleStartEffect
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.OfferState
import dev.nori.music.ffi.plugins.ReadmeBlock
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.PluginRow
import dev.nori.music.ffi.plugins.RowKind
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.plugins.RowRef
import dev.nori.music.text.Fmt

private const val DIMMED = 0.38f

/** A repo as its rows name it: nori's own, else its host. */
private fun repoName(url: String, official: Boolean): String = if (official) say.pluginRepoOfficial else repoHost(url)

private fun repoHost(url: String): String = android.net.Uri.parse(url).host ?: url

/** What a card is, in words: its extension points and whether it changes nori's values. */
private fun PluginCard.kinds() = say.pluginKinds(points, overrides.isNotEmpty())

/** How the plugin pages reach each other. */
class PluginPages(val open: (String) -> Unit, val browser: () -> Unit, val repos: () -> Unit)

fun Nav.pluginPages() = PluginPages(::plugin, ::pluginBrowser, ::pluginRepos)

private const val BROWSER = "browser"
private const val REPOS = "repos"

/**
 * Settings › Plugins and the pages it opens, for where there is no navigation of the app's yet (the first
 * profile's form): a stack of its own, back going down it.
 */
@Composable
fun PluginsPages(vm: PluginsViewModel, sound: SettingsViewModel) {
    var stack by remember { mutableStateOf(listOf<String>()) }
    val back = { stack = stack.dropLast(1) }
    val pages = PluginPages({ stack = stack + it }, { stack = stack + BROWSER }, { stack = stack + REPOS })
    BackHandler(stack.isNotEmpty(), back)
    when (val page = stack.lastOrNull()) {
        null -> PluginsScreen(vm, sound, pages)
        BROWSER -> PluginBrowser(vm, pages)
        REPOS -> PluginRepos(vm)
        else -> PluginPage(vm, sound, page, back)
    }
}

/**
 * Settings › Plugins: updates waiting, the installed plugins in their order (each opening its page), and
 * the switch over them all. The title's plus opens the [PluginBrowser].
 */
@Composable
fun PluginsScreen(vm: PluginsViewModel, sound: SettingsViewModel, pages: PluginPages) {
    SoundPage(sound)
    val cards by vm.cards.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    val refused by vm.refused.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { vm.loadOffers() }
    val updates by vm.updates.collectAsStateWithLifecycle()
    Column(Modifier.verticalScroll(rememberScrollState())) {
        LargeTitle(say.plugins, trailing = { GlyphButton(Iconoir.Plus, say.pluginsGet, Modifier.testTag("plugins-browse"), onClick = pages.browser) })
        Refused(refused)
        if (cards.isEmpty()) {
            Column(Modifier.padding(horizontal = Space.gutter, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Quiet(say.pluginsEmpty)
                PillButton(say.pluginsGet, null, pages.browser, Modifier.fillMaxWidth(), prominent = true)
            }
        } else {
            if (updates.isNotEmpty()) {
                GroupLabel(say.pluginsUpdates)
                GroupCard {
                    updates.forEachIndexed { i, o ->
                        val to = o.versions.first().version
                        PluginLine(vm, o.id, o, o.name, null, say.pluginUpdateLine(o.installed.orEmpty(), to), i < updates.lastIndex, Modifier.testTag("update-${o.id}"), { pages.open(o.id) }) {
                            TextAction(say.pluginUpdate, { vm.previewOffer(o) })
                        }
                    }
                }
            }
            GroupLabel(say.pluginsInstalled)
            GroupCard {
                cards.forEachIndexed { i, c ->
                    val line = listOfNotNull(c.version, c.kinds().takeIf { it.isNotEmpty() }).joinToString(" · ")
                    PluginLine(vm, c.id, remember(offers, c) { vm.offerOf(c.id, c) }, c.name, c.about, line, i < cards.lastIndex, Modifier.testTag("plugin-${c.id}"), { pages.open(c.id) }) {
                        NoriSwitch(c.enabled, { on -> vm.turn(c.id, on) }, Modifier.testTag("plugin-on-${c.id}"))
                    }
                }
            }
            val prefs by sound.prefs.collectAsStateWithLifecycle()
            Spacer(Modifier.height(Space.section))
            GroupCard {
                Toggle(say.pluginsOff, say.pluginsOffDetail, prefs.pluginsOff, key = "pluginsOff", divider = false) { off -> vm.allOff(sound, off) }
            }
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
    InstallSheet(vm)
    BluetoothAsk(vm, row = false)
}

/**
 * What the repos offer: searched, narrowed to one repo when there are several, each row opening the
 * plugin's page. The title holds the repos and installing from a file.
 */
@Composable
fun PluginBrowser(vm: PluginsViewModel, pages: PluginPages) {
    val offers by vm.offers.collectAsStateWithLifecycle()
    val found by vm.found.collectAsStateWithLifecycle()
    val query by vm.query.collectAsStateWithLifecycle()
    val shown by vm.repoShown.collectAsStateWithLifecycle()
    val repos by vm.repos.collectAsStateWithLifecycle()
    val refused by vm.refused.collectAsStateWithLifecycle()
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) vm.previewFile(uri) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    LazyColumn(contentPadding = PaddingValues(bottom = Space.section + LocalChromeInset.current)) {
        item {
            LargeTitle(say.pluginsGet, trailing = {
                GlyphButton(Iconoir.FolderPlus, say.pluginsInstallFile, Modifier.testTag("plugin-install-file")) { pick.launch(arrayOf("*/*")) }
                GlyphButton(Iconoir.Server, say.pluginRepos, Modifier.testTag("plugin-repos"), onClick = pages.repos)
            })
        }
        item { SearchField(query, vm::search, say.pluginsSearch, Modifier.padding(horizontal = Space.gutter, vertical = 8.dp), testTag = "plugin-search") }
        if (repos.size > 1) item {
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = Space.gutter), Arrangement.spacedBy(6.dp)) {
                Chip(say.pluginsAll, shown == null) { vm.showRepo(null) }
                repos.forEach { r -> Chip(repoName(r.url, r.official), shown == r.url) { vm.showRepo(r.url) } }
            }
        }
        item { Refused(refused) }
        items(offers.failed, key = { it.repo.url }) { f ->
            Quiet(say.pluginRepoUnreachable(repoHost(f.repo.url)), Modifier.padding(horizontal = Space.gutter, vertical = 4.dp))
        }
        if (found.isEmpty()) item {
            Quiet(if (offers.loading) say.pluginsAsking else say.pluginsNoneFound, Modifier.padding(horizontal = Space.gutter, vertical = 16.dp))
        }
        items(found, key = { it.repo.url + it.id }) { o ->
            val version = o.versions.firstOrNull()?.version.orEmpty()
            PluginLine(vm, o.id, o, o.name, o.about, say.pluginOfferLine(version, repoName(o.repo.url, o.repo.official)), true, Modifier.testTag("offer-${o.id}"), { pages.open(o.id) }) {
                when (o.state) {
                    OfferState.NEW -> TextAction(say.pluginInstall, { vm.previewOffer(o) })
                    OfferState.UPDATE -> TextAction(say.pluginUpdate, { vm.previewOffer(o) })
                    OfferState.INSTALLED -> Quiet(say.pluginUpToDate)
                }
            }
        }
    }
    InstallSheet(vm)
    BluetoothAsk(vm, row = false)
}

/** The repos plugins come from, what each offers, and adding one with the title's plus. */
@Composable
fun PluginRepos(vm: PluginsViewModel) {
    val repos by vm.repos.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    var adding by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    Column(Modifier.verticalScroll(rememberScrollState())) {
        LargeTitle(say.pluginRepos, trailing = { GlyphButton(Iconoir.Plus, say.pluginAddRepo, Modifier.testTag("plugin-add-repo")) { adding = true } })
        Spacer(Modifier.height(8.dp))
        GroupCard {
            repos.forEachIndexed { i, r ->
                val line = when {
                    offers.failed.any { it.repo.url == r.url } -> say.pluginRepoUnreachable(repoHost(r.url))
                    offers.loading && offers.offers.none { it.repo.url == r.url } -> say.pluginsAsking
                    else -> say.pluginRepoCount(offers.offers.count { it.repo.url == r.url })
                }
                GroupRow(
                    repoName(r.url, r.official), subtitle = listOfNotNull(r.url.takeIf { !r.official }, line).joinToString("\n"), divider = i < repos.lastIndex,
                    trailing = if (r.official) null else ({ TextAction(say.remove, { vm.removeRepo(r.url) }, destructive = true) }),
                )
            }
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
    AddRepoSheet(adding, { adding = false }) { url -> vm.addRepo(url); adding = false }
}

/** Why an install was refused, under what it was asked from. */
@Composable
private fun Refused(refused: String?) {
    refused?.let { Text(it, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp).testTag("plugin-refused"), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
}

/** A plugin's mark on a tile: its icon in ink, or its first letter without one. */
@Composable
private fun PluginTile(vm: PluginsViewModel, id: String, offer: Offer?, name: String, side: Dp) {
    val ink = MaterialTheme.colorScheme.onSurface
    val icon by produceState(vm.iconRead(id), id) { value = java.util.Optional.ofNullable(vm.icon(id, offer)) }
    Box(Modifier.size(side).background(MaterialTheme.colorScheme.surfaceContainerHighest, TileShape), Alignment.Center) {
        Crossfade(icon, label = "plugin icon") { shown ->
            val bitmap = shown?.orElse(null)
            when {
                bitmap != null -> Image(bitmap.asImageBitmap(), null, Modifier.size(side * 0.6f), colorFilter = ColorFilter.tint(ink))
                shown != null -> Text(name.take(1).uppercase(), style = MaterialTheme.typography.titleMedium.copy(fontSize = (side.value * 0.42f).sp, fontWeight = FontWeight.SemiBold))
            }
        }
    }
}

/** A plugin in a list: its tile, name, what it is on up to two lines, a quiet [line], and what ends the row. */
@Composable
private fun PluginLine(vm: PluginsViewModel, id: String, offer: Offer?, name: String, about: String?, line: String, divider: Boolean, modifier: Modifier, open: () -> Unit, trailing: @Composable () -> Unit) {
    Column(modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth().clickable(onClick = open).padding(horizontal = GROUP_INSET, vertical = 12.dp), Arrangement.spacedBy(12.dp), Alignment.CenterVertically) {
            PluginTile(vm, id, offer, name, 44.dp)
            Column(Modifier.weight(1f)) {
                Text(name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                about?.let { Quiet(it, lines = 2) }
                Text(line, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            trailing()
        }
        if (divider) Hairline(startIndent = GROUP_INSET + 44.dp + 12.dp)
    }
}

/** A line in the quiet ink. */
@Composable
private fun Quiet(text: String, modifier: Modifier = Modifier, lines: Int = Int.MAX_VALUE) = Text(
    text, modifier, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
    maxLines = lines, overflow = TextOverflow.Ellipsis,
)

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
 * A plugin's page, installed or only offered by a repo: what it is, the way to install or update it, its
 * switches and rows, its README, what it may do, the versions its repo offers. The header's menu moves and
 * removes it.
 */
@Composable
fun PluginPage(vm: PluginsViewModel, sound: SettingsViewModel, id: String, back: () -> Unit) {
    SoundPage(sound)
    val cards by vm.cards.collectAsStateWithLifecycle()
    val notice by vm.notice.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    val refused by vm.refused.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<PluginCard?>(null) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    val at = cards.indexOfFirst { it.id == id }
    val card = cards.getOrNull(at)
    val offer = remember(offers, card) { vm.offerOf(id, card) }
    val readme by produceState(emptyList<ReadmeBlock>(), id, offer?.readme) { value = vm.readme(id, offer) }
    val newest = offer?.versions?.firstOrNull()
    Column(Modifier.verticalScroll(rememberScrollState())) {
        val name = card?.name ?: offer?.name ?: return@Column
        Row(Modifier.padding(start = Space.gutter, end = Space.tight, top = 8.dp), Arrangement.spacedBy(14.dp), Alignment.CenterVertically) {
            PluginTile(vm, id, offer, name, 64.dp)
            Column(Modifier.weight(1f)) {
                Text(name, style = titled(MaterialTheme.typography.headlineSmall), maxLines = 2, overflow = TextOverflow.Ellipsis)
                val repo = card?.repo?.let(android.net.Uri::parse)?.host ?: offer?.takeIf { card == null }?.let { repoName(it.repo.url, it.repo.official) }
                Quiet(listOfNotNull(card?.version ?: newest?.version, card?.kinds()?.takeIf { it.isNotEmpty() }, repo, say.pluginFromFile.takeIf { card != null && card.repo == null }).joinToString(" · "))
            }
            if (card != null) MoreButton(listOfNotNull(
                (say.pluginMoveUp to { vm.move(id, -1) }).takeIf { at > 0 },
                (say.pluginMoveDown to { vm.move(id, 1) }).takeIf { at < cards.lastIndex },
                say.remove to { removing = card },
            ))
        }
        Column(Modifier.padding(horizontal = Space.gutter, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            (card?.about ?: offer?.about)?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
            notice?.takeIf { it.plugin == id }?.let { n ->
                Text(n.line, style = MaterialTheme.typography.bodySmall, color = if (n.problem) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (card == null && newest != null) PillButton(say.pluginInstall, null, { vm.previewVersion(newest) }, Modifier.fillMaxWidth().testTag("plugin-install"), prominent = true)
            if (offer?.state == OfferState.UPDATE && newest != null) PillButton(say.pluginUpdateTo(newest.version), null, { vm.previewVersion(newest) }, Modifier.fillMaxWidth().testTag("plugin-update"), prominent = true)
            refused?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("plugin-refused")) }
        }
        if (card != null) {
            GroupCard {
                Toggle(say.pluginOn, "", card.enabled, key = "plugin-on", divider = card.repo != null) { on -> vm.turn(id, on) }
                if (card.repo != null) Toggle(say.pluginKeepVersion, "", card.held, divider = false) { held -> vm.hold(id, held) }
            }
            val controlled by vm.controlled.collectAsStateWithLifecycle()
            controlled.filter { it.card.id == id }.forEach { d -> DeviceControls(vm, d, Modifier.padding(top = Space.section)) }
            if (vm.needsBluetooth(card) && card.enabled) BluetoothAsk(vm, Modifier.padding(top = Space.section))
            PluginRows(vm, card, null)
            PluginAssets(vm, id)
        }
        AnimatedVisibility(readme.isNotEmpty(), enter = fadeIn()) { Readme(readme) }
        if (card != null) {
            GroupLabel(say.pluginMayTitle)
            GroupCard {
                val may = say.pluginMay(card)
                may.forEachIndexed { i, line -> GroupRow(line, minHeight = 44.dp, divider = i < may.lastIndex) }
            }
        }
        if (offer != null) {
            val uri = androidx.compose.ui.platform.LocalUriHandler.current
            GroupLabel(say.pluginVersions)
            GroupCard {
                offer.versions.forEachIndexed { i, v ->
                    GroupRow(
                        v.version, minHeight = 44.dp, value = v.released, divider = i < offer.versions.lastIndex || offer.source != null,
                        trailing = {
                            if (v.version != card?.version) TextAction(say.pluginInstall, { vm.previewVersion(v) }, Modifier.padding(start = 8.dp))
                        },
                    )
                }
                offer.source?.let { src -> GroupRow(say.pluginSource, { runCatching { uri.openUri(src) } }, minHeight = 44.dp, value = android.net.Uri.parse(src).host, chevron = true, divider = false) }
            }
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
    InstallSheet(vm)
    RemoveSheet(removing, { removing = null }) { c -> vm.uninstall(c.id); removing = null; back() }
}

/** A plugin's README under its label. */
@Composable
private fun Readme(blocks: List<ReadmeBlock>) {
    val type = MaterialTheme.typography
    Column {
        GroupLabel(say.pluginAbout)
        Column(Modifier.padding(horizontal = Space.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            blocks.forEach { b ->
                when (b) {
                    is ReadmeBlock.Heading -> Text(b.text, Modifier.padding(top = 6.dp), style = if (b.level <= 2u) type.titleMedium else type.titleSmall)
                    is ReadmeBlock.Text -> Text(b.text, style = type.bodyMedium)
                    is ReadmeBlock.Item -> Row { Text("•", Modifier.padding(end = 8.dp), style = type.bodyMedium); Text(b.text, style = type.bodyMedium) }
                    is ReadmeBlock.Code -> Text(
                        b.text, Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer, TileShape).horizontalScroll(rememberScrollState()).padding(10.dp),
                        style = type.bodySmall.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace), softWrap = false,
                    )
                }
            }
        }
    }
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
