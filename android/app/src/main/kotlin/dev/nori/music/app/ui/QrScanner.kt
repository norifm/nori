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
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.unit.dp
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

/** [text] as a QR code, black on white whatever the theme, as a scanner wants it; the room is kept while it is made. */
@Composable
internal fun QrPicture(text: String, side: androidx.compose.ui.unit.Dp, modifier: Modifier = Modifier) {
    val qr by androidx.compose.runtime.produceState<dev.nori.music.ffi.remote.QrCode?>(null, text) {
        value = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) { dev.nori.music.ffi.remote.qrCode(text) }
    }
    androidx.compose.foundation.layout.Box(
        modifier.clip(androidx.compose.foundation.shape.RoundedCornerShape(20.dp)).background(Color.White).padding(18.dp).size(side),
    ) {
        qr?.let { code ->
            androidx.compose.foundation.layout.Spacer(Modifier.fillMaxSize().drawWithCache {
                val n = code.size.toInt()
                val cell = size.width / n
                val path = Path()
                for (y in 0 until n) for (x in 0 until n) {
                    if (code.dark[y * n + x]) path.addRect(Rect(Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f)))
                }
                onDrawBehind { drawPath(path, Color.Black) }
            })
        }
    }
}
