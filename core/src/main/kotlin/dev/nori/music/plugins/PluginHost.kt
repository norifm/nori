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
import dev.nori.music.ffi.plugins.Cast
import dev.nori.music.ffi.plugins.CastButton
import dev.nori.music.ffi.plugins.Client
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.Updated
import dev.nori.music.ffi.plugins.Version
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import dev.nori.music.ffi.plugins.Offers
import dev.nori.music.ffi.plugins.InfoGroup
import dev.nori.music.ffi.plugins.Artwork
import dev.nori.music.ffi.plugins.ItemKind
import dev.nori.music.ffi.plugins.Items
import dev.nori.music.ffi.plugins.MenuEntry
import dev.nori.music.ffi.plugins.PluginPreview
import dev.nori.music.ffi.model.Song
import dev.nori.music.ffi.plugins.Repo
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.ffi.plugins.ControlledDevice
import dev.nori.music.ffi.plugins.Noise
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
 * what the plugins say while they run. Plugins' HTTP goes through [transport], the cores' own, and their
 * Bluetooth through [PluginBluetooth], watched only while an enabled plugin reaches devices. Built off the
 * main thread; every call into it is made off it too.
 */
class PluginHost(context: android.content.Context, private val dir: File, private val settings: Settings, private val transport: () -> dev.nori.music.ffi.net.Transport) {
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

    private val _menus = MutableStateFlow<Map<ItemKind, List<MenuEntry>>>(emptyMap())
    /** The actions plugins add to the menus of each item kind, read with the cards so a menu has them as it opens. */
    val menus: StateFlow<Map<ItemKind, List<MenuEntry>>> = _menus

    private val _deviceCards = MutableStateFlow<List<PluginCard>>(emptyList())
    /** The plugins whose rows show on the devices sheet too. */
    val deviceCards: StateFlow<List<PluginCard>> = _deviceCards

    private val _controlled = MutableStateFlow<List<ControlledDevice>>(emptyList())
    /** The connected devices plugins control, as they last reported them. */
    val controlled: StateFlow<List<ControlledDevice>> = _controlled

    private val bluetooth = PluginBluetooth(context.applicationContext) { plugins }

    private val _notices = MutableSharedFlow<PluginNotice>(extraBufferCapacity = 8)
    /** Each line a plugin shows, as it shows it. */
    val notices: SharedFlow<PluginNotice> = _notices

    private val _status = MutableStateFlow<Map<RowRef, String>>(emptyMap())
    /** Status rows' lines, as their plugins last set them. */
    val status: StateFlow<Map<RowRef, String>> = _status

    private val _shown = MutableStateFlow<Map<RowRef, RowShown>>(emptyMap())
    val shown: StateFlow<Map<RowRef, RowShown>> = _shown

    private val _notice = MutableStateFlow<PluginNotice?>(null)
    /** The last line a plugin showed. */
    val notice: StateFlow<PluginNotice?> = _notice

    private val lang get() = Locale.getDefault().language

    private val _cast = MutableStateFlow<Cast?>(null)
    /** A plugin's cast while one runs: the devices sheet shows it, the playback service plays along. */
    val cast: StateFlow<Cast?> = _cast

    private val lan: LanGlue = LanGlue(context) { plugins }

    // Opened at first use, which is off the main thread: the cheap flows above are read before it is.
    private val plugins: dev.nori.music.ffi.plugins.Plugins by lazy {
        openPlugins(dir.path, settings.core, client).also { p ->
            // The transport is the plugins' own, whichever Kinds over them sets it.
            Kinds.withPlugins(p).setTransport(transport())
            p.useBluetooth(bluetooth)
        }
    }

