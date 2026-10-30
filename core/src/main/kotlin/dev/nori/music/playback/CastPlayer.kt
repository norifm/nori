package dev.nori.music.playback

import androidx.media3.common.DeviceInfo
import androidx.media3.common.ForwardingSimpleBasePlayer
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import dev.nori.music.ffi.plugins.Cast
import dev.nori.music.ffi.plugins.CastButton
import dev.nori.music.plugins.PluginHost

/**
 * The session's player while a plugin casts (PluginHost.cast): the notification, the lock screen and the
 * system's output switcher show the cast's device, and the volume keys set its volume (a remote playback
 * type, which media3 hands the system as a volume provider). Everything else is this phone's own player's:
 * while the device plays the songs itself, the engine keeps the queue for it and plays nothing here
 * (`Engine::play_elsewhere`), and what it plays there is what the engine says.
 */
@UnstableApi
class CastPlayer(controls: Player, private val plugins: PluginHost) : ForwardingSimpleBasePlayer(controls) {
    private var cast: Cast? = null

    /** The cast changed (its device's state, or a cast of another kind). */
    fun show(c: Cast) {
        cast = c
        invalidateState()
    }

    override fun getState(): State {
        val state = super.getState()
        val c = cast ?: return state
        val commands = state.availableCommands.buildUpon().addAll(Player.COMMAND_GET_DEVICE_VOLUME, Player.COMMAND_ADJUST_DEVICE_VOLUME_WITH_FLAGS)
            .apply { if (c.volume != null) add(Player.COMMAND_SET_DEVICE_VOLUME_WITH_FLAGS) }.build()
        return state.buildUpon().setAvailableCommands(commands)
            .setDeviceInfo(DeviceInfo.Builder(DeviceInfo.PLAYBACK_TYPE_REMOTE).setMaxVolume(STEPS).build())
            .setDeviceVolume(c.volume?.let { (it.toInt() * STEPS + 50) / 100 } ?: (STEPS / 2))
            .build()
    }

    override fun handleSetDeviceVolume(deviceVolume: Int, flags: Int): ListenableFuture<*> {
        plugins.castVolumeTook(deviceVolume * 100 / STEPS)
        return Futures.immediateVoidFuture()
    }

    override fun handleIncreaseDeviceVolume(flags: Int): ListenableFuture<*> = step(1)

    override fun handleDecreaseDeviceVolume(flags: Int): ListenableFuture<*> = step(-1)

    private fun step(by: Int): ListenableFuture<*> {
        plugins.castTook(if (by > 0) CastButton.VOLUME_UP else CastButton.VOLUME_DOWN)
        return Futures.immediateVoidFuture()
    }

    private companion object {
        /** The volume keys' steps over the device's 0 to 100. */
        const val STEPS = 20
    }
}
