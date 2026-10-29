package dev.nori.music.app.vm

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.nori.music.Nori
import dev.nori.music.app.R
import dev.nori.music.ffi.plugins.Offer
import dev.nori.music.ffi.plugins.Refusal
import dev.nori.music.ffi.plugins.RepoFailure
import dev.nori.music.ffi.plugins.Value
import dev.nori.music.ffi.plugins.Noise
import dev.nori.music.ffi.plugins.PluginPreview
import dev.nori.music.ffi.plugins.Version
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** What the repos offer: [loading] while they are asked, then the offers and the repos that couldn't be read. */
data class OffersUi(val loading: Boolean = false, val offers: List<Offer> = emptyList(), val failed: List<RepoFailure> = emptyList())

/**
 * Settings › Plugins: the installed plugins (the core's cards, in the plugins' own words), what the repos
 * offer, and each plugin's rows. Every call into the plugin host runs off the main thread.
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

    private val _offers = MutableStateFlow(OffersUi())
    val offers: StateFlow<OffersUi> = _offers

    private val _refused = MutableStateFlow<String?>(null)
    /** Why the last install was refused, in words. */
    val refused: StateFlow<String?> = _refused

    private val _preview = MutableStateFlow<PluginPreview?>(null)
    /** A package read but not installed: its install sheet shows what it is and may reach. */
    val preview: StateFlow<PluginPreview?> = _preview

    /** The sheet put away: nothing is installed. */
    fun dropPreview() { _preview.value = null }

    /** Installs what the sheet showed, off when new; one reaching Bluetooth devices asks for the permission. */
    fun installPreview() {
        val p = _preview.value ?: return
        _preview.value = null
        viewModelScope.launch {
            if (refusing { host.installPreview(p) } != null && p.card.bluetooth && !bluetoothAllowed()) _askBluetooth.value = true
            loadOffers()
        }
    }

    /** The connected devices plugins control. */
    val controlled = host.controlled

    fun setNoise(device: String, noise: Noise) = viewModelScope.launch { host.setNoise(device, noise) }

    private val _askBluetooth = MutableStateFlow(false)
    /** A plugin reaching Bluetooth devices was just installed without the permission: it is asked for. */
    val askBluetooth: StateFlow<Boolean> = _askBluetooth

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
        if (_offers.value.loading) return
        _offers.value = _offers.value.copy(loading = true)
        viewModelScope.launch { _offers.value = host.offers().let { OffersUi(false, it.offers, it.failed) } }
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

    /** Shows the install sheet of [offer]'s newest version. */
    fun previewOffer(offer: Offer) = offer.versions.firstOrNull()?.let(::previewVersion)

    /** Shows the install sheet of [version], newer or older than the one here. */
    fun previewVersion(version: Version) = viewModelScope.launch { previewing { host.previewVersion(version) } }

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
    }

    fun turn(id: String, on: Boolean) = viewModelScope.launch { host.turn(id, on) }

    /** Moves plugin [id] one place [by] (−1 up, 1 down). */
    fun move(id: String, by: Int) {
        val at = cards.value.indexOfFirst { it.id == id }.takeIf { it >= 0 } ?: return
        val to = (at + by).coerceIn(0, cards.value.lastIndex)
        if (to != at) viewModelScope.launch { host.place(id, to) }
    }

    fun uninstall(id: String) = viewModelScope.launch { host.uninstall(id) }

    fun set(id: String, row: String, value: Value) = viewModelScope.launch(writes) { host.set(id, row, value) }

    fun act(id: String, row: String) = viewModelScope.launch { host.act(id, row) }

    fun allowMobileAssets(id: String) = viewModelScope.launch { host.allowMobileAssets(id) }

    fun addRepo(url: String) = viewModelScope.launch { host.addRepo(url.trim()); loadOffers() }

    fun removeRepo(url: String) = viewModelScope.launch { host.removeRepo(url); loadOffers() }
}
