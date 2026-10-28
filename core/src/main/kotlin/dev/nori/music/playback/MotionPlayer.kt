package dev.nori.music.playback

import android.content.Context
import android.view.TextureView
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import dev.nori.music.net.Http
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The player of a plugin's looping sleeve (the artwork slot): muted, looping, video only, and nothing to do with the music. It asks
 * for no audio focus, has no media session and no wake lock, opens no audio track (the audio track type
 * is switched off, so no audio renderer is ever enabled), and is its own ExoPlayer, apart from the
 * music's player in PlaybackService. It is the one thing media3's ExoPlayer still plays in the app.
 *
 * The ExoPlayer is built by the first [play] and let go of by [release]; the screen releases it when
 * the player is put away, when the app is left or the screen goes off, and when the song has no loop.
 * Main thread only, like every media3 player.
 *
 * Bytes come through the app's one OkHttp pool and the loop cache (MediaSources.motionCache),
 * which matters more than it looks: a loop is the same few seconds over and over, and without the cache
 * every pass would fetch them again.
 */
@OptIn(UnstableApi::class)
class MotionPlayer(
    private val context: Context,
    private val http: Http,
    private val sources: MediaSources,
    /** The loop answered this HTTP status: its plugin may look it up again. */
    private val onFailed: (Int) -> Unit,
) {
    private var player: ExoPlayer? = null
    private var view: TextureView? = null
    private var loaded: String? = null

    /** Where each loop was when it last stopped, so it goes on from there rather than from its start. Memory only. */
    private val positions = HashMap<String, Long>()

    /** Where the loaded loop is now, or where it stopped (ms); for the test bridge. */
    val position: Long get() = player?.currentPosition ?: loaded?.let { positions[it] } ?: 0

    /** Keeps where the loaded loop is. */
    private fun keepPosition(p: ExoPlayer) {
        loaded?.let { positions[it] = p.currentPosition }
    }

    private val _ready = MutableStateFlow<String?>(null)

    /** The video whose first frame is on the surface now, or null. The screen fades in only after this. */
    val ready: StateFlow<String?> = _ready.asStateFlow()

    /**
     * The cache is only built on the playback thread, when the first bytes are wanted: it does a little
     * disk work, and the screen that calls [play] must not wait for it.
     */
    private val cached: DataSource.Factory by lazy {
        CacheDataSource.Factory()
            .setCache(sources.motionCache)
            .setUpstreamDataSourceFactory(OkHttpDataSource.Factory(http.callFactory))
            .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)
    }
    private val bytes = DataSource.Factory { cached.createDataSource() }

    /**
     * Reads the start of the loop at [url] into the loop cache, so it plays at once when its song comes:
     * a single file's first [WARM_BYTES], or an HLS loop's playlists, init section and first segment of
     * the rendition this player picks (the largest within [MAX_SIDE] this phone decodes). Nothing is
     * decoded. Blocking: off the main thread.
     */
    fun warm(url: String) {
        if (!url.substringBefore('?').endsWith(".m3u8")) {
            read(url, WARM_BYTES)
            return
        }
        val master = text(url) ?: return
        val media = if ("#EXT-X-STREAM-INF" in master) rendition(url, master) ?: return else url
        val playlist = if (media == url) master else text(media) ?: return
        INIT.find(playlist)?.groupValues?.get(1)?.let { read(resolve(media, it), C.LENGTH_UNSET.toLong()) }
        playlist.lineSequence().map(String::trim).firstOrNull { it.isNotEmpty() && !it.startsWith("#") }
            ?.let { read(resolve(media, it), C.LENGTH_UNSET.toLong()) }
    }

    /** The variant of [master] this player would pick: the highest bandwidth within [MAX_SIDE] in a codec this phone decodes. */
    private fun rendition(base: String, master: String): String? {
        val lines = master.lines()
        return lines.indices.filter { lines[it].startsWith("#EXT-X-STREAM-INF") }.mapNotNull { i ->
            val tag = lines[i]
            val uri = lines.drop(i + 1).firstOrNull { it.isNotBlank() && !it.startsWith("#") }?.trim() ?: return@mapNotNull null
            val side = RESOLUTION.find(tag)?.destructured?.let { (w, h) -> maxOf(w.toInt(), h.toInt()) } ?: 0
            val bandwidth = BANDWIDTH.find(tag)?.groupValues?.get(1)?.toLong() ?: 0
            val codecs = CODECS.find(tag)?.groupValues?.get(1).orEmpty()
            Triple(uri, side, bandwidth).takeIf { side <= MAX_SIDE && decodes(codecs) }
        }.maxByOrNull { it.third }?.let { resolve(base, it.first) }
    }

    /** Whether this phone has a decoder for the video codec named in an HLS CODECS list. */
    private fun decodes(codecs: String): Boolean {
        val video = codecs.split(',').map(String::trim).firstOrNull { it.startsWith("avc") || it.startsWith("hvc") || it.startsWith("hev") || it.startsWith("dvh") } ?: return true
        val mime = androidx.media3.common.MimeTypes.getMediaMimeType(video) ?: return false
        return android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS).codecInfos.any { !it.isEncoder && mime in it.supportedTypes.map(String::lowercase) }
    }

    private fun resolve(base: String, ref: String): String = java.net.URI(base).resolve(ref).toString()

    private fun text(url: String): String? = runCatching { String(readAll(url)) }.getOrNull()

    /** Reads [length] bytes of [url] through the loop cache, which keeps them. */
    private fun read(url: String, length: Long) {
        runCatching { readAll(url, length) }.onFailure { dev.nori.music.NoriLog.w("motion warm: ${it.message}") }
    }

    private fun readAll(url: String, length: Long = C.LENGTH_UNSET.toLong()): ByteArray {
        val source = cached.createDataSource()
        return try {
            source.open(androidx.media3.datasource.DataSpec.Builder().setUri(url).setLength(length).build())
            androidx.media3.datasource.DataSourceUtil.readToEnd(source)
        } finally {
            source.close()
        }
    }

    private val listener = object : Player.Listener {
        override fun onRenderedFirstFrame() {
            _ready.value = loaded
        }

        override fun onPlayerError(error: PlaybackException) {
            dev.nori.music.NoriLog.w("motion artwork: ${error.errorCodeName}")
            // Asked again on the next rest rather than never: the screen calls play() when it wants it.
            loaded = null
            val status = (error.cause as? androidx.media3.datasource.HttpDataSource.InvalidResponseCodeException)?.responseCode
            if (status != null) onFailed(status)
        }
    }

    /** Where the video is drawn. The screen hands its surface over when it is composed. */
    fun show(target: TextureView) {
        if (target === view) return
        view?.let { player?.clearVideoTextureView(it) }
        view = target
        // A new surface has nothing on it until a frame is rendered there.
        _ready.value = null
        player?.setVideoTextureView(target)
    }

    /** [target] has left the screen; it is no longer drawn into, and not held on to. */
    fun hide(target: TextureView) {
        if (target !== view) return
        player?.clearVideoTextureView(target)
        view = null
        _ready.value = null
    }

    /** Plays [url] (HLS or a single file) on a loop, from where it last stopped, also after a [release]. */
    fun play(url: String) {
        val p = player ?: build().also { player = it }
        if (url != loaded || p.mediaItemCount == 0) {
            keepPosition(p)
            loaded = url
            _ready.value = null
            p.setMediaSource(DefaultMediaSourceFactory(bytes).createMediaSource(MediaItem.fromUri(url)))
            // Before its first frame, so that frame is the one it stopped on.
            positions[url]?.let(p::seekTo)
            p.prepare()
        } else if (p.playbackState == Player.STATE_IDLE) {
            // Rested (see [rest]) or failed: the same video again from the frame it stopped on.
            p.prepare()
        }
        p.play()
    }

    /** Holds the frame on screen; nothing decodes while paused. */
    fun pause() {
        val p = player ?: return
        p.pause()
        keepPosition(p)
    }

    /** Lets the ExoPlayer and its decoder go. The next [play] builds a new one. */
    fun release() {
        val p = player ?: return
        keepPosition(p)
        p.removeListener(listener)
        p.release()
        player = null
        loaded = null
        _ready.value = null
        live--
    }

    private fun build(): ExoPlayer = ExoPlayer.Builder(context)
        // A few seconds ahead, not media3's 50 s: the loop comes out of the cache on disk.
        .setLoadControl(
            DefaultLoadControl.Builder().setBufferDurationsMs(4_000, 8_000, 1_000, 2_000)
                .setTargetBufferBytes(4 * 1024 * 1024).setPrioritizeTimeOverSizeThresholds(false).build()
        )
        // The playback thread sleeps until a frame is due instead of looking every 10 ms: a loop of a
        // 24-frame video woke it a hundred times a second, far more often than there was a frame to show.
        .experimentalSetDynamicSchedulingEnabled(true)
        .build().apply {
        // No sound at all: nothing is decoded for the audio track, and the volume is nought besides.
        volume = 0f
        repeatMode = Player.REPEAT_MODE_ONE
        trackSelectionParameters = trackSelectionParameters.buildUpon()
            .setTrackTypeDisabled(C.TRACK_TYPE_AUDIO, true)
            // The still cover is 800 px; a video larger than this is only more bytes and more decoding.
            // One fixed choice, not an adaptive one, so every loop and every visit is the same rendition
            // and comes out of the cache.
            .setMaxVideoSize(MAX_SIDE, MAX_SIDE)
            .setForceHighestSupportedBitrate(true)
            .build()
        addListener(listener)
        view?.let { setVideoTextureView(it) }
        live++
    }

    companion object {
        private const val MAX_SIDE = 1080
        /** How much of a single-file loop [warm] reads ahead. */
        private const val WARM_BYTES = 1024L * 1024
        private val INIT = Regex("#EXT-X-MAP:URI=\"([^\"]+)\"")
        private val RESOLUTION = Regex("RESOLUTION=(\\d+)x(\\d+)")
        private val BANDWIDTH = Regex("[^-]BANDWIDTH=(\\d+)")
        private val CODECS = Regex("CODECS=\"([^\"]+)\"")

        /** How many of these players exist, for the test bridge: nought whenever no loop shows. */
        @Volatile var live = 0
            internal set
    }
}
