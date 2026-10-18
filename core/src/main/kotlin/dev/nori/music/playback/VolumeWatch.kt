package dev.nori.music.playback

import android.content.Context
import android.database.ContentObserver
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import dev.nori.music.ffi.devices.OutputPort
import dev.nori.music.ffi.devices.outputPort

/**
 * Calls [changed] on the main thread whenever the music stream's volume may have changed, until the
 * returned function is called. The system announces a change with a broadcast (the one ExoPlayer's volume
 * tracking listens to); that is not public API, so the settings store, which the system writes every
 * volume to a moment later, is watched too.
 */
fun watchMusicVolume(context: Context, changed: () -> Unit): () -> Unit {
    val receiver = object : android.content.BroadcastReceiver() {
        override fun onReceive(c: Context, intent: android.content.Intent) {
            if (intent.getIntExtra("android.media.EXTRA_VOLUME_STREAM_TYPE", AudioManager.STREAM_MUSIC) == AudioManager.STREAM_MUSIC) changed()
        }
    }
    androidx.core.content.ContextCompat.registerReceiver(
        context, receiver, android.content.IntentFilter("android.media.VOLUME_CHANGED_ACTION"),
        androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED,
    )
    val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) = changed()
    }
    context.contentResolver.registerContentObserver(android.provider.Settings.System.CONTENT_URI, true, observer)
    return {
        context.unregisterReceiver(receiver)
        context.contentResolver.unregisterContentObserver(observer)
    }
}

/**
 * The music volume, for loudness compensation (nori_player::contour): read when the system's settings
 * say something changed and when the output changes, and handed on only when the step or the output is
 * another; the core applies it only when that moves the sound. Registered only while the setting is on:
 * switched off, nothing is listened to and nothing is read.
 */
class VolumeWatch(
    private val context: Context,
    private val output: () -> String,
    private val tell: (index: Int, max: Int, db: Float) -> Unit,
) {
    private val audio = context.getSystemService(AudioManager::class.java)
    private var stop: (() -> Unit)? = null
    private var lastIndex = -1
    private var lastOutput = ""

    /** Loudness compensation switched on or off. */
    fun set(enabled: Boolean) {
        if (enabled == (stop != null)) return
        if (enabled) {
            stop = watchMusicVolume(context, ::read)
            lastIndex = -1
            read()
        } else {
            stop?.invoke()
            stop = null
        }
    }

    /** The music went to another output, whose volume is its own. */
    fun outputChanged() {
        if (stop != null) read()
    }

    private fun read() {
        val index = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        val out = output()
        if (index == lastIndex && out == lastOutput) return
        lastIndex = index
        lastOutput = out
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        // The platform's own curve for this kind of output; an output that says 0 dB below the top step
        // (absolute volume, where the headphones turn themselves down) is read by the core's curve instead.
        var db = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) runCatching { audio.getStreamVolumeDb(AudioManager.STREAM_MUSIC, index, deviceType(out)) }.getOrDefault(Float.NaN) else Float.NaN
        if (db > -0.01f && index < max) db = Float.NaN
        tell(index, max, db)
    }

    private fun deviceType(output: String): Int = when (outputPort(output)) {
        OutputPort.BLUETOOTH -> AudioDeviceInfo.TYPE_BLUETOOTH_A2DP
        OutputPort.WIRED -> AudioDeviceInfo.TYPE_WIRED_HEADPHONES
        OutputPort.USB -> AudioDeviceInfo.TYPE_USB_HEADSET
        OutputPort.SPEAKER, OutputPort.OTHER -> AudioDeviceInfo.TYPE_BUILTIN_SPEAKER
    }
}
