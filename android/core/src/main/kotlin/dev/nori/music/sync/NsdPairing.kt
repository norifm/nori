package dev.nori.music.sync

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import dev.nori.music.ffi.sync.Nearby
import dev.nori.music.ffi.sync.NearbyAnnouncement
import dev.nori.music.ffi.sync.NearbyTxt
import dev.nori.music.ffi.sync.Syncing

/**
 * The core's [Nearby] over Android's NsdManager: a device waiting to be paired is announced while its
 * pairing screen shows its code, and looked for while a pairing screen is open (the core says when). The
 * system's daemon does both, so the app holds no multicast lock. Refused the local network, nothing is
 * found, and a code travels by camera or link.
 */
class NsdPairing(context: Context, private val sync: () -> Syncing) : Nearby {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private var registered: NsdManager.RegistrationListener? = null
    private var browsing: dev.nori.music.net.NsdBrowse? = null

    @Synchronized
    override fun announce(waiting: NearbyAnnouncement?) {
        registered?.let { runCatching { nsd.unregisterService(it) } }
        registered = null
        waiting ?: return
        val info = NsdServiceInfo().apply {
            serviceName = waiting.name
            serviceType = SERVICE
            // Nothing listens: the announcement carries the code, and a port is only what DNS-SD asks.
            port = DISCARD_PORT
            waiting.txt.forEach { setAttribute(it.key, it.value) }
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {}
            override fun onRegistrationFailed(info: NsdServiceInfo, error: Int) = dev.nori.music.NoriLog.w("nsd: pairing not announced ($error)")
            override fun onServiceUnregistered(info: NsdServiceInfo) {}
            override fun onUnregistrationFailed(info: NsdServiceInfo, error: Int) {}
        }
        registered = listener
        runCatching { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener) }
    }

    @Synchronized
    override fun browse(on: Boolean) {
        browsing?.stop()
        browsing = if (on) dev.nori.music.net.NsdBrowse(nsd, SERVICE, { f ->
            sync().nearbyFound(f.name, f.txt.map { (k, v) -> NearbyTxt(k, v) })
        }, { name -> sync().nearbyLost(name) }) else null
    }

    private companion object {
        /** nori-sync's `nearby::SERVICE`, as NsdManager writes a type. */
        const val SERVICE = "_nori-pair._tcp"
        const val DISCARD_PORT = 9
    }
}
