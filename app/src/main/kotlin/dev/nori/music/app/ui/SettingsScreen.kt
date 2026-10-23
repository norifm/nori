package dev.nori.music.app.ui

import android.content.Intent
import android.media.audiofx.AudioEffect
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import kotlinx.coroutines.launch
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.zIndex
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LifecycleStartEffect
import dev.nori.music.app.vm.SettingsViewModel
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.R
import dev.nori.music.app.vm.SettingRow
import dev.nori.music.app.vm.SettingsSection
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import dev.nori.music.ffi.settings.KeyAction
import dev.nori.music.ffi.settings.SavedServer

/**
 * Search lands on a row, not on a page: the group page is told which row to reveal, the row reports
 * where it is, and the page scrolls there and lets the highlight fade out. A row's key is its title
 * slugged (`settingKey`), the same key the search result carries.
 */
class SettingSpotlight(val key: String?, val onPlaced: (Int) -> Unit)

val LocalSpotlight = androidx.compose.runtime.compositionLocalOf { SettingSpotlight(null) {} }

/** The wash that says "this is the one you searched for", fading out once you have seen it. */
@Composable
private fun Modifier.spotlight(key: String?): Modifier {
    val spot = LocalSpotlight.current
    val on = key != null && spot.key == key
    if (!on) return this
    val fade = remember { androidx.compose.animation.core.Animatable(1f) }
    LaunchedEffect(Unit) {
        kotlinx.coroutines.delay(900)
        fade.animateTo(0f, androidx.compose.animation.core.tween(1400))
    }
    val colour = MaterialTheme.colorScheme.onSurface
    return this
        .onGloballyPositioned { spot.onPlaced(it.positionInWindow().y.toInt()) }
        .drawBehind { drawRect(colour.copy(alpha = 0.10f * fade.value)) }
}

@Composable
fun Toggle(title: String, detail: String, value: Boolean, enabled: Boolean = true, key: String? = null, divider: Boolean = true, onChange: (Boolean) -> Unit) {
    GroupRow(
        title, if (enabled) ({ onChange(!value) }) else null, Modifier.spotlight(key).alpha(if (enabled) 1f else DIMMED),
        subtitle = detail, divider = divider,
        trailing = { NoriSwitch(value, if (enabled) onChange else null) },
    )
}

/** One setting with a few values as chips, the one it has lit; a tap chooses another. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun ChipChoice(row: SettingRow.ChipChoice, divider: Boolean, onChange: (String) -> Unit) {
    Column {
        Column(Modifier.fillMaxWidth().spotlight(row.key).padding(horizontal = GROUP_INSET, vertical = 12.dp).alpha(if (row.enabled) 1f else DIMMED)) {
            Text(row.title, style = MaterialTheme.typography.bodyLarge)
            if (row.detail.isNotEmpty()) Text(row.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            androidx.compose.foundation.layout.FlowRow(
                Modifier.padding(top = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp),
            ) { row.options.forEach { o -> Chip(o.label, o.value == row.chosen) { if (row.enabled && o.value != row.chosen) onChange(o.value) } } }
        }
        if (divider) Hairline(startIndent = GROUP_INSET)
    }
}

/** Conditions that each count on their own, as chips: a tap turns one on or off, any number may be lit. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun Flags(row: SettingRow.Flags, divider: Boolean, onChange: (String) -> Unit) {
    Column {
        Column(Modifier.fillMaxWidth().spotlight(row.key).padding(horizontal = GROUP_INSET, vertical = 12.dp).alpha(if (row.enabled) 1f else DIMMED)) {
            Text(row.title, style = MaterialTheme.typography.bodyLarge)
            if (row.detail.isNotEmpty()) Text(row.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            androidx.compose.foundation.layout.FlowRow(
                Modifier.padding(top = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                row.options.forEach { o ->
                    Chip(o.label, dev.nori.music.ffi.settings.flagOn(row.value, o.value)) {
                        if (row.enabled) onChange(dev.nori.music.ffi.settings.flagToggled(row.value, o.value))
                    }
                }
            }
        }
        if (divider) Hairline(startIndent = GROUP_INSET)
    }
}

/**
 * The four seasons as tiles, each its season's paper with its kanji in its accent and its name under it, the
 * one worn ringed in that accent. Picking a tile stops following; following again lets the calendar choose.
 */
