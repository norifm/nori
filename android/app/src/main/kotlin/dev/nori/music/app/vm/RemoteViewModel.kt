package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.JamView
import dev.nori.music.ffi.RelaySupport
import dev.nori.music.ffi.RemoteDevice
import dev.nori.music.ffi.remote.Op
import dev.nori.music.ffi.remote.QrCode
import dev.nori.music.settings.server
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The devices sheet and the jam (its header in the queue, the invite, the people, a guest's requests) over the
 * core's remote control (Remotes). Everything is the core's: the jam is read again whenever the core says
 * something changed, and the devices only while a sheet is watching.
 */
class RemoteViewModel(app: Application) : NoriViewModel(app) {
    private val remotes = nori.remotes

    private val _devices = MutableStateFlow<List<RemoteDevice>>(emptyList())
    /** The account's other devices with their states, nearby ones first. */
    val devices: StateFlow<List<RemoteDevice>> = _devices.asStateFlow()

    private val _names = MutableStateFlow<List<String>>(emptyList())
    /** [devices]' names as the picker lists them, told apart where two share one (the core's `device_names`). */
    val names: StateFlow<List<String>> = _names.asStateFlow()

    private val _outputs = MutableStateFlow<List<dev.nori.music.ffi.ReachedOutput>>(emptyList())
    /** What other devices can cast to and this phone cannot, each once. */
    val outputs: StateFlow<List<dev.nori.music.ffi.ReachedOutput>> = _outputs.asStateFlow()

    private val _pairingLink = MutableStateFlow<String?>(null)
    /** This phone's pairing code; null where this profile doesn't pair, or remote control is off. */
    val pairingLink: StateFlow<String?> = _pairingLink.asStateFlow()
    private val _paired = MutableStateFlow<List<dev.nori.music.ffi.PairedDevice>>(emptyList())
    /** The devices paired with this phone, the nearby ones first. */
    val paired: StateFlow<List<dev.nori.music.ffi.PairedDevice>> = _paired.asStateFlow()
    private val _pairRequest = MutableStateFlow<String?>(null)
    /** A pairing code opened from outside the app (a link, the system camera), waiting for a yes. */
    val pairRequest: StateFlow<String?> = _pairRequest.asStateFlow()

    fun offerPairing(link: String) { _pairRequest.value = link }
    fun dismissPairing() { _pairRequest.value = null }

    /** Pairs with the device whose code [link] is; [then] gets the device paired, or why not. */
    fun pair(link: String, then: (Result<dev.nori.music.ffi.PairedDevice>) -> Unit) = viewModelScope.launch {
        val paired = runCatching { remotes.pair(link) }
        if (paired.isSuccess) refresh()
        then(paired)
    }

    /** Forgets paired device [id]. */
    fun forget(id: String) {
        _paired.value = _paired.value.filter { it.id != id }
        remotes.forget(id)
    }

    private val me = dev.nori.music.ffi.RemoteMe(dev.nori.music.remote.Remotes.deviceName(app), dev.nori.music.ffi.remote.DeviceKind.PHONE)
    private val kinds = app.resources.let { r ->
        dev.nori.music.ffi.KindWords(
            r.getString(dev.nori.music.app.R.string.devices_kind_phone), r.getString(dev.nori.music.app.R.string.devices_kind_desktop),
            r.getString(dev.nori.music.app.R.string.devices_kind_terminal), r.getString(dev.nori.music.app.R.string.devices_kind_guest),
        )
    }

    /** The jam this phone hosts or is a guest in. */
    val jam: StateFlow<JamView?> = remotes.jam

    /** The jam this phone was a guest in ended, by its host's name if seen; the phone's own profile is open again. */
    val ended = remotes.ended

    /** The device playing while it is not this phone (Remotes.mirror). */
    val mirror: StateFlow<dev.nori.music.ffi.Mirror?> = remotes.mirror
    private val _volumeKeys = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST)
    /** A volume key moved [mirror]'s volume. */
    val volumeKeys: SharedFlow<Unit> = _volumeKeys

    /**
     * A volume key pressed while the app is in front: [mirror]'s volume takes the step the core gives
     * (`mirror_volume_key`). False when nothing elsewhere has a volume to set, and the key is the system's.
     */
    fun volumeKey(up: Boolean): Boolean {
        val m = remotes.mirror.value ?: return false
        val level = dev.nori.music.ffi.mirrorVolumeKey(m, up) ?: return false
        dev.nori.music.ffi.mirrorOp(m, dev.nori.music.ffi.Press.Volume(level.toFloat() / 100f))?.let(remotes::command)
        _volumeKeys.tryEmit(Unit)
        return true
    }

    /** Where the music plays (the core's `destination`): what the devices list lights. */
    val destination: StateFlow<dev.nori.music.ffi.Destination> = combine(remotes.mirror, remotes.jam, nori.plugins.cast) { _, _, c -> remotes.destination(c) }
        .flowOn(Dispatchers.Default)
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), dev.nori.music.ffi.Destination.Here(false))

    /** Whether the server relays: jams and devices elsewhere only then. */
    val relay: StateFlow<RelaySupport> = remotes.relay

    /** Where starting a jam here stands (the core's `jam_start`): offered, being started, or hosted. */
    val jamStarting: StateFlow<dev.nori.music.ffi.JamStart> = remotes.jamStart

    /** Whether a jam can be started here; song, album and playlist menus offer "Start a jam" only then. */
    val canStartJam: StateFlow<Boolean> = remotes.jamStart.map { it == dev.nori.music.ffi.JamStart.OFFERED }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    /** The songs this guest asked for that the host has not decided on yet, by id. */
    val asked: StateFlow<Set<String>> = remotes.jam.map { j -> j?.takeIf { !it.hosting }?.let { v -> v.pending.filter { it.from == v.you }.map { it.song.id }.toSet() }.orEmpty() }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    private val _jamFailed = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    /** A jam asked for here did not start. */
    val jamFailed: SharedFlow<Unit> = _jamFailed

    private var watchers = 0

    init {
        remotes.refresh()
        viewModelScope.launch { remotes.changes.collect { if (watchers > 0) refresh() } }
    }

    private fun refresh() = remotes.ask({ r ->
        val link = r.pairingLink()
        Shown(r.outputs(), r.devices().let { d -> d to dev.nori.music.ffi.deviceNames(d, me, kinds) }, link, if (link != null) r.paired() else emptyList())
    }) { shown ->
        _outputs.value = shown.outputs
        _devices.value = shown.devices.first
        _names.value = shown.devices.second
        _pairingLink.value = shown.link
        _paired.value = shown.paired
    }

    /** What one read of the remote shows. */
    private class Shown(
        val outputs: List<dev.nori.music.ffi.ReachedOutput>,
        val devices: Pair<List<RemoteDevice>, List<String>>,
        val link: String?,
        val paired: List<dev.nori.music.ffi.PairedDevice>,
    )

    /** A sheet that shows devices is on screen (true) or gone (false): other devices are followed while it is. */
    fun watch(on: Boolean) {
        watchers = (watchers + if (on) 1 else -1).coerceAtLeast(0)
        remotes.watch(watchers > 0)
        if (on) refresh()
    }

    override fun onCleared() {
        if (watchers > 0) remotes.watch(false)
    }

    /** Moves the playback to [device], or to this phone (null). */
    fun pick(device: String?) = remotes.pick(device)

    /** The music goes to [output], through the device reaching it. */
    fun castOn(output: dev.nori.music.ffi.ReachedOutput) = remotes.castOn(output)

    /** Starts hosting a jam around what plays; a failure comes out of [jamFailed]. */
    fun jamStart() = viewModelScope.launch {
        if (runCatching { remotes.jamOpen() }.isFailure) _jamFailed.tryEmit(Unit)
    }

    fun jamEnd() = remotes.ask({ it.jamClose() })

    /** Plays the jam this phone is a guest in here, in step with its host, or only shows it. */
    fun listen(on: Boolean) = remotes.listen(on)

    /** Lets the hosted jam's guests listen along, or not. */
    fun jamAlong(on: Boolean) = remotes.jamAlong(on)
    fun jamRemoteSongs(on: Boolean) = remotes.jamRemoteSongs(on)

    /** A jam op from here: the host's own, or a guest's sent to the host. */
    fun jamAct(op: Op) = remotes.ask({ it.jamAct(op) })

    fun accept(request: ULong, yes: Boolean) = jamAct(Op.Decide(request, yes))

    /** Makes member [id] an admin, or a guest again. */
    fun promote(id: String, admin: Boolean) = jamAct(Op.Promote(id, admin))

    /** Sends member [id] out of the jam. */
    fun remove(id: String) = jamAct(Op.Kick(id))

    fun leave() = nori.leaveJam()

    /** The server's address when an invite to it works only on a home network (the core's `is_home_only`). */
    fun homeOnly(): String? = nori.settings.value.server?.url?.takeIf { dev.nori.music.ffi.remote.isHomeOnly(it) }

    /** The invite as a QR code, made off the main thread. */
    suspend fun qr(link: String): QrCode? = withContext(Dispatchers.Default) { dev.nori.music.ffi.remote.qrCode(link) }
}
