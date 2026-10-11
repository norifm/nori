package dev.nori.music.remote

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaRoute2Info
import android.media.MediaRoute2ProviderService
import android.media.MediaRouter2
import android.media.RouteDiscoveryPreference
import android.media.RoutingSessionInfo
import android.os.Build
import android.os.Bundle
import androidx.annotation.RequiresApi
import dev.nori.music.Nori
import dev.nori.music.core.R
import dev.nori.music.ffi.Mirror
import dev.nori.music.ffi.RemoteDevice
import dev.nori.music.ffi.remote.Op

/** The mirrored device and transfer choices in Android's output picker, while the media session is engaged. */
object RemoteRoute {
    internal data class Shown(val id: String, val name: String, val volume: Int?, val devices: List<Pair<String, String>>)
    /** The system creates the provider, so its current routes are kept for the next binding. */
    private var shown: Shown? = null
    /** The provider while the system has it bound: it is the system's to make, so it is reached through here. */
    private var provider: Provider? = null
    private var scan: MediaRouter2.RouteCallback? = null

    private const val FEATURE = "dev.nori.music.REMOTE"
    private const val HERE = "here"
    private const val SESSION = "mirrored"

    /** The routing controller id of the session, as the system knows it: the provider's id and the session's. */
    fun controllerId(context: Context): String? =
        if (Build.VERSION.SDK_INT >= 30) ComponentName(context, Provider::class.java).flattenToShortString() + ":" + SESSION else null

    /** Shows the device mirrored now and the account's other devices. Main thread. */
    fun show(context: Context, device: Mirror?, devices: List<RemoteDevice> = emptyList()) {
        if (Build.VERSION.SDK_INT < 30) return
        val next = device?.let { Shown(it.id, it.name, it.volume?.toInt(), devices.map { d -> d.id to d.name }) }
        if (next == shown) return
        shown = next
        provider?.show(next)
        if (next != null) {
            enable(context, true)
            startScan(context)
        } else {
            stopScan(context)
            enable(context, false)
        }
    }

