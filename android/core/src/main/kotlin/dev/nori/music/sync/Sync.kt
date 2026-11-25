package dev.nori.music.sync

import android.content.Context
import android.os.Handler
import android.os.Looper
import dev.nori.music.Nori
import dev.nori.music.NoriLog
import dev.nori.music.ffi.sync.Ended
import dev.nori.music.ffi.sync.Here
import dev.nori.music.ffi.sync.NearbyDevice
import dev.nori.music.ffi.sync.NeededPlugin
import dev.nori.music.ffi.sync.ProfilesSynced
import dev.nori.music.ffi.sync.SyncClient
import dev.nori.music.ffi.sync.SyncEvent
import dev.nori.music.ffi.sync.SyncState
import dev.nori.music.ffi.sync.Sockets
import dev.nori.music.ffi.sync.Syncing
import dev.nori.music.playback.EnginePlayer
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * The sync server's WebSockets over the app's one OkHttp pool (Http.sockets): Kotlin opens, reads and
 * writes; the core decides everything else. OkHttp sends no WebSocket ping on demand, only on a timer of
 * its own, so a keep-alive is the protocol's text `ping`.
 */
private class OkSockets(private val client: () -> OkHttpClient) : Sockets {
    @Volatile var sync: Syncing? = null
    private val open = ConcurrentHashMap<ULong, WebSocket>()

    override fun open(id: ULong, url: String) {
        open[id] = client().newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (open.containsKey(id)) sync?.frame(id, bytes.toByteArray())
            }

            // The text keep-alive's answer, `pong`.
            override fun onMessage(webSocket: WebSocket, text: String) {}

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                ended(id, Ended.Closed(code.toUShort()))
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val status = response?.code?.takeIf { it != 101 }
                ended(id, if (status != null) Ended.Status(status.toUShort()) else Ended.Dropped)
            }
        })
    }

    /** Told once, unless sync closed it. */
    private fun ended(id: ULong, how: Ended) {
        if (open.remove(id) != null) sync?.ended(id, how)
    }

    override fun send(id: ULong, frame: ByteArray) {
        open[id]?.send(frame.toByteString())
    }

    override fun ping(id: ULong) {
        open[id]?.send("ping")
    }

    override fun close(id: ULong) {
        open.remove(id)?.close(1000, null)
    }
}

/**
 * Pairing and profile sync (the core's `Syncing`): nothing runs unless this phone is paired with sync on.
 * It hears whether the app is shown and playing, and lends sync's keep-alive the engine's wakes, so it
 * never wakes the phone by itself. Built off the main thread ([Nori.warmUp]).
 */
class Sync(private val nori: Nori, context: Context) : SyncClient {
    private val sockets = OkSockets { nori.http.sockets }
    private val pairing = NsdPairing(context) { syncing }

    /** Made on [worker]: it reads whether this phone is paired. */
    val syncing: Syncing by lazy {
        Syncing.onPhone(nori.settings.core, sockets, this).also {
            sockets.sync = it
            it.nearbyWith(pairing)
        }
    }

    private val main = Handler(Looper.getMainLooper())
    /** Calls into the core leave the main thread here, one at a time. */
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "nori-sync-calls") }
    private val calls = worker.asCoroutineDispatcher()

    /** [ask] of the core's sync, on the thread its calls leave the main thread on. */
    suspend fun <T> call(ask: (Syncing) -> T): T = withContext(calls) { ask(syncing) }

    /** [act] on the core's sync there, whatever becomes of the caller (a screen left). */
    fun post(act: (Syncing) -> Unit) = worker.execute { act(syncing) }

    private val _changes = MutableStateFlow(0L)

    private val _paired = MutableStateFlow(false)

    /** Whether this phone is paired with sync on: the app then asks to reach the local network ([LocalNetwork]). */
    val paired: StateFlow<Boolean> = _paired.asStateFlow()

    private val _nearby = MutableStateFlow<List<NearbyDevice>>(emptyList())

    /** The devices waiting to be paired that the pairing screen lists, while it looks ([look]). */
    val nearby: StateFlow<List<NearbyDevice>> = _nearby.asStateFlow()

    /** Bumped when where sync stands, the pairing, the group's devices or the nearby list changed; screens read them again. */
    val changes: StateFlow<Long> = _changes.asStateFlow()

    private val _events = MutableSharedFlow<SyncEvent>(extraBufferCapacity = 16)

    /** What happened in the group, for the screens to word. */
    val events: SharedFlow<SyncEvent> = _events

    private val _installs = MutableStateFlow<List<NeededPlugin>>(emptyList())

    /** Plugins the synced profiles need, for the plugin screens to install from their repos. */
    val installs: StateFlow<List<NeededPlugin>> = _installs.asStateFlow()

    /** The playback service's engine while it runs, which sync's keep-alive rides on while sync runs. */
    private var engine: EnginePlayer? = null
    private var riding = false

    /** Starts sync if this phone is paired with sync on, off the main thread. */
    fun start() = worker.execute { syncing; heard() }

    /** A pairing screen opened or closed: while open it lists the devices nearby waiting to be paired. */
    fun look(on: Boolean) = worker.execute { syncing.pairLook(on); heard() }

    /** The app came on screen or left it. */
    fun shown(on: Boolean) = worker.execute { syncing.foreground(on) }

    /** Music started or stopped. */
    fun playing(on: Boolean) = worker.execute { syncing.playing(on) }

    /** The sync or sync server setting changed. */
    fun settingsChanged() = worker.execute { syncing.settingsChanged() }

    /** The playback service's engine came, or went (null). */
    fun engine(e: EnginePlayer?) = worker.execute {
        synchronized(this) {
            engine = e
            riding = false
        }
        ride()
    }

    /** Lends the engine's wakes while sync runs, and takes them back when it stops. */
    @Synchronized
    private fun ride() {
        val e = engine ?: return
        val runs = syncing.state() != SyncState.Idle
        if (runs == riding) return
        riding = runs
        e.syncWakes(if (runs) syncing else null)
    }

    override fun here(): Here = nori.plugins.kinds.syncHere()

    override fun profiles(synced: ProfilesSynced) = main.post { nori.profilesSynced(synced) }.let {}

    override fun install(plugins: List<NeededPlugin>) {
        NoriLog.i("sync: the profiles need plugins ${plugins.joinToString { it.id }}")
        _installs.value = plugins
    }

    override fun event(event: SyncEvent) {
        NoriLog.i("sync: $event")
        _events.tryEmit(event)
    }

    override fun changed() {
        ride()
        heard()
        _changes.update { it + 1 }
    }

    /** Reads again what the flows show. */
    private fun heard() {
        _paired.value = syncing.state() != SyncState.Idle
        _nearby.value = syncing.nearby()
    }
}
