package dev.nori.music.remote

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraAccessException
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Size
import android.view.Surface

/**
 * The back camera for the pairing scanner: from [start] to [stop] it streams into a preview surface, and each
 * frame's luma is read for a QR code ([QrScan]) on a thread of its own, the latest frame only, so reading
 * paces itself; [found] hears each text read, on the main thread. Platform Camera2: no library. The camera
 * permission is the caller's to have.
 */
class QrCamera(context: Context, private val found: (String) -> Unit) {
    private val manager = context.getSystemService(CameraManager::class.java)
    private val thread = HandlerThread("nori-qr").apply { start() }
    private val handler = Handler(thread.looper)
    private val main = Handler(Looper.getMainLooper())

    /** The camera streamed from, the back one if there is one, and its frame size nearest 1280 × 720. */
    private val chosen: Pair<String, Size>? = runCatching {
        manager.cameraIdList.map { it to manager.getCameraCharacteristics(it) }
            .sortedBy { (_, c) -> if (c.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK) 0 else 1 }
            .firstNotNullOfOrNull { (id, c) ->
                c.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)?.getOutputSizes(ImageFormat.YUV_420_888)
                    ?.minByOrNull { kotlin.math.abs(it.width * it.height - 1280 * 720) }?.let { id to it }
            }
    }.getOrNull()

    /** The frame's size, as the sensor stands (landscape); null without a camera. */
    val size: Size? get() = chosen?.second

    /** Which start the camera answers: a stop makes the callbacks of the start before it moot. */
    @Volatile private var run = 0
    private var camera: CameraDevice? = null
    private var reader: ImageReader? = null
    /** A camera asked for has not answered yet, or one closed has not said so: its callbacks are still to come. */
    private var opening = false
    private var closing = 0
    /** [close] was called: the thread ends once the camera is closed, so its last callbacks still land. */
    private var ending = false

    @SuppressLint("MissingPermission")
    fun start(preview: Surface) {
        val (id, size) = chosen ?: return
        val mine = ++run
        handler.post {
            val frames = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
            reader = frames
            frames.setOnImageAvailableListener({ r ->
                val text = r.acquireLatestImage()?.use { frame -> frame.planes[0].let { QrScan.read(it.buffer, frame.width, frame.height, it.rowStride) } }
                if (text != null) main.post { if (run == mine) found(text) }
            }, handler)
            try {
                opening = true
                manager.openCamera(id, object : CameraDevice.StateCallback() {
                    override fun onOpened(device: CameraDevice) {
                        opening = false
                        if (run != mine) { closing++; return device.close() }
                        camera = device
                        stream(device, listOf(preview, frames.surface), mine)
                    }
                    override fun onDisconnected(device: CameraDevice) = gone(device)
                    override fun onError(device: CameraDevice, error: Int) = gone(device)
                    override fun onClosed(device: CameraDevice) {
                        closing--
                        if (ending && settled()) thread.quitSafely()
                    }
                }, handler)
            } catch (_: CameraAccessException) {
                opening = false
            } catch (_: SecurityException) {
                opening = false
            }
        }
    }

    private fun stream(device: CameraDevice, targets: List<Surface>, mine: Int) {
        val repeating = object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(session: CameraCaptureSession) {
                if (run != mine) return session.close()
                val request = device.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                    targets.forEach(::addTarget)
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                }.build()
                runCatching { session.setRepeatingRequest(request, null, handler) }
            }
            override fun onConfigureFailed(session: CameraCaptureSession) = Unit
        }
        // SessionConfiguration needs API 28; this is the same session.
        @Suppress("DEPRECATION")
        runCatching { device.createCaptureSession(targets, repeating, handler) }
    }

    /** The camera was taken away or failed: it is closed. */
    private fun gone(device: CameraDevice) {
        opening = false
        if (camera == device) camera = null
        closing++
        device.close()
    }

    /** No camera answer is still to come. */
    private fun settled() = !opening && closing == 0 && camera == null

    /** The camera closed; [start] opens it again (its preview surface was made anew). */
    fun stop() {
        run++
        handler.post {
            camera?.let { closing++; it.close() }
            camera = null
            reader?.close()
            reader = null
        }
    }

    /** Stopped for good: its thread ends, after the camera says it closed. */
    fun close() {
        stop()
        handler.post {
            ending = true
            if (settled()) thread.quitSafely()
        }
    }
}
