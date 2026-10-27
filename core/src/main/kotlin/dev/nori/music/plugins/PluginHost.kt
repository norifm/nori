package dev.nori.music.plugins

import dev.nori.music.NoriLog
import dev.nori.music.ffi.Kinds
import dev.nori.music.ffi.ProviderInfo
import dev.nori.music.ffi.settings.ServerKind
import dev.nori.music.settings.Settings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import dev.nori.music.ffi.plugins.Client
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.Updated
import dev.nori.music.ffi.plugins.Version
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import dev.nori.music.ffi.plugins.Offers
import dev.nori.music.ffi.plugins.Repo
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.ffi.plugins.openPlugins
import java.io.File
import java.util.Locale

/** A plugin's row, as what plugins say while they run is keyed. */
data class RowRef(val plugin: String, val row: String)

/** Whether a row shows, and whether it can be changed, as its plugin last said. */
data class RowShown(val shown: Boolean, val enabled: Boolean)

/** A line a plugin showed, in its own words. */
data class PluginNotice(val plugin: String, val problem: Boolean, val line: String)

/**
 * The plugin host (crates/plugins), opened once for the process over the app's settings, under [dir]:
 * the kinds of profile every core opens ([kinds]), the cards and offers Settings › Plugins draws, and
 * what the plugins say while they run. Plugins' HTTP goes through [transport], the cores' own. Built off the main thread; every call into it is made off it too.
 */
class PluginHost(private val dir: File, private val settings: Settings, private val transport: () -> dev.nori.music.ffi.net.Transport) {
    private val _cards = MutableStateFlow<List<PluginCard>>(emptyList())
    /** The installed plugins in their order, in the user's language; empty until [refresh]. */
    val cards: StateFlow<List<PluginCard>> = _cards

    private val _providers = MutableStateFlow<List<ProviderInfo>>(emptyList())
    /** The kinds of profile that can be added (the core's `Kinds::providers`), in the order offered. */
    val providers: StateFlow<List<ProviderInfo>> = _providers

    private val _names = MutableStateFlow<Map<String, String>>(emptyMap())
    /** Installed plugins' names by id (the core's `Kinds::plugin_name`), for naming a plugin kind of profile. */
    val names: StateFlow<Map<String, String>> = _names

    private val _repos = MutableStateFlow<List<Repo>>(emptyList())
    /** Where plugins are offered from, nori's own first. */
    val repos: StateFlow<List<Repo>> = _repos

    private val _status = MutableStateFlow<Map<RowRef, String>>(emptyMap())
    /** Status rows' lines, as their plugins last set them. */
    val status: StateFlow<Map<RowRef, String>> = _status

    private val _shown = MutableStateFlow<Map<RowRef, RowShown>>(emptyMap())
    val shown: StateFlow<Map<RowRef, RowShown>> = _shown

    private val _notice = MutableStateFlow<PluginNotice?>(null)
    /** The last line a plugin showed. */
    val notice: StateFlow<PluginNotice?> = _notice

    private val lang get() = Locale.getDefault().language

    // Opened at first use, which is off the main thread: the cheap flows above are read before it is.
    private val plugins by lazy {
        openPlugins(dir.path, settings.core, client).also { p ->
            // The transport is the plugins' own, whichever Kinds over them sets it.
            Kinds.withPlugins(p).setTransport(transport())
            settings.asked(LAYER_EFFECT)
        }
    }

    private val client = object : Client {
        // A plugin changed what it lays over the settings: the player derives its settings again.
        override fun layerChanged() = settings.asked(LAYER_EFFECT)
        override fun status(id: String, row: String, words: String, args: List<String>) {
            val line = said(id, words, args)
            _status.update { it + (RowRef(id, row) to line) }
        }
        override fun shown(id: String, row: String, shown: Boolean, enabled: Boolean) {
            _shown.update { it + (RowRef(id, row) to RowShown(shown, enabled)) }
        }
        override fun notice(id: String, problem: Boolean, words: String, args: List<String>) {
            val line = said(id, words, args)
            NoriLog.i("plugin $id: $line")
            _notice.value = PluginNotice(id, problem, line)
        }
    }

