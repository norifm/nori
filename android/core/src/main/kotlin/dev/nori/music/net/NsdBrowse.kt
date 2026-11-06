package dev.nori.music.net

import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo

/** A service NsdManager found and resolved: its name, address, port and TXT. */
class Found(val name: String, val host: String, val port: Int, val txt: List<Pair<String, String>>)

/**
 * Browsing one mDNS service [type] through Android's NsdManager until [stop]: each service found is
 * resolved and handed to [found], each one gone to [lost]. The system's daemon does the looking, so the
 * app holds no multicast lock for it.
 */
class NsdBrowse(private val nsd: NsdManager, private val type: String, private val found: (Found) -> Unit, private val lost: (String) -> Unit) {
    /** Before Android 14 one resolve runs at a time; the others wait here. */
    private val resolving = ArrayDeque<NsdServiceInfo>()
    private var busy = false

    private val listener = object : NsdManager.DiscoveryListener {
        override fun onDiscoveryStarted(type: String) {}
        override fun onDiscoveryStopped(type: String) {}
        override fun onStartDiscoveryFailed(type: String, error: Int) = dev.nori.music.NoriLog.w("nsd: no discovery of $type ($error)")
        override fun onStopDiscoveryFailed(type: String, error: Int) {}
        override fun onServiceFound(info: NsdServiceInfo) = resolve(info)
        override fun onServiceLost(info: NsdServiceInfo) = lost(info.serviceName)
    }

    init {
        runCatching { nsd.discoverServices(type, NsdManager.PROTOCOL_DNS_SD, listener) }
    }

    fun stop() {
        runCatching { nsd.stopServiceDiscovery(listener) }
    }

    @Synchronized
    private fun resolve(info: NsdServiceInfo) {
        if (busy) { resolving.addLast(info); return }
        busy = true
        @Suppress("DEPRECATION")
        nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, error: Int) = next()
            override fun onServiceResolved(info: NsdServiceInfo) {
                @Suppress("DEPRECATION")
                val host = info.host?.hostAddress
                if (host != null) found(Found(info.serviceName, host, info.port, info.attributes.map { (k, v) -> k to v?.toString(Charsets.UTF_8).orEmpty() }))
                next()
            }
        })
    }

    @Synchronized
    private fun next() {
        busy = false
        resolving.removeFirstOrNull()?.let(::resolve)
    }
}
