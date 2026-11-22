package dev.nori.music.app.vm

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.nori.music.Nori
import dev.nori.music.app.R
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.PluginCard
import dev.nori.music.ffi.plugins.Refusal
import dev.nori.music.ffi.plugins.RepoAdd
import dev.nori.music.ffi.plugins.RepoFailure
import dev.nori.music.ffi.plugins.RepoProblem
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.ffi.plugins.Noise
import dev.nori.music.ffi.plugins.PluginPreview
import dev.nori.music.ffi.plugins.Version
import dev.nori.music.ffi.plugins.offerFor
import dev.nori.music.ffi.plugins.offersFound
import dev.nori.music.ffi.plugins.offersToUpdate
import dev.nori.music.ffi.plugins.providerOffers
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A repo being added: [looking] for it, then why it wasn't added. */
data class RepoAdding(val looking: Boolean = false, val problem: RepoProblem? = null)

/** What the repos offer: [loading] while they are asked, then the offers and the repos that couldn't be read. */
data class OffersUi(val loading: Boolean = false, val offers: List<Offer> = emptyList(), val failed: List<RepoFailure> = emptyList())

/**
 * Settings › Plugins, its browser and repos: the installed plugins (the core's cards, in the plugins' own
 * words), what the repos offer, and each plugin's rows. Every call into the plugin host runs off the main thread.
 */
class PluginsViewModel(app: Application) : AndroidViewModel(app) {
    private val nori = Nori.get(app)
    private val host = nori.plugins
    private val res get() = getApplication<Application>().resources

    val cards = host.cards
    val repos = host.repos
    val status = host.status
    /** Each plugin's assets and where they stand. */
    val assets = host.assets
    val shown = host.shown
    val notice = host.notice