    /** nori's own kinds and the enabled plugins' providers, for every core and the add form. */
    val kinds: Kinds by lazy { Kinds.withPlugins(plugins) }

    /** Plugin [id]'s word [key] with [args] in its `{0}`, `{1}`. */
    private fun said(id: String, key: String, args: List<String>): String =
        args.foldIndexed(plugins.word(id, key, lang) ?: key) { i, line, a -> line.replace("{$i}", a) }

    /** Reads the cards and the kinds again. */
    suspend fun refresh() = withContext(Dispatchers.IO) {
        _cards.value = plugins.cards(lang)
        _repos.value = plugins.repos()
        _providers.value = kinds.providers()
        _names.value = _cards.value.associate { c -> c.id to (kinds.pluginName(ServerKind.Plugin(c.id), lang) ?: c.id) }
    }

    /** Installs the package at [path], off; its id. Throws [dev.nori.music.ffi.plugins.Refusal]. */
    suspend fun install(path: String): String = withContext(Dispatchers.IO) { plugins.installFrom(path).also { refresh() } }

    suspend fun turn(id: String, on: Boolean) = withContext(Dispatchers.IO) { plugins.turn(id, on); refresh() }

    suspend fun place(id: String, index: Int) = withContext(Dispatchers.IO) { plugins.place(id, index.toUInt()); refresh() }

    suspend fun uninstall(id: String) = withContext(Dispatchers.IO) { plugins.uninstall(id); refresh() }

    /** Sets row [row] of plugin [id]; the card takes the value without reading every card again. */
    suspend fun set(id: String, row: String, value: Value) = withContext(Dispatchers.IO) {
        plugins.set(id, row, value)
        _cards.update { cards -> cards.map { c -> if (c.id != id) c else c.copy(rows = c.rows.map { r -> if (r.id == row) r.copy(value = value) else r }) } }
    }

    suspend fun addRepo(url: String) = withContext(Dispatchers.IO) { plugins.addRepo(url); _repos.value = plugins.repos() }

    suspend fun removeRepo(url: String) = withContext(Dispatchers.IO) { plugins.removeRepo(url); _repos.value = plugins.repos() }

    /** What the repos offer, and those that couldn't be read. Asks each repo. */
    suspend fun offers(): Offers = withContext(Dispatchers.IO) { plugins.offers() }

    /** Installs [version] of an offer, newer or older than the one here; its id. Throws [dev.nori.music.ffi.plugins.Refusal]. */
    suspend fun installVersion(version: Version): String = withContext(Dispatchers.IO) { plugins.installVersion(version).also { refresh() } }

    /** Plugin [id] keeps the version it has ([held]) or updates from its repo. */
    suspend fun hold(id: String, held: Boolean) = withContext(Dispatchers.IO) { plugins.holdAt(id, held); refresh() }

    private val _updated = MutableSharedFlow<List<Pair<String, Updated>>>(extraBufferCapacity = 1)
    /** Plugins updated from their repos, each with its name. */
    val updated: SharedFlow<List<Pair<String, Updated>>> = _updated

    /** Updates the plugins from their repos, if it is time to ask them. Called as the app comes to the front. */
    suspend fun updateIfDue() = withContext(Dispatchers.IO) {
        val now = System.currentTimeMillis()
        if (!plugins.updatesDue(now)) return@withContext
        val done = plugins.update(now)
        if (done.isEmpty()) return@withContext
        refresh()
        _updated.tryEmit(done.map { u -> (_names.value[u.id] ?: u.id) to u })
    }

    suspend fun act(id: String, row: String) = withContext(Dispatchers.IO) { plugins.act(id, row) }


    private companion object {
        /** What a new layer asks of the player (settings_store.rs): the chain (1), the gain (2) and the plan (4) again. */
        const val LAYER_EFFECT = 7u
    }
}
