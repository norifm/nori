package dev.nori.music.remote

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import dev.nori.music.ffi.Announcement
import dev.nori.music.ffi.Discovery
import dev.nori.music.ffi.Param
import dev.nori.music.ffi.Remote

/**
 * The core's [Discovery] over Android's NsdManager: the system's mDNS daemon announces and looks, so the app
 * holds no multicast lock and wakes only for a door found or lost. Announcing runs while the device is
 * controllable, looking while a picker or jam screen is open (the core says when).
 */
class NsdDiscovery(context: Context, private val remote: () -> Remote?) : Discovery {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private var registered: NsdManager.RegistrationListener? = null
    private var browsing: dev.nori.music.net.NsdBrowse? = null

    @Synchronized
    override fun announce(door: Announcement?) {
        registered?.let { runCatching { nsd.unregisterService(it) } }
        registered = null
        door ?: return
        val info = NsdServiceInfo().apply {
            serviceName = door.name
            serviceType = SERVICE
            port = door.port.toInt()
            door.txt.forEach { setAttribute(it.key, it.value) }
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {}
            override fun onRegistrationFailed(info: NsdServiceInfo, error: Int) = dev.nori.music.NoriLog.w("nsd: not announced ($error)")
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
            remote()?.lanFound(f.name, f.host, f.port.toUShort(), f.txt.map { (k, v) -> Param(k, v) })
        }, { name -> remote()?.lanLost(name) }) else null
    }

    private companion object {
        /** nori-remote's `lan::SERVICE`, as NsdManager writes a type. */
        const val SERVICE = "_nori._tcp"
    }
}