    private val client = object : Client {
        // A plugin's overrides or rows changed what it lays over the settings: the player applies what that asks.
        override fun layerChanged(effect: UInt) = settings.asked(effect)
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
            PluginNotice(id, problem, line).let { _notice.value = it; _notices.tryEmit(it) }
        }
        // On a plugin's own thread, never the main one.
        override fun devicesChanged() { _controlled.value = plugins.controlled(lang) }
        override fun browse(kind: String, on: Boolean) = lan.browse(kind, on)
        override fun multicast(on: Boolean) = lan.multicast(on)
        override fun cast(cast: Cast?) {
            lan.cast(cast)
            _cast.value = cast
        }
    }

    /** The open profile's client makes songs' URLs for a plugin casting them. */
    fun giveMedia(client: dev.nori.music.ffi.Client) = kinds.giveMedia(client)

    /** A plugin's cast takes [player]'s audio while it runs (crates/android's `castTo`). */
    internal fun castTo(player: dev.nori.music.playback.EnginePlayer) = player.castTo(plugins)

    /** The volume keys or the devices sheet, for the cast running. */
    suspend fun castButton(button: CastButton) = withContext(Dispatchers.IO) { plugins.castButton(button) }

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
        _menus.value = ItemKind.entries.associateWith { plugins.menuEntries(it, lang) }
        _deviceCards.value = plugins.deviceCards(lang)
        _controlled.value = plugins.controlled(lang)
    }

    /** Whether an installed plugin reaches Bluetooth devices, so the permission is asked for. */
    suspend fun reachesBluetooth(): Boolean = withContext(Dispatchers.IO) { plugins.reachesBluetooth() }

    /** The Bluetooth permission was granted: the devices connected are read again. */
    suspend fun bluetoothGranted() = withContext(Dispatchers.IO) { plugins.useBluetooth(bluetooth) }

    suspend fun setNoise(device: String, noise: Noise) = withContext(Dispatchers.IO) { plugins.setNoise(device, noise) }


    /** The package at [path] as its install sheet shows it; nothing is installed. Throws [dev.nori.music.ffi.plugins.Refusal]. */
    suspend fun previewFile(path: String): PluginPreview = withContext(Dispatchers.IO) { plugins.previewFile(path, lang) }

    /** [version] of an offer, fetched and checked, as its install sheet shows it. Throws [dev.nori.music.ffi.plugins.Refusal]. */
    suspend fun previewVersion(version: Version): PluginPreview = withContext(Dispatchers.IO) { plugins.previewVersion(version, lang) }

    /** Installs what [preview] showed, off when new; its id. Throws [dev.nori.music.ffi.plugins.Refusal]. */
    suspend fun installPreview(preview: PluginPreview): String = withContext(Dispatchers.IO) { plugins.installPreview(preview).also { refresh() } }

    suspend fun turn(id: String, on: Boolean) = withContext(Dispatchers.IO) { plugins.turn(id, on); refresh() }

    suspend fun place(id: String, index: Int) = withContext(Dispatchers.IO) { plugins.place(id, index.toUInt()); refresh() }

    suspend fun uninstall(id: String) = withContext(Dispatchers.IO) { plugins.uninstall(id); refresh() }

    /** Sets row [row] of plugin [id]; the card takes the value without reading every card again. */
    suspend fun set(id: String, row: String, value: Value) = withContext(Dispatchers.IO) {
        plugins.set(id, row, value)
        val laid = { cards: List<PluginCard> -> cards.map { c -> if (c.id != id) c else c.copy(rows = c.rows.map { r -> if (r.id == row) r.copy(value = value) else r }) } }
        _cards.update(laid)
        _deviceCards.update(laid)
    }

    suspend fun addRepo(url: String) = withContext(Dispatchers.IO) { plugins.addRepo(url); _repos.value = plugins.repos() }

    suspend fun removeRepo(url: String) = withContext(Dispatchers.IO) { plugins.removeRepo(url); _repos.value = plugins.repos() }

    /** What the repos offer, and those that couldn't be read. Asks each repo. */
    suspend fun offers(): Offers = withContext(Dispatchers.IO) { plugins.offers() }

    /** Plugin [id] keeps the version it has ([held]) or updates from its repo. */
    suspend fun hold(id: String, held: Boolean) = withContext(Dispatchers.IO) { plugins.hold(id, held); refresh() }

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

    /** Runs a plugin's menu action on [items]. */
    suspend fun runMenu(entry: MenuEntry, items: Items) = withContext(Dispatchers.IO) { plugins.runMenu(entry, items) }

    /** The lines plugins add to [song]'s info. */
    suspend fun songInfo(song: Song): List<InfoGroup> = withContext(Dispatchers.IO) { plugins.songInfo(song, lang) }

    /** The sleeve a plugin gives [song], or null for nori's own cover. */
    suspend fun artwork(song: Song): Artwork? = withContext(Dispatchers.IO) { plugins.artwork(song) }

    /** [artwork]'s address answered HTTP [status]: its plugin may look it up again. */
    suspend fun artworkFailed(artwork: Artwork, song: Song, status: Int) = withContext(Dispatchers.IO) { plugins.artworkFailed(artwork, song, status.toUShort()) }

    /** The lines plugins say under the player's output row. */
    suspend fun playerStatus(): List<String> = withContext(Dispatchers.IO) { plugins.playerStatus() }
}
