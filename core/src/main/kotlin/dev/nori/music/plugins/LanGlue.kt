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
 * lock while a sink cast streams nori's audio from the phone.
 */
internal class LanGlue(context: Context, private val plugins: () -> Plugins) {
    private val app = context.applicationContext
    private val nsd by lazy { app.getSystemService(NsdManager::class.java) }
    private val wifi by lazy { app.getSystemService(WifiManager::class.java) }
    private val browsing = mutableMapOf<String, NsdBrowse>()
    private var multicast: WifiManager.MulticastLock? = null
    private var streaming: WifiManager.WifiLock? = null

    @Synchronized
    fun browse(kind: String, on: Boolean) {
        browsing.remove(kind)?.stop()
        if (on) browsing[kind] = NsdBrowse(nsd, kind, { f ->
            plugins().discovered(kind, Service(f.name, f.host, f.port.toUShort(), f.txt.map { (k, v) -> TxtEntry(k, v) }))
        }, {})
    }

    @Synchronized
    fun multicast(on: Boolean) {
        multicast?.takeIf { it.isHeld }?.release()
        multicast = if (on) wifi.createMulticastLock("nori plugins").apply { setReferenceCounted(false); acquire() } else null
    }

    @Synchronized
    fun cast(cast: Cast?) {
        val sink = cast?.kind == CastKind.SINK
        if (sink == (streaming != null)) return
        streaming?.takeIf { it.isHeld }?.release()
        @Suppress("DEPRECATION")
        streaming = if (sink) wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "nori cast").apply { setReferenceCounted(false); acquire() } else null
    }
}