@Composable
private fun SeasonPicker(row: SettingRow.Seasons, onChange: (String) -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val following = row.chosen == row.following
    val worn = if (following) row.now else row.tiles.indexOfFirst { it.value == row.chosen }
    Column(Modifier.spotlight(row.key)) {
        GroupLabel(row.title)
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), Arrangement.spacedBy(10.dp)) {
            row.tiles.forEachIndexed { i, t ->
                val chosen = i == worn
                val accent = Color(t.accent)
                Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        Modifier.fillMaxWidth().height(72.dp).clip(TileShape).background(Color(t.page))
                            .border(if (chosen) 2.dp else androidx.compose.ui.unit.Dp.Hairline, if (chosen) accent else scheme.outlineVariant, TileShape)
                            .clickable { if (t.value != row.chosen) onChange(t.value) }
                            .semantics { contentDescription = t.label; selected = chosen },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(Kanji.seasons[t.season.ordinal], null, Modifier.size(32.dp), tint = accent)
                    }
                    Text(
                        t.label, Modifier.padding(top = 6.dp), style = MaterialTheme.typography.labelMedium, maxLines = 1,
                        color = if (chosen) scheme.onSurface else scheme.onSurfaceVariant,
                    )
                }
            }
        }
        GroupCard(Modifier.padding(top = 10.dp)) {
            Toggle(row.followTitle, row.follow, following, divider = false) { on -> onChange(if (on) row.following else row.tiles[row.now].value) }
        }
    }
}

/** What a setting the app is going to ignore looks like: still there, still readable, plainly not live. */
private const val DIMMED = 0.38f

@Composable
private fun Choice(row: SettingRow.Choice, divider: Boolean, onChange: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    GroupRow(
        row.title, if (row.enabled) ({ open = true }) else null, Modifier.spotlight(row.key).alpha(if (row.enabled) 1f else DIMMED),
        value = row.shown, chevron = true, divider = divider,
        trailing = { NoriPopover(open, { open = false }) { row.options.forEach { o -> PopoverItem(o.label, chosen = o.label == row.shown) { onChange(o.value); open = false } } } },
    )
}

/** Each group's icon; the groups themselves, their order and their words are SettingsPages.kt's. */
private fun groupIcon(id: String): ImageVector = when (id) {
    "servers" -> Iconoir.Server
    "playing" -> Iconoir.Play
    "sound" -> Iconoir.ControlSlider
    "look" -> Iconoir.Palette
    "lyrics" -> Iconoir.EmojiSingRightNote
    "library" -> Iconoir.AlbumList
    "data" -> Iconoir.CloudDownload
    "about" -> Iconoir.InfoCircle
    else -> Iconoir.Sparks
}

/** A titled group: small-caps label over a card of rows. */
@Composable
private fun Section(title: String, content: @Composable ColumnScope.() -> Unit) {
    GroupLabel(title)
    GroupCard(content = content)
}

/** A row with a line of text and a button at its end: a count, an action. */
/**
 * What the compressor is taking off right now: a line and a bar. Read from the player only while this
 * page is [seen], the way the equalizer screen reads the limiter's meter, and dropped the moment it is
 * not: nothing ticks with the screen off. Its own scope, so a reading redraws only this row.
 */
@Composable
private fun CompressionMeter(enabled: Boolean, divider: Boolean) {
    val db = meterReading(viewModel<SettingsViewModel>().compressionDb)
    val active = db > 0.05f
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth().padding(horizontal = GROUP_INSET, vertical = 8.dp).alpha(if (enabled) 1f else DIMMED)) {
        // The words change only with the tenth of a dB they show; the bar moves far more finely.
        Text(
            remember(kotlin.math.round(db * 10f)) { say.compression(db) },
            style = MaterialTheme.typography.labelMedium,
            color = if (active) scheme.onSurface else scheme.onSurfaceVariant,
        )
        val track = scheme.surfaceVariant
        val fill = scheme.onSurface
        // Full width is 20 dB of reduction, more than any preset takes off music.
        Spacer(
            Modifier.padding(top = 6.dp).fillMaxWidth().height(4.dp).drawBehind {
                val r = androidx.compose.ui.geometry.CornerRadius(size.height / 2f)
                drawRoundRect(track, cornerRadius = r)
                val w = size.width * (db / 20f).coerceIn(0f, 1f)
                if (w > 0f) drawRoundRect(fill, size = size.copy(width = w), cornerRadius = r)
            },
        )
    }
    if (divider) Hairline(startIndent = GROUP_INSET)
}

