package dev.nori.music.playback

import android.content.Context
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper

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
