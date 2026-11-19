package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.Cadence
import dev.nori.music.ffi.PayIn
import dev.nori.music.ffi.Payment
import dev.nori.music.ffi.Stamp
import dev.nori.music.ffi.SupportChoice
import dev.nori.music.ffi.SupportCurrency
import dev.nori.music.ffi.SupportPick
import dev.nori.music.ffi.SupportService
import dev.nori.music.net.lifted
import dev.nori.music.net.said
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

/** What paying is doing: nothing, asking the service, or a payment for the screen to take. */
sealed interface Paying {
    data object Idle : Paying
    data object Starting : Paying
    data class Take(val payment: Payment, val currency: String) : Paying
    data class Failed(val message: String, val otherCurrency: Boolean) : Paying
}

/** The Support page: the stamp book (the core's `support_stamps`) and paying (the core's support service calls). */
class SupportViewModel(app: Application) : NoriViewModel(app) {
    /** The build's support service; none, and there is no page. */
    val service: SupportService? = dev.nori.music.ffi.supportService()

    private val currencies: List<SupportCurrency> = dev.nori.music.ffi.supportCurrencies()
    private val southern = dev.nori.music.ffi.look.southern(java.util.TimeZone.getDefault().id)

    val stamps: StateFlow<List<Stamp>> = nori.settings.prefs
        .map { dev.nori.music.ffi.supportStamps(it.support.paid, southern) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /** The season a payment made now is stamped in: the kanji the book's empty place shows. */
    val nextSeason: dev.nori.music.ffi.look.Season =
        dev.nori.music.ffi.supportStamps(listOf(System.currentTimeMillis() / 1000), southern).first().season

    val monthly: StateFlow<Boolean> = nori.settings.prefs.map { it.support.monthly }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    private val first = dev.nori.music.ffi.supportFirstChoice(java.util.Locale.getDefault().country)
    private val currency = MutableStateFlow(first.currency.code)
    private val cadence = MutableStateFlow(first.cadence)
    private val pick = MutableStateFlow(first.pick)
    private val other = MutableStateFlow(first.other)

    /** What is chosen and what it comes to (the core's `support_choice`). */
    val choice: StateFlow<SupportChoice> = combine(currency, cadence, pick, other) { c, k, p, o -> dev.nori.music.ffi.supportChoice(c, k, p, o) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, first)

    val allCurrencies: List<SupportCurrency> get() = currencies

    private val _paying = MutableStateFlow<Paying>(Paying.Idle)
    val paying: StateFlow<Paying> = _paying

    fun pickCurrency(c: SupportCurrency) { currency.value = c.code }
    fun pickCadence(k: Cadence) { cadence.value = k }
    fun pickOffered(i: Int) { pick.value = SupportPick.Offered(i.toUInt()) }
    fun pickOther() { pick.value = SupportPick.Other }
    fun type(text: String) { other.value = text }

    /** Asks the service what this app has paid: when the page opens and after a payment. */
    fun refresh() {
        viewModelScope.launch {
            runCatching { lifted { nori.client.supportRefresh() } }.onSuccess { c -> c?.let(nori.settings::took) }
        }
    }

    /** Starts paying what is chosen, in the app's payment sheet. */
    fun pay() {
        val c = choice.value
        val amount = c.amount ?: return
        if (_paying.value == Paying.Starting) return
        _paying.value = Paying.Starting
        viewModelScope.launch {
            _paying.value = runCatching { lifted { nori.client.supportStart(c.currency.code, amount, c.cadence, PayIn.APP, southern) } }.fold(
                { s -> s.change?.let(nori.settings::took); Paying.Take(s.payment, c.currency.code) },
                { e -> Paying.Failed(e.said ?: e.javaClass.simpleName, (e as? dev.nori.music.net.HttpStatusException)?.status == 409) },
            )
        }
    }

    /** The sheet closed: paid (a new seal), closed unpaid, or failed saying [failure]. */
    fun taken(paid: Boolean, failure: String? = null) {
        _paying.value = if (failure != null) Paying.Failed(failure, false) else Paying.Idle
        if (paid) refresh()
    }

    /** The page where the monthly payment is changed or stopped, for [open]. */
    fun manage(open: (String) -> Unit) {
        viewModelScope.launch {
            runCatching { lifted { nori.client.supportManage() } }
                .onSuccess(open)
                .onFailure { e -> _paying.value = Paying.Failed(e.said ?: e.javaClass.simpleName, false) }
        }
    }
}
