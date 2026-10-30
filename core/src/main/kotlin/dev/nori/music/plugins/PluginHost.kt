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
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.Offers
import dev.nori.music.ffi.plugins.ReadmeBlock
import dev.nori.music.ffi.plugins.InfoGroup
import dev.nori.music.ffi.plugins.Artwork
import dev.nori.music.ffi.plugins.ItemKind
import dev.nori.music.ffi.plugins.Items
import dev.nori.music.ffi.plugins.MenuEntry
import dev.nori.music.ffi.plugins.PluginPreview
import dev.nori.music.ffi.model.Song
import dev.nori.music.ffi.plugins.Repo
import dev.nori.music.ffi.plugins.SafeStart
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.ffi.plugins.ControlledDevice
import dev.nori.music.ffi.plugins.Noise
import dev.nori.music.ffi.plugins.openPlugins
import dev.nori.music.ffi.plugins.AssetLine
import dev.nori.music.ffi.plugins.AssetState
import dev.nori.music.ffi.plugins.Command
import dev.nori.music.ffi.plugins.Control
import dev.nori.music.ffi.plugins.Hook
import dev.nori.music.ffi.plugins.Moment
import dev.nori.music.ffi.plugins.Press
import dev.nori.music.ffi.plugins.WakeEvent
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
class PluginHost(
    private val context: android.content.Context,
    private val dir: File,
    private val settings: Settings,
    private val transport: () -> dev.nori.music.ffi.net.Transport,
    /** Does what a plugin asked of the player, as a press of the app's own would (PlayerConnection's). */
    private val control: (Control) -> Unit,
    /** A cast's device's button, as a media button's press. */
    private val pressed: (dev.nori.music.ffi.model.Press) -> Unit,
) {
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

    private val _signIns = MutableSharedFlow<Pair<ServerKind, String>>(extraBufferCapacity = 1)
    /** A provider's plugin asks for its sign-in page: its kind and the page's address. */
    val signIns: SharedFlow<Pair<ServerKind, String>> = _signIns

    private val _assets = MutableStateFlow<Map<String, List<AssetLine>>>(emptyMap())
    /** Each plugin's assets and where they stand, by plugin; read with the cards, then as they change. */
    val assets: StateFlow<Map<String, List<AssetLine>>> = _assets

    private val lang get() = Locale.getDefault().language

    private val _cast = MutableStateFlow<Cast?>(null)
    /** A plugin's cast while one runs: the devices sheet shows it, the playback service plays along. */
    val cast: StateFlow<Cast?> = _cast

    private val lan: LanGlue = LanGlue(context) { plugins }
    @Volatile private var opened = false

    // Opened at first use, which is off the main thread: the cheap flows above are read before it is.
    private val plugins: dev.nori.music.ffi.plugins.Plugins by lazy {
        openPlugins(dir.path, settings.core, client).also { p ->
            // The transport is the plugins' own, whichever Kinds over them sets it.
            Kinds.withPlugins(p).setTransport(transport())
            p.useBluetooth(bluetooth, false)
            opened = true
        }
    }

    /** The screen's on and off while a plugin's timer needs them (registered only then). */
    private val screen = object : android.content.BroadcastReceiver() {
        override fun onReceive(c: android.content.Context, intent: android.content.Intent) = plugins.screen(intent.action == android.content.Intent.ACTION_SCREEN_ON)
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
        override fun control(control: Control) = this@PluginHost.control(control)
        override fun streaming(on: Boolean) = lan.streaming(on)
        override fun pressed(press: dev.nori.music.ffi.model.Press) = this@PluginHost.pressed(press)
        override fun signIn(id: String, url: String): Boolean = _signIns.tryEmit(ServerKind.Plugin(id) to url)
        override fun wake(events: List<WakeEvent>) = PluginReceivers.wake(context, events)
        override fun commands(commands: List<Command>) = PluginReceivers.commands(context, commands.isNotEmpty())
        override fun watchScreen(on: Boolean) {
            if (on) {
                val filter = android.content.IntentFilter(android.content.Intent.ACTION_SCREEN_ON).apply { addAction(android.content.Intent.ACTION_SCREEN_OFF) }
                androidx.core.content.ContextCompat.registerReceiver(context, screen, filter, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED)
                plugins.screen(context.getSystemService(android.os.PowerManager::class.java).isInteractive)
            } else {
                runCatching { context.unregisterReceiver(screen) }
            }
        }
        override fun asset(id: String, name: String, state: AssetState) {
            _assets.update { all -> all + (id to all[id].orEmpty().map { a -> if (a.name == name) a.copy(state = state) else a }) }
        }
    }

    /** A plugin's cast takes [player]'s audio while it runs (crates/android's `castTo`). */
    internal fun castTo(player: dev.nori.music.playback.EnginePlayer) = player.castTo(plugins)

    /** The volume keys or the devices sheet, for the cast running. */
    suspend fun castButton(button: CastButton) = withContext(Dispatchers.IO) { plugins.castButton(button) }

    /** [button] for the cast running, from the session's player: whether the cast took it. It only hands the plugin an event. */
    fun castTook(button: CastButton): Boolean = opened && plugins.castButton(button)

    /** Whether a plugin follows the player: only then does the service say what plays ([played]). Never opens the host. */
    fun follows(): Boolean = opened && plugins.follows()

    /** Whether a plugin hears [hook]; false while the host isn't open. */
    fun hears(hook: Hook): Boolean = opened && plugins.hears(hook)

    /** What plays now, while [follows]. */
    suspend fun played(moment: Moment) = withContext(Dispatchers.IO) { plugins.played(moment) }

    suspend fun heardPress(press: Press) = withContext(Dispatchers.IO) { plugins.heardPress(press) }

    /** An output device [name] connected or went, while a plugin [hears] it. */
    suspend fun device(name: String, connected: Boolean) = withContext(Dispatchers.IO) { plugins.device(name, connected) }

    /** The player screen shows (its seek bar's timer runs) or not; nothing while the host isn't open. */
    fun playerShown(shown: Boolean) { if (opened) plugins.playerShown(shown) }

    /** The devices sheet opened or closed: plugins that hear it look for devices only meanwhile. Nothing while the host isn't open. */
    fun outputPicker(open: Boolean) { if (opened) plugins.outputPicker(open) }

    suspend fun woke(event: WakeEvent) = withContext(Dispatchers.IO) { plugins.woke(event) }

    suspend fun command(command: Command) = withContext(Dispatchers.IO) { plugins.command(command) }

    /** The user lets plugin [id] fetch its assets on mobile data. */
    suspend fun allowMobileAssets(id: String) = withContext(Dispatchers.IO) { plugins.allowMobileAssets(id) }

    /** [client]'s profile is the one open: plugins' sessions reach it, and it makes songs' URLs for a plugin casting them. */
    fun openClient(client: dev.nori.music.ffi.Client) = kinds.openClient(client)

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
        _assets.value = _cards.value.associate { c -> c.id to plugins.assets(c.id) }
    }

    /** Whether an installed plugin reaches Bluetooth devices, so the permission is asked for. */
    suspend fun reachesBluetooth(): Boolean = withContext(Dispatchers.IO) { plugins.reachesBluetooth() }

    /** The Bluetooth permission was granted: the devices connected are read again. */
    suspend fun bluetoothGranted() = withContext(Dispatchers.IO) { plugins.useBluetooth(bluetooth, false) }

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

    private val _offers = MutableStateFlow<Offers?>(null)
    /** What the repos last offered, shared by every plugin page; null before [offers]. */
    val offersRead: StateFlow<Offers?> = _offers

    /** Asks each repo what it offers, and which couldn't be read, into [offersRead]. */
    suspend fun offers() = withContext(Dispatchers.IO) { _offers.value = plugins.offers() }

    /** Plugin [id]'s README: its package's, else what [offer] points at. */
    suspend fun readme(id: String, offer: Offer?): List<ReadmeBlock> = withContext(Dispatchers.IO) { plugins.readme(id, offer) }

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

    /** Why plugins are off at this start, with the names of those that stopped nori; null after the first ask. */
    suspend fun takeSafeStart(): Pair<SafeStart, List<String>>? = withContext(Dispatchers.IO) {
        val safe = plugins.takeSafeStart() ?: return@withContext null
        val names = plugins.cards(lang).associate { it.id to it.name }
        safe to ((safe as? SafeStart.Crashed)?.plugins.orEmpty().map { names[it] ?: it })
    }

    /** The app has started (a screen is up, or the player): a crash from now on is no startup crash. */
    suspend fun started() = withContext(Dispatchers.IO) { plugins.started() }

    /** The switch over every plugin changed: the host reads again what runs, then the cards. */
    suspend fun switched() = withContext(Dispatchers.IO) { plugins.switched(); refresh() }

    /** The plugins on, and what they override, for a report of a problem (English). */
    fun report(): String = plugins.report()

    suspend fun act(id: String, row: String) = withContext(Dispatchers.IO) { plugins.act(id, row) }

    /** Runs a plugin's menu action on [items]. */
    suspend fun runMenu(entry: MenuEntry, items: Items) = withContext(Dispatchers.IO) { plugins.runMenu(entry, items) }

    /** The lines plugins add to [song]'s info. */
    suspend fun songInfo(song: Song): List<InfoGroup> = withContext(Dispatchers.IO) { plugins.songInfo(song, lang) }

    /** The sleeve a plugin gives [song], or null for nori's own cover. */
    suspend fun artwork(song: Song): Artwork? = withContext(Dispatchers.IO) { plugins.artwork(song) }

    /** [artwork]'s address answered HTTP [status]: its plugin may look it up again. */
    suspend fun artworkFailed(artwork: Artwork, song: Song, status: Int) = withContext(Dispatchers.IO) { plugins.artworkFailed(artwork, song, status.toUShort()) }

    /** A backup of the settings, sound profiles and plugins, sealed with [password] (nori-backup). Throws its `BackupError`. */
    suspend fun backup(password: String): ByteArray = withContext(Dispatchers.IO) { dev.nori.music.ffi.backup.backupMake(settings.core, plugins, password) }

    /**
     * Restores the backup [file] with [password]: the settings come back in the core (the caller takes in
     * the change it returns), the plugins from their repos. Throws its `BackupError`; nothing changes then.
     */
    suspend fun restore(file: ByteArray, password: String): dev.nori.music.ffi.backup.Restored = withContext(Dispatchers.IO) {
        dev.nori.music.ffi.backup.backupRestore(settings.core, plugins, file, password).also { refresh() }
    }

    /** The lines plugins say under the player's output row. */
    suspend fun playerStatus(): List<String> = withContext(Dispatchers.IO) { plugins.playerStatus() }
}