@Composable
private fun ActionRowSetting(row: SettingRow.Action, divider: Boolean, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().spotlight(row.key).padding(start = GROUP_INSET, end = 8.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).alpha(if (row.enabled) 1f else DIMMED)) {
            Text(row.title, style = MaterialTheme.typography.bodyLarge)
            Text(row.detail, style = MaterialTheme.typography.bodySmall, color = if (row.error) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
        }
        TextAction(row.button, onClick, enabled = row.enabled)
    }
    if (divider) Hairline(startIndent = GROUP_INSET)
}

@Composable
fun SettingsScreen(vm: SettingsViewModel) {
    val nav = LocalNav.current
    var query by remember { mutableStateOf("") }
    val res = LocalContext.current.resources
    val config = LocalConfiguration.current
    // Titles first, then anything whose explanation mentions it, asked once per change of the query (or
    // of the language).
    val hits = remember(query, config) { if (query.isBlank()) emptyList() else vm.searchSettings(query, res) }
    val groups = remember(config) { vm.settingsGroups(res) }
    val perf = dev.nori.music.app.PerfHooks.recorder != null
    Column {
        LargeTitle(say.settings)
        SearchField(query, { query = it }, stringResource(R.string.settings_search), Modifier.padding(horizontal = Space.gutter, vertical = 6.dp))
        if (query.isNotBlank()) {
            // A result is the setting itself: tapping opens its page and puts the finger on the row.
            LazyColumn(contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
                items(hits, key = { it.group + it.title }) { e ->
                    Column(Modifier.clickable { nav.settingsGroup(e.group, e.key) }) {
                        Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 11.dp)) {
                            Text(e.title, style = MaterialTheme.typography.bodyLarge)
                            Text(
                                e.detail,
                                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                            )
                        }
                        Hairline(startIndent = Space.gutter)
                    }
                }
                if (hits.isEmpty()) item { EmptyNote(stringResource(R.string.settings_nothing_matches, query)) }
            }
            return@Column
        }
        LazyColumn(contentPadding = PaddingValues(top = 8.dp, bottom = LocalChromeInset.current)) {
            item(key = "groups") {
                GroupCard {
                    groups.forEachIndexed { i, g ->
                        // Only the perf build has a recorder, and with it the last row (docs/site/developers/building.md).
                        GroupRow(g.title, { nav.settingsGroup(g.id) }, subtitle = g.summary, icon = groupIcon(g.id), chevron = true, divider = i < groups.lastIndex || perf)
                    }
                    if (perf) GroupRow(say.performance, { nav.go("perf") }, subtitle = say.performanceDetail, icon = Iconoir.DashboardSpeed, chevron = true, divider = false)
                }
            }
        }
    }
}

/** One group's page: its rows on their plates, scrolled to whichever row the search sent us to. */
@Composable
fun SettingsGroupScreen(vm: SettingsViewModel, id: String, highlight: String = "", actions: dev.nori.music.app.vm.ActionsViewModel? = null) {
    val nav = LocalNav.current
    val p by vm.prefs.collectAsStateWithLifecycle()
    val facts by vm.settingsFacts.collectAsStateWithLifecycle()
    // The page's facts, fetched while it is open: the songs AutoMix measured, what is stored on the
    // phone, the server's music folders.
    LaunchedEffect(id, p.autoMix) { if (id == "playing" && p.autoMix) vm.refreshAnalysed() }
    LaunchedEffect(id) { if (id == "data") vm.refreshStorage() }
    if (id == "sound") LifecycleStartEffect(vm) { vm.soundPageShown(true); onStopOrDispose { vm.soundPageShown(false) } }
    LaunchedEffect(id, p.activeServerId) { if (id == "servers") vm.loadMusicFolders() }
    val res = LocalContext.current.resources
    val config = LocalConfiguration.current
    // Built once for the whole page, again only when the settings, its facts or the language change.
    val page = remember(id, p, facts, config) { vm.settingsPage(id, p, facts, res) } ?: return
    val scroll = rememberScrollState()
    var target by remember { mutableIntStateOf(-1) }
    LaunchedEffect(target) { if (target >= 0) scroll.animateScrollTo((scroll.value + target - 400).coerceAtLeast(0)) }
    CompositionLocalProvider(LocalSpotlight provides SettingSpotlight(highlight.ifEmpty { null }) { y -> if (target < 0) target = y }) {
        Column(Modifier.verticalScroll(scroll)) {
            BackTitle(page.title)
            when (id) {
                // The build's facts, then the rows the page builder made for About (the update check).
                "about" -> AboutContent({ title, content -> Section(title, content) }, { nav.settingsGroup("licences") }, vm::reportProblem) {
                    page.sections.forEach { s -> SettingsSectionRows(vm, s, actions) }
                }
                "licences" -> LicencesContent { title, content -> Section(title, content) }
                else -> page.sections.forEach { s -> SettingsSectionRows(vm, s, actions) }
            }
            Spacer(Modifier.height(Space.section + LocalChromeInset.current))
        }
    }
}

