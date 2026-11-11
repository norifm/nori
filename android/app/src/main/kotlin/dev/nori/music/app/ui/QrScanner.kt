package dev.nori.music.app.ui

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import dev.nori.music.remote.QrCamera

/**
 * The back camera's picture, upright, each frame read for a QR code: [onFound] hears every text read. The
 * camera runs only while this is on screen. Without a camera, [noCamera] shows in its place.
 */
@Composable
internal fun QrScanner(onFound: (String) -> Unit, noCamera: @Composable () -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val found by rememberUpdatedState(onFound)
    val camera = remember { QrCamera(context) { found(it) } }
    DisposableEffect(camera) { onDispose { camera.close() } }
    val size = camera.size ?: return noCamera()
    AndroidView(
        factory = { c ->
            SurfaceView(c).apply {
                holder.setFixedSize(size.width, size.height)
                holder.addCallback(object : SurfaceHolder.Callback {
                    override fun surfaceCreated(h: SurfaceHolder) = camera.start(h.surface)
                    override fun surfaceChanged(h: SurfaceHolder, format: Int, width: Int, height: Int) = Unit
                    override fun surfaceDestroyed(h: SurfaceHolder) = camera.stop()
                })
            }
        },
        // The sensor lies landscape; a portrait screen shows its frame turned upright.
        modifier = modifier.fillMaxWidth(0.75f).aspectRatio(size.height.toFloat() / size.width).clip(CardShape),
    )
}