    private fun enable(context: Context, on: Boolean) = context.packageManager.setComponentEnabledSetting(
        ComponentName(context, Provider::class.java),
        if (on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DEFAULT,
        PackageManager.DONT_KILL_APP,
    )

    /** A scan for this app's own route only: a self-scan-only provider is bound for no other app's. */
    @RequiresApi(30)
    private fun startScan(context: Context) {
        if (scan != null) return
        val callback = object : MediaRouter2.RouteCallback() {}
        scan = callback
        MediaRouter2.getInstance(context).registerRouteCallback(context.mainExecutor, callback, RouteDiscoveryPreference.Builder(listOf(FEATURE), true).build())
    }

    @RequiresApi(30)
    private fun stopScan(context: Context) {
        val callback = scan ?: return
        scan = null
        MediaRouter2.getInstance(context).unregisterRouteCallback(callback)
    }

    /** The provider the system binds (manifest: self-scan only, so only this app's scan binds it). */
    @RequiresApi(30)
    class Provider : MediaRoute2ProviderService() {
        private var session: RoutingSessionInfo? = null

        override fun onCreate() {
            super.onCreate()
            provider = this
        }

        override fun onDestroy() {
            if (provider === this) provider = null
            super.onDestroy()
        }

        override fun onDiscoveryPreferenceChanged(preference: RouteDiscoveryPreference) {
            // Bound for the scan: what is mirrored goes up.
            show(shown)
        }

        private fun nativeId(device: String) = "device-$device"

        private fun sessionInfo(device: Shown): RoutingSessionInfo {
            val selected = nativeId(device.id)
            val builder = RoutingSessionInfo.Builder(SESSION, packageName).addSelectedRoute(selected).setName(device.name)
                .addTransferableRoute(HERE)
            device.devices.forEach { (id, _) -> if (id != device.id) builder.addTransferableRoute(nativeId(id)) }
            device.volume?.let { builder.setVolumeHandling(MediaRoute2Info.PLAYBACK_VOLUME_VARIABLE).setVolumeMax(100).setVolume(it) }
            return builder.build()
        }

        private fun route(id: String, name: String, local: Boolean, selected: Boolean, volume: Int?): MediaRoute2Info {
            val builder = MediaRoute2Info.Builder(id, name).addFeature(FEATURE).addFeature(MediaRoute2Info.FEATURE_LIVE_AUDIO)
                .setType(if (local) MediaRoute2Info.TYPE_BUILTIN_SPEAKER else MediaRoute2Info.TYPE_REMOTE_SPEAKER)
                .setConnectionState(if (selected) MediaRoute2Info.CONNECTION_STATE_CONNECTED else MediaRoute2Info.CONNECTION_STATE_DISCONNECTED)
            volume?.let { builder.setVolumeHandling(MediaRoute2Info.PLAYBACK_VOLUME_VARIABLE).setVolumeMax(100).setVolume(it) }
            return builder.build()
        }

        internal fun show(device: Shown?) {
            if (device == null) {
                session?.let { notifySessionReleased(it.id) }
                session = null
                notifyRoutes(emptyList())
                return
            }
            val choices = device.devices.toMap().toMutableMap()
            choices[device.id] = device.name
            notifyRoutes(listOf(route(HERE, getString(R.string.remote_this_phone), true, false, null)) +
                choices.map { (id, name) -> route(nativeId(id), name, false, id == device.id, device.volume.takeIf { id == device.id }) })
            val made = sessionInfo(device)
            if (session == null) notifySessionCreated(REQUEST_ID_NONE, made) else notifySessionUpdated(made)
            session = made
        }

        override fun onCreateSession(requestId: Long, packageName: String, routeId: String, sessionHints: Bundle?) =
            notifyRequestFailed(requestId, REASON_ROUTE_NOT_AVAILABLE)
        override fun onReleaseSession(requestId: Long, sessionId: String) {
            if (sessionId != SESSION || session == null) return notifyRequestFailed(requestId, REASON_INVALID_COMMAND)
            notifySessionReleased(sessionId)
            session = null
            Nori.get(this).remotes.pick(null)
        }

        override fun onSelectRoute(requestId: Long, sessionId: String, routeId: String) = onTransferToRoute(requestId, sessionId, routeId)
        override fun onDeselectRoute(requestId: Long, sessionId: String, routeId: String) = notifyRequestFailed(requestId, REASON_INVALID_COMMAND)

        override fun onTransferToRoute(requestId: Long, sessionId: String, routeId: String) {
            val device = shown ?: return notifyRequestFailed(requestId, REASON_ROUTE_NOT_AVAILABLE)
            if (sessionId != SESSION) return notifyRequestFailed(requestId, REASON_INVALID_COMMAND)
            val target = when (routeId) {
                HERE -> null
                nativeId(device.id) -> device.id
                else -> device.devices.firstOrNull { nativeId(it.first) == routeId }?.first
                    ?: return notifyRequestFailed(requestId, REASON_ROUTE_NOT_AVAILABLE)
            }
            Nori.get(this).remotes.pick(target)
        }

        private fun setVolume(requestId: Long, volume: Int) {
            if (shown?.volume == null) return notifyRequestFailed(requestId, REASON_INVALID_COMMAND)
            Nori.get(this).remotes.command(Op.Volume(volume.coerceIn(0, 100).toUByte()))
        }

        override fun onSetRouteVolume(requestId: Long, routeId: String, volume: Int) {
            if (shown?.let { nativeId(it.id) } != routeId) return notifyRequestFailed(requestId, REASON_ROUTE_NOT_AVAILABLE)
            setVolume(requestId, volume)
        }

        override fun onSetSessionVolume(requestId: Long, sessionId: String, volume: Int) {
            if (sessionId != SESSION) return notifyRequestFailed(requestId, REASON_INVALID_COMMAND)
            setVolume(requestId, volume)
        }
    }
}
