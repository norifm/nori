package dev.nori.music.plugins

import android.content.Context
import android.net.nsd.NsdManager
import android.net.wifi.WifiManager
import dev.nori.music.ffi.plugins.Cast
import dev.nori.music.ffi.plugins.CastKind
import dev.nori.music.ffi.plugins.Plugins
import dev.nori.music.ffi.plugins.Service
import dev.nori.music.ffi.plugins.TxtEntry
import dev.nori.music.net.NsdBrowse

/**
 * What Android holds for plugins on the LAN, only while the plugin host asks (crates/plugins lan.rs):
 * NsdManager browsing a kind, the multicast lock while a plugin's socket is in a group, and the Wi-Fi
 * lock while a sink cast streams nori's audio from the phone: for the whole cast, or for a deep sink
 * only while a burst is sent ([streaming]), so the radio sleeps between bursts.
 */
internal class LanGlue(context: Context, private val plugins: () -> Plugins) {
    private val app = context.applicationContext
    private val nsd by lazy { app.getSystemService(NsdManager::class.java) }
    private val wifi by lazy { app.getSystemService(WifiManager::class.java) }
    private val browsing = mutableMapOf<String, NsdBrowse>()
    private val multicast by lazy { wifi.createMulticastLock("nori plugins").apply { setReferenceCounted(false) } }
    @Suppress("DEPRECATION")
    private val castLock by lazy { wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "nori cast").apply { setReferenceCounted(false) } }
    /** The cast running, and whether a deep sink's burst is being sent: what [holdCast] follows. */
    private var cast: Cast? = null
    private var sending = false

    @Synchronized
    fun browse(kind: String, on: Boolean) {
        browsing.remove(kind)?.stop()
        if (on) browsing[kind] = NsdBrowse(nsd, kind, { f ->
            plugins().discovered(kind, Service(f.name, f.hosts.first(), f.port.toUShort(), f.txt.map { (k, v) -> TxtEntry(k, v) }))
        }, {})
    }

    @Synchronized
    fun multicast(on: Boolean) {
        if (on == multicast.isHeld) return
        if (on) multicast.acquire() else multicast.release()
    }

    @Synchronized
    fun cast(cast: Cast?) {
        this.cast = cast
        if (cast == null) sending = false
        holdCast()
    }

    /** A deep sink's burst is being sent ([on]) or all sent. */
    @Synchronized
    fun streaming(on: Boolean) {
        sending = on
        holdCast()
    }

    /** The Wi-Fi lock is held for a sink cast: the whole of it, or a deep sink's bursts alone. */
    private fun holdCast() {
        val c = cast
        val hold = c?.kind == CastKind.SINK && (!c.bursts || sending)
        if (hold == castLock.isHeld) return
        if (hold) castLock.acquire() else castLock.release()
    }
}
