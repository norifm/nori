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
    val shown = host.shown
    val notice = host.notice

    private val _offers = MutableStateFlow(OffersUi())
    val offers: StateFlow<OffersUi> = _offers

    private val _said = MutableStateFlow<Pair<Boolean, String>?>(null)
    /** The last install's outcome in words; true when it is a refusal. */
    val said: StateFlow<Pair<Boolean, String>?> = _said

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

    /** Copies the picked package to the cache and installs it from there. */
    fun installFile(uri: Uri) = viewModelScope.launch {
        val file = java.io.File(getApplication<Application>().cacheDir, "picked.noriplugin")
        val copied = withContext(Dispatchers.IO) {
            runCatching { getApplication<Application>().contentResolver.openInputStream(uri)?.use { i -> file.outputStream().use { i.copyTo(it) } } }.getOrNull() != null
        }
        if (!copied) { _said.value = true to res.getString(R.string.plugins_refused_unreadable); return@launch }
        installing { host.install(file.path) }
        withContext(Dispatchers.IO) { file.delete() }
    }

    /** Installs [offer]'s newest version. */
    fun installOffer(offer: Offer) = offer.versions.firstOrNull()?.let(::installVersion)

    /** Installs [version], newer or older than the one here. */
    fun installVersion(version: Version) = viewModelScope.launch {
        installing { host.installVersion(version) }
        loadOffers()
    }

    fun hold(id: String, held: Boolean) = viewModelScope.launch { host.hold(id, held) }

    private suspend fun installing(install: suspend () -> String) {
        _said.value = try {
            val id = install()
            false to res.getString(R.string.plugins_installed_off, cards.value.firstOrNull { it.id == id }?.name ?: id)
        } catch (e: Refusal) {
            true to refusalWords(e)
        } catch (e: Exception) {
            dev.nori.music.NoriLog.w("plugin install: ${e.message}")
            true to res.getString(R.string.plugins_refused_other)
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

    fun addRepo(url: String) = viewModelScope.launch { host.addRepo(url.trim()); loadOffers() }

    fun removeRepo(url: String) = viewModelScope.launch { host.removeRepo(url); loadOffers() }
}
