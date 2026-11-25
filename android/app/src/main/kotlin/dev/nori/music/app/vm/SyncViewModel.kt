package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.sync.Came
import dev.nori.music.ffi.sync.Confirmation
import dev.nori.music.ffi.sync.MergeOffer
import dev.nori.music.ffi.sync.NearbyDevice
import dev.nori.music.ffi.sync.NeededPlugin
import dev.nori.music.ffi.sync.PairingRefused
import dev.nori.music.ffi.sync.PairingView
import dev.nori.music.ffi.sync.ResetNotice
import dev.nori.music.ffi.sync.Seen
import dev.nori.music.ffi.sync.SyncEvent
import dev.nori.music.ffi.sync.SyncState
import dev.nori.music.ffi.sync.Syncing
import dev.nori.music.remote.Remotes
import dev.nori.music.settings.label
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** Where this device's sync stands, as the core says, read off the main thread. */
data class SyncView(
    /** The core answered: before, nothing here is known. */
    val read: Boolean = false,
    /** The `sync` setting. */
    val on: Boolean = true,
    /** The `syncServer` setting; empty for nori's own. */
    val server: String = "",
    val state: SyncState = SyncState.Idle,
    val pairing: PairingView = PairingView.Idle,
    /** The group's other devices, in [deviceOrder]. */
    val devices: List<Seen> = emptyList(),
    val keepsCopy: Boolean = false,
    /** Saved profiles that may be one account. */
    val offers: List<MergeOffer> = emptyList(),
    /** The saved profiles that sync: removing one removes it from every paired device. */
    val synced: Set<String> = emptySet(),
    /** The plugins synced profiles play from that this phone lacks (the core's `needed_plugins`). */
    val needed: List<NeededPlugin> = emptyList(),
    /** Install is under way. */
    val installing: Boolean = false,
) {
    /** In a group, as far as this device can tell: sync runs only for a group (or a pairing), and only while on. */
    val paired: Boolean get() = on && state != SyncState.Idle
}

/** Which side of a pairing this device is on: the new device showing its code, or the one taking another's. */
enum class PairRole { SHOWS, TAKES }

/** What the pairing screen shows now. */
sealed interface PairStep {
    data object Idle : PairStep
    data object Starting : PairStep
    /** The code to show; [note] is a refusal the new device goes on waiting after. */
    data class Code(val link: String, val words: List<String>, val note: PairingRefused? = null) : PairStep
    /** The code shown here ran out: a new one is a tap away. */
    data object RanOut : PairStep
    data class Confirm(val confirmation: Confirmation) : PairStep
    /** This device joined the group. */
    data object Paired : PairStep
    /** The key went to the new device. */
    data object Sent : PairStep
    data class Refused(val reason: PairingRefused) : PairStep
}

/**
 * The pairing screen's step for the core's [view], on [role]'s side, the code shown last being [shown]:
 * the new device keeps showing its code after a refusal it waits on, and its code running out is
 * worded as that, with a new code offered.
 */
fun pairStep(view: PairingView, role: PairRole?, shown: PairingView.Code?): PairStep = when (view) {
    PairingView.Idle -> PairStep.Idle
    PairingView.Starting -> PairStep.Starting
    is PairingView.Code -> PairStep.Code(view.link, view.words)
    is PairingView.Confirm -> PairStep.Confirm(view.confirmation)
    PairingView.Paired -> PairStep.Paired
    PairingView.Sent -> PairStep.Sent
    is PairingView.Refused -> when {
        role != PairRole.SHOWS -> PairStep.Refused(view.reason)
        view.reason == PairingRefused.Expired -> PairStep.RanOut
        shown != null && (view.reason == PairingRefused.Tampered || view.reason == PairingRefused.Spent) -> PairStep.Code(shown.link, shown.words, view.reason)
        else -> PairStep.Refused(view.reason)
    }
}

