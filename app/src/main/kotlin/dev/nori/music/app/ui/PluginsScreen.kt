package dev.nori.music.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.PluginRow
import dev.nori.music.ffi.plugins.RowKind
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.plugins.RowRef

private const val DIMMED = 0.38f

/** A repo as its rows name it: nori's own, else its host. */
private fun repoName(url: String, official: Boolean): String = if (official) say.pluginRepoOfficial else android.net.Uri.parse(url).host ?: url

/**
 * Settings › Plugins: the installed plugins in their order (on or off, moved, removed, each opening its
 * page), installing one from a file, what the repos offer, and the repos.
 */
@Composable
fun PluginsScreen(vm: PluginsViewModel) {
    val nav = LocalNav.current
    val cards by vm.cards.collectAsStateWithLifecycle()
    val repos by vm.repos.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    val said by vm.said.collectAsStateWithLifecycle()
    var adding by remember { mutableStateOf(false) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) vm.installFile(uri) }
    LaunchedEffect(Unit) { vm.loadOffers() }
    Column(Modifier.verticalScroll(rememberScrollState())) {
        BackTitle(say.plugins)
        GroupLabel(say.pluginsInstalled)
        GroupCard {
            if (cards.isEmpty()) GroupRow(say.pluginsNone, divider = false)
            cards.forEachIndexed { i, c ->
                GroupRow(
                    c.name, { nav.plugin(c.id) }, Modifier.testTag("plugin-${c.id}"), subtitle = say.pluginVersion(c.version), divider = i < cards.lastIndex,
                    trailing = {
                        MoreButton(listOfNotNull(
                            (say.pluginMoveUp to { vm.move(c.id, -1) }).takeIf { i > 0 },
                            (say.pluginMoveDown to { vm.move(c.id, 1) }).takeIf { i < cards.lastIndex },
                            say.remove to { vm.uninstall(c.id) },
                        ))
                        NoriSwitch(c.enabled, { on -> vm.turn(c.id, on) }, Modifier.testTag("plugin-on-${c.id}"))
                    },
                )
            }
        }
        Column(Modifier.padding(horizontal = Space.gutter, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            PillButton(say.pluginsInstallFile, null, { pick.launch(arrayOf("*/*")) }, Modifier.fillMaxWidth().testTag("plugin-install-file"))
            said?.let { (refused, line) ->
                Text(
                    line, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("plugin-said"),
                    color = if (refused) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        GroupLabel(say.pluginsGet)
        GroupCard {
            if (offers.loading && offers.offers.isEmpty()) GroupRow(say.pluginsAsking, divider = false)
            offers.offers.forEachIndexed { i, o -> OfferRow(o, i < offers.offers.lastIndex) { vm.installOffer(o) } }
        }
        offers.failed.forEach { f ->
            Text(
                say.pluginRepoUnreachable(android.net.Uri.parse(f.repo.url).host ?: f.repo.url),
                Modifier.padding(horizontal = Space.gutter, vertical = 4.dp),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
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
}

/** An offered plugin: its name and about, its version and repo, and Install, Update or Installed. */
@Composable
private fun OfferRow(o: Offer, divider: Boolean, install: () -> Unit) {
    GroupRow(
        o.name, modifier = Modifier.testTag("offer-${o.id}"),
        subtitle = listOfNotNull(o.about, o.versions.firstOrNull()?.let { say.pluginOfferLine(it.version, repoName(o.repo.url, o.repo.official)) }).joinToString("\n"),
        divider = divider,
        trailing = {
            when (o.installed) {
                o.versions.firstOrNull()?.version -> Text(say.pluginUpToDate, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                null -> TextAction(say.pluginInstall, install)
                else -> TextAction(say.pluginUpdate, install)
            }
        },
    )
}

/** A repo's address, asked under the warning that it isn't nori's: Add is the user agreeing. */
@Composable
private fun AddRepoSheet(visible: Boolean, close: () -> Unit, add: (String) -> Unit) {
    NoriSheet(visible, close) {
        var url by remember { mutableStateOf("") }
        Column(Modifier.padding(horizontal = Space.gutter, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(say.pluginAddRepo, style = MaterialTheme.typography.titleMedium)
            Text(say.pluginRepoWarning, color = MaterialTheme.colorScheme.onSurfaceVariant)
            FormField(
                url, { url = it }, Modifier.fillMaxWidth().testTag("repo-url"), label = { Text(say.pluginRepoUrl) }, singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
            Row(Modifier.fillMaxWidth(), Arrangement.End) {
                TextAction(say.cancel, close)
                TextAction(say.add, { add(url) }, Modifier.testTag("repo-add"), enabled = url.isNotBlank())
            }
        }
    }
}

/**
 * A plugin's page: what it is and does, on or off, the last thing it said, and its rows, drawn with the
 * settings' own components; a section row starts a new card.
 */
@Composable
fun PluginPage(vm: PluginsViewModel, id: String) {
    val nav = LocalNav.current
    val cards by vm.cards.collectAsStateWithLifecycle()
    val status by vm.status.collectAsStateWithLifecycle()
    val shown by vm.shown.collectAsStateWithLifecycle()
    val notice by vm.notice.collectAsStateWithLifecycle()
    val offers by vm.offers.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { vm.loadOffers() }
    val card = cards.firstOrNull { it.id == id } ?: return
    // What the repo it came from offers of it; a plugin from a file updates from nowhere.
    val offer = offers.offers.firstOrNull { it.id == id && it.repo.url == card.repo }
    Column(Modifier.verticalScroll(rememberScrollState())) {
        BackTitle(card.name)
        val facts = listOfNotNull(
            card.about, say.pluginVersion(card.version), say.pluginFromFile.takeIf { card.repo == null },
            card.net.takeIf { it.isNotEmpty() }?.let(say::pluginTalksTo), card.overrides.takeIf { it.isNotEmpty() }?.let(say::pluginChanges),
        )
        Text(
            facts.joinToString("\n"), Modifier.padding(horizontal = Space.gutter, vertical = 8.dp),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        notice?.takeIf { it.plugin == id }?.let { n ->
            Text(
                n.line, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), style = MaterialTheme.typography.bodySmall,
                color = if (n.problem) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        GroupCard {
            Toggle(say.pluginOn, "", card.enabled, key = "plugin-on", divider = card.repo != null) { on -> vm.turn(id, on) }
            if (card.repo != null) Toggle(say.pluginKeepVersion, "", card.held, divider = false) { held -> vm.hold(id, held) }
        }
        // The rows in cards, each section row the label of the card after it.
        val groups = mutableListOf<Pair<String?, MutableList<PluginRow>>>(null to mutableListOf())
        card.rows.filter { shown[RowRef(id, it.id)]?.shown != false }.forEach { r ->
            if (r.kind == RowKind.SECTION) groups += r.label to mutableListOf() else groups.last().second += r
        }
        groups.filter { it.second.isNotEmpty() || it.first != null }.forEach { (title, rows) ->
            if (title != null) GroupLabel(title) else Spacer(Modifier.height(Space.section))
            if (rows.isNotEmpty()) GroupCard {
                rows.forEachIndexed { i, r ->
                    val enabled = shown[RowRef(id, r.id)]?.enabled != false
                    PluginRowView(card, r, status[RowRef(id, r.id)], enabled, i < rows.lastIndex, vm)
                }
            }
        }
        if (offer != null) {
            val uri = androidx.compose.ui.platform.LocalUriHandler.current
            GroupLabel(say.pluginVersions)
            GroupCard {
                offer.versions.forEachIndexed { i, v ->
                    GroupRow(
                        say.pluginVersionLine(v.version, v.released), divider = i < offer.versions.lastIndex || offer.source != null,
                        trailing = {
                            if (v.version == card.version) Text(say.pluginUpToDate, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            else TextAction(say.pluginInstall, { vm.installVersion(v) })
                        },
                    )
                }
                offer.source?.let { src -> GroupRow(say.pluginSource, { runCatching { uri.openUri(src) } }, value = android.net.Uri.parse(src).host, chevron = true, divider = false) }
            }
        }
        Row(Modifier.padding(horizontal = 8.dp, vertical = 12.dp)) {
            TextAction(say.remove, { vm.uninstall(id); nav.back() }, destructive = true)
        }
        Spacer(Modifier.height(Space.section + LocalChromeInset.current))
    }
}

@Composable
private fun PluginRowView(card: PluginCard, r: PluginRow, status: String?, enabled: Boolean, divider: Boolean, vm: PluginsViewModel) {
    val dim = Modifier.alpha(if (enabled) 1f else DIMMED)
    when (r.kind) {
        RowKind.SWITCH -> Toggle(r.label, "", (r.value as? Value.Switch)?.v1 == true, enabled = enabled, divider = divider) { vm.set(card.id, r.id, Value.Switch(it)) }
        RowKind.CHOICE -> {
            var open by remember { mutableStateOf(false) }
            val chosen = (r.value as? Value.Choice)?.v1?.toInt()
            GroupRow(
                r.label, if (enabled) ({ open = true }) else null, dim, value = chosen?.let(r.options::getOrNull), chevron = true, divider = divider,
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
            Text(r.label, dim.padding(start = GROUP_INSET, end = GROUP_INSET, top = 10.dp), style = MaterialTheme.typography.bodySmall)
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
                    text, { text = it }, Modifier.fillMaxWidth(), label = { Text(r.label) }, singleLine = true,
                    visualTransformation = if (r.kind == RowKind.SECRET) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
                )
                if (text != stored) TextAction(say.save, { vm.set(card.id, r.id, Value.Text(text)) }, enabled = enabled)
            }
            if (divider) Hairline(startIndent = GROUP_INSET)
        }
        RowKind.ACTION -> GroupRow(r.label, if (enabled) ({ vm.act(card.id, r.id) }) else null, dim, divider = divider)
        RowKind.STATUS -> GroupRow(r.label, modifier = dim, value = status, divider = divider)
        RowKind.NOTE -> {
            Text(r.label, Modifier.padding(horizontal = GROUP_INSET, vertical = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (divider) Hairline(startIndent = GROUP_INSET)
        }
        RowKind.SECTION -> {}
    }
}
