package dev.nori.music.playback

import android.os.SystemClock
import androidx.media3.common.DeviceInfo
import androidx.media3.common.ForwardingSimpleBasePlayer
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import dev.nori.music.ffi.plugins.Cast
import dev.nori.music.ffi.plugins.CastButton
import dev.nori.music.ffi.plugins.CastKind
import dev.nori.music.plugins.PluginHost

/**
 * The session's player while a plugin casts (PluginHost.cast): the notification, the lock screen and the
 * system's output switcher show the cast's device, and the volume keys set its volume (a remote playback
 * type, which media3 hands the system as a volume provider). Each control is offered to the cast first
 * (`Plugins::cast_button`); one it doesn't take goes to this phone's own player. While the device plays
 * the songs itself it takes the transport, and what plays is what it reports, so the service stays in the
 * foreground while it plays.
 */
@UnstableApi
class CastPlayer(controls: Player, private val plugins: PluginHost) : ForwardingSimpleBasePlayer(controls) {
    private var cast: Cast? = null
    /** When [cast] was reported, on elapsedRealtime. */
    private var reportedAt = 0L

    /** The cast changed (its device's state, or a cast of another kind). */
    fun show(c: Cast) {
        cast = c
        reportedAt = SystemClock.elapsedRealtime()
        invalidateState()
    }

    override fun getState(): State {
        val state = super.getState()
        val c = cast ?: return state
        val commands = state.availableCommands.buildUpon().addAll(Player.COMMAND_GET_DEVICE_VOLUME, Player.COMMAND_ADJUST_DEVICE_VOLUME_WITH_FLAGS)
            .apply { if (c.volume != null) add(Player.COMMAND_SET_DEVICE_VOLUME_WITH_FLAGS) }.build()
        val b = state.buildUpon().setAvailableCommands(commands)
            .setDeviceInfo(DeviceInfo.Builder(DeviceInfo.PLAYBACK_TYPE_REMOTE).setMaxVolume(STEPS).build())
            .setDeviceVolume(c.volume?.let { (it.toInt() * STEPS + 50) / 100 } ?: (STEPS / 2))
        if (c.kind != CastKind.TARGET) return b.build()
        val ran = if (c.playing) SystemClock.elapsedRealtime() - reportedAt else 0
        return b.setPlayWhenReady(c.playing, Player.PLAY_WHEN_READY_CHANGE_REASON_REMOTE)
            .setPlaybackState(if (state.playlist.isEmpty()) Player.STATE_ENDED else Player.STATE_READY)
            .setContentPositionMs(PositionSupplier.getExtrapolating(c.positionMs.toLong() + ran, if (c.playing) 1f else 0f))
            .build()
    }

    /** [button] for the cast; [here] when it doesn't take it. */
    private fun cast(button: CastButton, here: () -> ListenableFuture<*>): ListenableFuture<*> =
        if (plugins.castTook(button)) Futures.immediateVoidFuture() else here()

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> =
        cast(if (playWhenReady) CastButton.Play else CastButton.Pause) { super.handleSetPlayWhenReady(playWhenReady) }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val button = when (seekCommand) {
            Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> CastButton.Next
            Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> CastButton.Previous
            Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM -> CastButton.Seek(positionMs.coerceAtLeast(0).toULong())
            else -> CastButton.Jump(mediaItemIndex.coerceAtLeast(0).toUInt())
        }
        return cast(button) { super.handleSeek(mediaItemIndex, positionMs, seekCommand) }
    }

    override fun handleSetDeviceVolume(deviceVolume: Int, flags: Int): ListenableFuture<*> =
        step(deviceVolume - getState().deviceVolume)

    override fun handleIncreaseDeviceVolume(flags: Int): ListenableFuture<*> = step(1)

    override fun handleDecreaseDeviceVolume(flags: Int): ListenableFuture<*> = step(-1)

    private fun step(by: Int): ListenableFuture<*> {
        if (by != 0) plugins.castTook(if (by > 0) CastButton.VolumeUp else CastButton.VolumeDown)
        return Futures.immediateVoidFuture()
    }

    private companion object {
        /** The volume keys' steps over the device's 0 to 100. */
        const val STEPS = 20
    }
}