/** The side of a pairing the core has under way, which a screen opened later takes up; null for none. */
fun pairRole(view: PairingView): PairRole? = when (view) {
    is PairingView.Code -> PairRole.SHOWS
    is PairingView.Confirm -> PairRole.TAKES
    else -> null
}

/** Whether the pairing screen shows this device's code at once: only a device in no group, which has nothing to give. */
fun showsCodeAtOnce(view: SyncView): Boolean = view.on && !view.paired

/** Whether this device can take another's code: it has profiles to give. */
fun takesCode(profiles: Int): Boolean = profiles > 0

/** The group's other devices as the list shows them: those in the room first, then the most recently seen. */
fun deviceOrder(devices: List<Seen>): List<Seen> = devices.sortedWith(compareByDescending<Seen> { it.online }.thenByDescending { it.lastMs })

/** Of two profiles that may be one, the one a merge keeps: the one in use, else the first. */
fun mergeKeeps(offer: MergeOffer, active: String): Pair<String, String> = if (offer.b == active) offer.b to offer.a else offer.a to offer.b

/** Whether turning Sync this profile off asks about the other devices: only for a profile that syncs. */
fun syncOffAsks(profile: String, view: SyncView): Boolean = view.paired && profile in view.synced

/** Whether Install has anything to fetch: a plugin installed from a file on the other device has no repo. */
fun installOffered(needed: List<NeededPlugin>): Boolean = needed.any { it.repo != null }

/** The events the app tells as its usual notice; a refusal shows where sync stands instead. */
fun told(event: SyncEvent): Boolean = event !is SyncEvent.Refused

/**
 * Pairing and profile sync for the screens: Your devices, Pair a device, a profile's Sync this profile and
 * the merge offers. Every call reaches the core off the main thread (`Sync.call`); what it says comes back
 * as [view], read again whenever sync says something changed or the profiles did.
 */
class SyncViewModel(app: Application) : NoriViewModel(app) {
    private val sync = nori.sync

    /** This device's name, as pairing gives it to the others. */
    val name: String = Remotes.deviceName(app)

    private val _view = MutableStateFlow(SyncView())
    val view: StateFlow<SyncView> = _view.asStateFlow()

    private val _role = MutableStateFlow<PairRole?>(null)

    /** The side of the pairing this screen started, if it started one. */
    val role: StateFlow<PairRole?> = _role.asStateFlow()
    private var shown: PairingView.Code? = null
    private val _step = MutableStateFlow<PairStep>(PairStep.Idle)
    val step: StateFlow<PairStep> = _step.asStateFlow()
    private var opened = false

    /** Devices waiting nearby with their pairing screen open. */
    val nearby: StateFlow<List<NearbyDevice>> = sync.nearby

    init {
        val profiles = nori.settings.prefs.map { it.servers }.distinctUntilChanged()
        val setting = nori.settings.prefs.map { it.sync to it.syncServer.orEmpty() }.distinctUntilChanged()
        // A plugin installed or removed changes the plugins the profiles need.
        viewModelScope.launch { combine(sync.changes, profiles, setting, nori.plugins.cards) { _, _, _, _ -> }.collect { read() } }
    }

    /** Reads where sync stands again: on a change, and when a screen comes back. */
    fun read() {
        viewModelScope.launch {
            val p = nori.settings.value
            val next = sync.call { s ->
                SyncView(
                    read = true, on = p.sync, server = p.syncServer.orEmpty(), state = s.state(), pairing = s.pairing(), devices = deviceOrder(s.devices()),
                    keepsCopy = s.keepsCopy(), offers = s.offers(), synced = p.servers.map { it.id }.filter(s::synced).toSet(),
                    needed = s.neededPlugins(), installing = _view.value.installing,
                )
            }
            _view.value = next
            (next.pairing as? PairingView.Code)?.let { shown = it }
            _step.value = pairStep(next.pairing, _role.value, shown)
        }
    }