/** A lyrics service's key, asked on its row: saved, the service is on; a cleared PaxSenix key turns off every
 *  service that needs it; cancelled, nothing changes. */
@Composable
private fun KeyDialog(row: SettingRow.Ranked, onDismiss: () -> Unit, onSave: (String) -> Unit) {
    var text by remember(row.id) { mutableStateOf("") }
    AlertCard(
        title = { Text(say.serviceKey(row.title)) },
        text = {
            Column {
                Text(if (row.keySet) (if (row.keyed) say.paxsenixKeyChange else say.keyChange) else if (row.keyed) say.paxsenixKeyAsk else say.betterLyricsKeyAsk, Modifier.padding(bottom = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                FormField(text, { text = it }, Modifier.fillMaxWidth(), singleLine = true, visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation())
            }
        },
        confirmButton = { TextAction(say.save, { onSave(text) }, enabled = !row.keyed || row.keySet || text.isNotBlank()) },
        dismissButton = { TextAction(say.cancel, onDismiss) },
    )
}

/** One section of a page as SettingsPages.kt laid it out; this only draws the rows and hands back what was picked. */
@Composable
private fun SettingsSectionRows(vm: SettingsViewModel, section: SettingsSection, actions: dev.nori.music.app.vm.ActionsViewModel?) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val p by vm.prefs.collectAsStateWithLifecycle()
    var editing by remember { mutableStateOf<SavedServer?>(null) }
    var asking by remember { mutableStateOf<Pair<String, dev.nori.music.app.vm.ActionAsk>?>(null) }
    NoriDialog(editing, { editing = null }, DialogStyle.Page) { e -> LoginScreen(vm, e) { editing = null } }
    val act: (String) -> Unit = { action ->
        when (action) {
            "equalizer" -> nav.equalizer()
            "home-rows" -> nav.homeRows()
            "library-sections" -> nav.librarySections()
            "system-effects" -> runCatching {
                context.startActivity(
                    Intent(AudioEffect.ACTION_DISPLAY_AUDIO_EFFECT_CONTROL_PANEL).putExtra(AudioEffect.EXTRA_PACKAGE_NAME, context.packageName)
                        .putExtra(AudioEffect.EXTRA_CONTENT_TYPE, AudioEffect.CONTENT_TYPE_MUSIC),
                )
            }
            "downloads" -> nav.downloads()
            // Asks first whether the beat model reads them too, as Download does.
            "analyse-downloads" -> actions?.analyseDownloads()
            "add-server" -> editing = vm.newProfile()
            // A page of its own inside this one (the lyrics sources), as the row names it.
            else -> when {
                action.startsWith("page:") -> nav.settingsGroup(action.removePrefix("page:"))
                // Whether it asks first, and what it says (`settingsActionAsks`).
                else -> vm.actionAsks(action, context.resources)?.let { asking = action to it } ?: vm.act(action)
            }
        }
    }
    NoriDialog(asking, { asking = null }) { (action, ask) ->
        AlertCard(
            title = { Text(ask.title) },
            text = { Text(ask.text) },
            confirmButton = { TextAction(ask.confirm, { vm.act(action); asking = null }) },
            dismissButton = { TextAction(say.cancel, { asking = null }) },
        )
    }
    // A ranked row held and dragged (the lyrics services, one list whether on or off): which one, in
    // what order the rows were when it was picked up and where it would go, how far the finger has gone,
    // and each row's height (they differ: a service's line runs to two or three), which places are
    // measured in. The ranking is only changed when it is let go,
    // in one step (`lyricsPlace`): moving a row's composition while a finger is on it cancels the gesture,
    // and the rest of the drag then scrolled the page instead.
    var drag by remember { mutableStateOf<RankDrag?>(null) }
    var fingerY by remember { mutableFloatStateOf(0f) }
    val heights = remember { androidx.compose.runtime.mutableStateMapOf<String, Float>() }
    /** Where row [id] starts, in pixels from the first, with the rows in [list]'s order. */
    fun top(list: List<String>, id: String): Float { var y = 0f; for (o in list) { if (o == id) break; y += heights[o] ?: 0f }; return y }
    val order = section.rows.mapNotNull { (it as? SettingRow.Ranked)?.id }
    // The order as it is now, for a gesture that outlives the composition it started in.
    val orderNow by androidx.compose.runtime.rememberUpdatedState(order)
    val shown = drag?.shown ?: order
    // Once the core's order is the one shown, the drag is over.
    drag?.let { d -> if (!d.active && d.shown == order) androidx.compose.runtime.SideEffect { drag = null } }
    var keying by remember { mutableStateOf<SettingRow.Ranked?>(null) }
    NoriDialog(keying, { keying = null }) { row -> KeyDialog(row, { keying = null }) { v -> vm.set(row.keyName, v); keying = null } }
    // Each row under a key of its own, so a ranked row moved in the core's order is the same row, its
    // glide and lift carried with it: keyed inside the `when` instead, a moved row began again from
    // nothing and was drawn at its new place in one frame.
    @Composable
    fun RowOf(at: Int, row: SettingRow, divider: Boolean) = androidx.compose.runtime.key((row as? SettingRow.Ranked)?.id ?: "#$at") {
            when (row) {
                is SettingRow.Toggle -> Toggle(row.title, row.detail, row.on, enabled = row.enabled, key = row.key) { on -> vm.set(row.name, on.toString()) }
                is SettingRow.Choice -> Choice(row, divider) { v -> vm.set(row.name, v) }
                is SettingRow.ChipChoice -> ChipChoice(row, divider) { v -> vm.set(row.name, v) }
                is SettingRow.Flags -> Flags(row, divider) { v -> vm.set(row.name, v) }
                is SettingRow.Note -> Text(row.text, Modifier.padding(horizontal = GROUP_INSET, vertical = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                is SettingRow.Link -> GroupRow(
                    row.title, if (row.enabled) ({ act(row.action) }) else null,
                    Modifier.spotlight(row.key).alpha(if (row.dimmed || !row.enabled) DIMMED else 1f),
                    value = row.status, chevron = true, divider = divider,
                )
                is SettingRow.Action -> ActionRowSetting(row, divider) { act(row.action) }
                is SettingRow.Usage -> UsageBar(row, divider)
                is SettingRow.Info -> GroupRow(row.title, modifier = Modifier.spotlight(row.key), subtitle = row.detail, divider = divider)
                is SettingRow.Slider -> {
                    Text(row.label, Modifier.padding(start = GROUP_INSET, end = GROUP_INSET, top = 10.dp), style = MaterialTheme.typography.bodySmall)
                    // A slider with a level is edited in place on every step, like the equalizer's.
                    NoriSlider(row.value, row.min..row.max, { v -> row.level?.let { vm.setLevel(it, v) } ?: vm.set(row.name, v.toString()) }, Modifier.padding(horizontal = GROUP_INSET), centred = row.centred)
                    if (divider) Hairline(startIndent = GROUP_INSET)
                }
                is SettingRow.CompressionMeter -> CompressionMeter(row.enabled, divider)
                is SettingRow.Seasons -> SeasonPicker(row) { v -> vm.set(row.name, v) }
                is SettingRow.Server -> {
                    Row(Modifier.fillMaxWidth().clickable(enabled = !row.active) { p.servers.firstOrNull { it.id == row.id }?.let(vm::switchServer) }.padding(start = GROUP_INSET), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(row.label, color = if (row.active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface)
                            Text(row.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        // A jam guest switches between the profiles; changing them is not its to do.
                        if (dev.nori.music.ffi.settings.SettingsPart.SERVER_OPTIONS in LocalRules.current.settings) {
                            TextAction(say.edit, { editing = p.servers.firstOrNull { it.id == row.id } })
                            TextAction(say.remove, { vm.removeServer(row.id) }, destructive = true)
                        }
                    }
                    // The server's own libraries, under it: which of them the app shows.
                    if (row.libraries.isNotEmpty()) Column(Modifier.fillMaxWidth().padding(start = GROUP_INSET, end = GROUP_INSET, bottom = 12.dp)) {
                        Text(row.note, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        androidx.compose.foundation.layout.FlowRow(
                            Modifier.padding(top = 8.dp),
                            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) { row.libraries.forEach { l -> Chip(l.label, l.on) { vm.set(l.name, l.tapped) } } }
                    }
                    if (divider) Hairline(startIndent = GROUP_INSET)
                }
                is SettingRow.Button -> TextAction(row.title, { act(row.action) }, Modifier.padding(horizontal = 8.dp))
                // Each row is drawn at its place in the order shown, gliding there, while it is laid out in the
                // core's: the neighbours make way as the held row passes them, and when it is let go it settles
                // into its place as the core's order catches up, with nothing jumping at either end.
                is SettingRow.Ranked -> {
                    val haptics = androidx.compose.ui.platform.LocalHapticFeedback.current
                    // The held row is lifted over its neighbours, so it needs the plate's own colour behind its words.
                    val plate = MaterialTheme.colorScheme.surfaceContainer
                    val scope = androidx.compose.runtime.rememberCoroutineScope()
                    val mine = drag?.id == row.id
                    val following = mine && drag?.active == true
                    // Where it is drawn, from the top of the list: it glides to its place in the order shown when a
                    // drag moves it, and is simply there when anything else does (rows being measured, a page
                    // opening). Not restarted when the drag ends, so a row settling finishes its glide.
                    val shownY = top(shown, row.id)
                    val slot = remember { androidx.compose.animation.core.Animatable(shownY) }
                    // Whether a drag moved it is read here, in the composition that moved it: by the time the effect
                    // starts the core may already have taken the drop and ended the drag.
                    val glide = drag != null
                    LaunchedEffect(shownY, following) {
                        if (following) return@LaunchedEffect
                        if (!glide) slot.snapTo(shownY)
                        else slot.animateTo(shownY, androidx.compose.animation.core.tween(if (AppMotion.reduce) 0 else 200))
                    }
                    // Picked up and put down over a moment, not in a frame: its lift, shadow and plate.
                    val lift = androidx.compose.animation.core.animateFloatAsState(
                        if (following) 1f else 0f, androidx.compose.animation.core.tween(if (AppMotion.reduce) 0 else 160), label = "lift",
                    )
                    val above by remember { androidx.compose.runtime.derivedStateOf { lift.value > 0f } }
                    Column(
                        Modifier.fillMaxWidth()
                            .zIndex(if (mine || above) 1f else 0f)
                            .graphicsLayer {
                                val at = if (following) top(drag?.from ?: order, row.id) + fingerY else slot.value
                                translationY = at - top(order, row.id)
                                val l = lift.value
                                shadowElevation = 14f * l; scaleX = 1f + 0.02f * l; scaleY = 1f + 0.02f * l
                            }
                            .drawBehind { lift.value.takeIf { it > 0f }?.let { drawRect(plate.copy(alpha = plate.alpha * it)) } }
                            .onGloballyPositioned { val h = it.size.height.toFloat(); if (heights[row.id] != h) heights[row.id] = h },
                    ) {
                        // Held anywhere, like the home page's rows: a long press picks it up (a plain drag
                        // scrolls the page, and never moves a row), with a tick and the row lifting. The
                        // switch still turns the service on or off where it stands with a tap.
                        Row(
                            Modifier.fillMaxWidth().spotlight(row.key).pointerInput(row.id) {
                                detectDragGesturesAfterLongPress(
                                    onDragStart = {
                                        drag = RankDrag(row.id, orderNow, orderNow.indexOf(row.id), active = true); fingerY = 0f
                                        haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                                    },
                                    onDragEnd = {
                                        val d = drag ?: return@detectDragGesturesAfterLongPress
                                        // From where the finger left it; the core is told once, where it was dropped.
                                        scope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { slot.snapTo(top(d.from, row.id) + fingerY) }
                                        val placed = d.to == d.from.indexOf(row.id) || vm.placeRanked(row.id, d.to)
                                        drag = if (placed) d.copy(active = false) else d.copy(to = d.from.indexOf(row.id), active = false)
                                    },
                                    onDragCancel = {
                                        val d = drag ?: return@detectDragGesturesAfterLongPress
                                        scope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { slot.snapTo(top(d.from, row.id) + fingerY) }
                                        drag = d.copy(to = d.from.indexOf(row.id), active = false)
                                    },
                                ) { change, moved ->
                                    change.consume()
                                    fingerY += moved.y
                                    val d = drag ?: return@detectDragGesturesAfterLongPress
                                    // Its place: after every other row whose middle its own middle has passed.
                                    val middle = top(d.from, row.id) + fingerY + (heights[row.id] ?: 0f) / 2f
                                    var y = 0f
                                    var to = 0
                                    for (o in d.from) {
                                        if (o == row.id) continue
                                        val h = heights[o] ?: 0f
                                        if (middle > y + h / 2f) to++
                                        y += h
                                    }
                                    if (to != d.to) {
                                        drag = d.copy(to = to)
                                        haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                                    }
                                }
                            }.padding(start = GROUP_INSET, top = 12.dp, bottom = 12.dp, end = GROUP_INSET),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f).padding(end = 14.dp)) {
                                Text(row.title, style = MaterialTheme.typography.bodyLarge, color = if (row.on) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
                                Text(row.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                // Under the words, in line with them: a key is set where the service is.
                                val keyLabel = when (row.keyButton) { KeyAction.CHANGE -> say.changeKey; KeyAction.ADD -> say.addKey; KeyAction.NONE -> null }
                                if (keyLabel != null) Text(
                                    keyLabel, Modifier.padding(top = 6.dp).clickable { keying = row },
                                    style = MaterialTheme.typography.labelLarge,
                                )
                            }
                            NoriSwitch(row.on, { on -> if (on && row.asksKey) keying = row else vm.set(row.name, on.toString()) })
                        }
                        if (divider) Hairline(startIndent = GROUP_INSET)
                    }
                }
            }
    }
    // The season picker stands on the page above the group's card.
    val (seasons, rows) = section.rows.partition { it is SettingRow.Seasons }
    seasons.forEachIndexed { at, row -> RowOf(at, row, divider = false) }
    if (rows.isNotEmpty()) Section(section.title) { rows.forEachIndexed { at, row -> RowOf(at, row, divider = at < rows.lastIndex) } }
}

/** A ranked row held: [id], the order when it was picked up ([from]), the place it would go ([to]), and whether the finger is still on it. */
private data class RankDrag(val id: String, val from: List<String>, val to: Int, val active: Boolean) {
    /** The order drawn: [from] with the held row moved to [to]. */
    val shown: List<String> = from.toMutableList().apply { remove(id); add(to, id) }
}

/** How much of the ink each usage part takes, in [SettingRow.Usage.parts] order: the biggest reads strongest, the last still clears the track. */
private val USAGE_INK = listOf(0.92f, 0.68f, 0.5f, 0.36f, 0.26f)

/** What is stored on this device: the total, one bar split by kind, and a line per kind with its dot. */
@Composable
private fun UsageBar(row: SettingRow.Usage, divider: Boolean) {
    val total = row.parts.sumOf { it.bytes }.coerceAtLeast(1)
    val scheme = MaterialTheme.colorScheme
    val colors = USAGE_INK.map { androidx.compose.ui.graphics.lerp(scheme.surfaceContainer, scheme.onSurface, it) }
    Column(Modifier.fillMaxWidth().spotlight(row.key).padding(horizontal = GROUP_INSET, vertical = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(row.title, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
            Text(row.total, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Row(
            Modifier.padding(top = 10.dp, bottom = 10.dp).fillMaxWidth().height(10.dp).clip(androidx.compose.foundation.shape.RoundedCornerShape(5.dp))
                .background(MaterialTheme.colorScheme.outlineVariant),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            row.parts.forEachIndexed { i, p ->
                if (p.bytes > 0) Box(Modifier.weight(p.bytes.toFloat() / total).fillMaxHeight().background(colors[i % colors.size]))
            }
        }
        row.parts.forEachIndexed { i, p ->
            Row(Modifier.padding(vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).clip(androidx.compose.foundation.shape.CircleShape).background(colors[i % colors.size]))
                Text(p.label, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(p.size, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
    if (divider) Hairline(startIndent = GROUP_INSET)
}