    private val asking = MutableStateFlow(false)
    val offers: StateFlow<OffersUi> = combine(host.offersRead, asking) { read, loading -> OffersUi(loading, read?.offers.orEmpty(), read?.failed.orEmpty()) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), OffersUi(false, host.offersRead.value?.offers.orEmpty(), host.offersRead.value?.failed.orEmpty()))

    private val _query = MutableStateFlow("")
    /** What the browser's search says. */
    val query: StateFlow<String> = _query

    private val _repoShown = MutableStateFlow<String?>(null)
    /** The one repo the browser shows; null for all. */
    val repoShown: StateFlow<String?> = _repoShown

    /** The browser's offers: those found, by name. */
    val found: StateFlow<List<Offer>> = combine(offers, _query, _repoShown) { o, q, r -> offersFound(o.offers, q, r) }
        .flowOn(Dispatchers.Default).stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** Plugins that would add a kind of profile if they were on. */
    val providerOffers: StateFlow<List<Offer>> = combine(offers, cards, nori.settings.prefs) { o, c, p -> providerOffers(o.offers, c, p.pluginsOff) }
        .flowOn(Dispatchers.Default).stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** Updates waiting for plugins not kept at their version. */
    val updates: StateFlow<List<Offer>> = combine(offers, cards) { o, c -> offersToUpdate(o.offers, c) }
        .flowOn(Dispatchers.Default).stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** What the repos hold of plugin [id]: from its own repo when installed. */
    fun offerOf(id: String, card: PluginCard?): Offer? = offerFor(offers.value.offers, id, card)

    fun search(query: String) { _query.value = query }

    fun showRepo(url: String?) { _repoShown.value = url }

    /** Plugin [id]'s icon as last read, if it was. */
    fun iconRead(id: String) = host.iconRead(id)

    suspend fun icon(id: String, offer: Offer?) = host.icon(id, offer)

    /** Plugin [id]'s README, empty without one. */
    suspend fun readme(id: String, offer: Offer?) = host.readme(id, offer)

    private val _refused = MutableStateFlow<String?>(null)
    /** Why the last install was refused, in words. */
    val refused: StateFlow<String?> = _refused

    private val _preview = MutableStateFlow<PluginPreview?>(null)
    /** A package read but not installed: its install sheet shows what it is and may reach. */
    val preview: StateFlow<PluginPreview?> = _preview

    /** The sheet put away: nothing is installed. */
    fun dropPreview() { _preview.value = null }

    /** Installs what the sheet showed, on when new; one that [needsBluetooth] asks for the permission. */
    fun installPreview() {
        val p = _preview.value ?: return
        _preview.value = null
        _installing.value = p.card.id
        viewModelScope.launch {
            if (refusing { host.installPreview(p) } != null && needsBluetooth(p.card) && !bluetoothAllowed()) _askBluetooth.value = true
            _installing.value = null
            loadOffers()
        }
    }

    private val _installing = MutableStateFlow<String?>(null)
    /** The plugin whose install is on its way. */
    val installing: StateFlow<String?> = _installing

    private val _fetching = MutableStateFlow<String?>(null)

    /**
     * The plugin whose package is on its way, for its install sheet, or whose install is: no other version is
     * asked for meanwhile, so a tap again on a slow connection doesn't fetch it twice and show its sheet twice.
     */
    val busy: StateFlow<String?> = combine(_fetching, _installing) { f, i -> f ?: i }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** The connected devices plugins control. */
    val controlled = host.controlled

    fun setNoise(device: String, noise: Noise) = viewModelScope.launch { host.setNoise(device, noise) }

    private val _askBluetooth = MutableStateFlow(false)
    /** A plugin that [needsBluetooth] was just installed without the permission: it is asked for. */
    val askBluetooth: StateFlow<Boolean> = _askBluetooth

    /** Android's Bluetooth permission is for reaching devices, and for hearing an audio device connect (its wake). */
    fun needsBluetooth(card: PluginCard): Boolean = card.bluetooth || dev.nori.music.ffi.plugins.WakeEvent.BLUETOOTH_CONNECTED in card.wakes

    fun bluetoothAllowed(): Boolean = android.os.Build.VERSION.SDK_INT < 31 ||
        androidx.core.content.ContextCompat.checkSelfPermission(getApplication(), android.Manifest.permission.BLUETOOTH_CONNECT) == android.content.pm.PackageManager.PERMISSION_GRANTED

    fun bluetoothAnswered(granted: Boolean) {
        _askBluetooth.value = false
        if (granted) viewModelScope.launch { host.bluetoothGranted() }
    }

    /** The plugins whose rows show on the devices sheet too. */
    val deviceCards = host.deviceCards

    /** Rows set in the order they were set: a slider's steps must not land out of order. */
    private val writes = Dispatchers.IO.limitedParallelism(1)

    init {
        viewModelScope.launch { host.refresh() }
    }

    fun loadOffers() {
        if (asking.value) return
        asking.value = true
        viewModelScope.launch { host.offers(); asking.value = false }
    }

    /** Copies the picked package to the cache and shows its install sheet from there. */
    fun previewFile(uri: Uri) = viewModelScope.launch {
        val file = java.io.File(getApplication<Application>().cacheDir, "picked.noriplugin")
        val copied = withContext(Dispatchers.IO) {
            runCatching { getApplication<Application>().contentResolver.openInputStream(uri)?.use { i -> file.outputStream().use { i.copyTo(it) } } }.getOrNull() != null
        }
        if (!copied) { _refused.value = res.getString(R.string.plugins_refused_unreadable); return@launch }
        previewing { host.previewFile(file.path) }
    }

    /**
     * Installs [offer]'s newest version without its sheet and turns it on, for the first run's welcome where
     * picking it asked for it; one installed already only turns on. Whether it is on.
     */
    suspend fun installPicked(offer: Offer): Boolean {
        if (cards.value.none { it.id == offer.id }) {
            val version = offer.versions.firstOrNull() ?: return false
            refusing { host.installPreview(host.previewVersion(version)) } ?: return false
        }
        host.turn(offer.id, true)
        return true
    }

    /** Shows the install sheet of [offer]'s newest version. */
    fun previewOffer(offer: Offer) = offer.versions.firstOrNull()?.let { previewVersion(offer.id, it) }

    /** Shows the install sheet of plugin [id]'s [version], newer or older than the one here, unless one is [busy]. */
    fun previewVersion(id: String, version: Version) {
        if (_fetching.value != null || _installing.value != null) return
        _fetching.value = id
        viewModelScope.launch {
            previewing { host.previewVersion(version) }
            _fetching.value = null
        }
    }

    private suspend fun previewing(read: suspend () -> PluginPreview) {
        _preview.value = refusing(read)
    }

    fun hold(id: String, held: Boolean) = viewModelScope.launch { host.hold(id, held) }

    /** [step]'s answer; a refusal is said instead, and null. */
    private suspend fun <T> refusing(step: suspend () -> T): T? {
        _refused.value = null
        return try {
            step()
        } catch (e: Refusal) {
            _refused.value = refusalWords(e)
            null
        } catch (e: Exception) {
            dev.nori.music.NoriLog.w("plugin install: ${e.message}")
            _refused.value = res.getString(R.string.plugins_refused_other)
            null
        }
    }

    private fun refusalWords(e: Refusal): String = when (e) {
        is Refusal.Unreadable -> res.getString(R.string.plugins_refused_unreadable)
        is Refusal.Api -> res.getString(R.string.plugins_refused_api)
        is Refusal.Needs -> res.getString(R.string.plugins_refused_needs, e.v1.joinToString(", "))
        is Refusal.Id -> res.getString(R.string.plugins_refused_id)
        is Refusal.UnknownKey -> res.getString(R.string.plugins_refused_key, e.v1)
        is Refusal.Signature -> res.getString(R.string.plugins_refused_signature)
        is Refusal.OtherKey -> res.getString(R.string.plugins_refused_other_key)
    }

    fun turn(id: String, on: Boolean) = viewModelScope.launch { host.turn(id, on) }

    /** The devices sheet shows (true) or went (false). */
    fun outputPicker(open: Boolean) { viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { host.outputPicker(open) } }

    /** The switch over every plugin: the setting, then the plugin host told. */
    fun allOff(settings: SettingsViewModel, off: Boolean) {
        if (settings.set("pluginsOff", off.toString())) viewModelScope.launch { host.switched() }
    }

    /** Moves plugin [id] one place [by] (−1 up, 1 down). */
    fun move(id: String, by: Int) {
        val at = cards.value.indexOfFirst { it.id == id }.takeIf { it >= 0 } ?: return
        val to = (at + by).coerceIn(0, cards.value.lastIndex)
        if (to != at) viewModelScope.launch { host.place(id, to) }
    }

    fun uninstall(id: String) = viewModelScope.launch { host.uninstall(id) }

    fun set(id: String, row: String, value: Value) = viewModelScope.launch(writes) { host.set(id, row, value) }

    fun switchFlag(id: String, row: String, at: UInt) = viewModelScope.launch(writes) { host.switchFlag(id, row, at) }

    fun act(id: String, row: String) = viewModelScope.launch { host.act(id, row) }

    fun allowMobileAssets(id: String) = viewModelScope.launch { host.allowMobileAssets(id) }

    private val _repoAdding = MutableStateFlow(RepoAdding())
    /** The repo being added: looked for, or why it wasn't. */
    val repoAdding: StateFlow<RepoAdding> = _repoAdding

    /** The add sheet opened afresh. */
    fun addRepoAsked() { _repoAdding.value = RepoAdding() }

    /** Adds the repo [typed] names, then [added]. */
    fun addRepo(typed: String, added: () -> Unit) = viewModelScope.launch {
        if (_repoAdding.value.looking) return@launch
        _repoAdding.value = RepoAdding(looking = true)
        when (val r = host.addRepo(typed)) {
            is RepoAdd.Added -> { _repoAdding.value = RepoAdding(); added(); loadOffers() }
            is RepoAdd.Failed -> _repoAdding.value = RepoAdding(problem = r.problem)
        }
    }

    fun renameRepo(url: String, name: String) = viewModelScope.launch { host.renameRepo(url, name) }

    fun removeRepo(url: String) = viewModelScope.launch { host.removeRepo(url); loadOffers() }
}