    /** Install: the plugins synced profiles need, each from its repo (the core's `Kinds::install_needed`). */
    fun installNeeded() {
        val needed = view.value.needed
        if (view.value.installing || !installOffered(needed)) return
        _view.value = _view.value.copy(installing = true)
        viewModelScope.launch {
            nori.plugins.kinds.installNeeded(needed)
            _view.value = _view.value.copy(installing = false)
            read()
        }
    }

    /**
     * The pairing screen opened: it takes up a pairing under way or a code opened from outside the app,
     * or a device in no group shows its code at once, once the core has said where it stands.
     */
    fun opened() {
        if (opened) return
        opened = true
        viewModelScope.launch {
            val v = view.first { it.read }
            sync.offered.value?.let { link ->
                sync.offered.value = null
                take(link, Came.LINK)
                return@launch
            }
            if (_role.value != null) return@launch
            _role.value = pairRole(v.pairing)
            if (_role.value == null && showsCodeAtOnce(v)) showCode() else read()
        }
    }

    /** The `sync` setting: off, nothing connects; on, sync starts again if this device is paired. */
    fun setOn(on: Boolean) {
        nori.settings.update { it.copy(sync = on) }
        sync.settingsChanged()
    }

    /** The `syncServer` setting: blank for nori's own. */
    fun setServer(address: String) {
        val kept = address.trim().ifEmpty { null }
        if (kept == nori.settings.value.syncServer) return
        nori.settings.update { it.copy(syncServer = kept) }
        sync.settingsChanged()
    }

    fun keepCopy(on: Boolean) = act { it.keepCopy(on) }

    /** Shows this device's code, to join a group. */
    fun showCode() {
        _role.value = PairRole.SHOWS
        shown = null
        act { it.pairShow(name) }
    }

    /** Takes another device's code: scanned, pasted, or a nearby device's. A code that does not read is refused at once. */
    fun take(text: String, came: Came) {
        _role.value = PairRole.TAKES
        act { it.pairTake(text.trim(), came) }
    }

    fun take(waiting: NearbyDevice) = take(waiting.link, Came.NEARBY)

    /** The pairing screen opened or closed: while open, devices waiting nearby are listed. */
    fun look(on: Boolean) = sync.look(on)

    /** Yes to the confirmation; [keepCopy] answers the first pairing's question. */
    fun confirm(keepCopy: Boolean) = act { it.pairConfirm(name, keepCopy) }

    /** Stops the pairing under way, or forgets how the last one ended. */
    fun cancelPairing() {
        _role.value = null
        shown = null
        act { it.pairCancel() }
    }

    /** What Reset pairing does, with this device's profiles named, for its confirmation; null while not paired. */
    suspend fun resetNotice(): Pair<ResetNotice, List<String>>? {
        val servers = nori.settings.value.servers
        return sync.call { it.resetNotice() }?.let { n -> n to n.signedIn.mapNotNull { id -> servers.firstOrNull { it.id == id }?.label } }
    }

    fun reset() = act { it.reset() }

    /** Merges the two profiles of [offer] on every device, keeping the one in use. */
    fun merge(offer: MergeOffer) {
        val (keep, drop) = mergeKeeps(offer, nori.settings.value.activeServerId)
        act { it.merge(keep, drop) }
    }

    fun keepApart(offer: MergeOffer) = act { it.keepApart(offer.a, offer.b) }

    /** Sync this profile on or off; off with [elsewhere], the other devices remove it too. */
    fun syncProfile(profile: SavedServer, on: Boolean, elsewhere: Boolean = false) {
        viewModelScope.launch {
            if (!on) sync.call { it.syncOff(profile.id, elsewhere) }
            nori.settings.value.servers.firstOrNull { it.id == profile.id }?.let { nori.updateServer(it.copy(syncOff = !on)) }
        }
    }

    private fun act(call: (Syncing) -> Unit) {
        viewModelScope.launch {
            sync.call(call)
            read()
        }
    }

    /** A pairing this screen started ends with it. */
    override fun onCleared() {
        if (_role.value != null) sync.post { it.pairCancel() }
    }
}
