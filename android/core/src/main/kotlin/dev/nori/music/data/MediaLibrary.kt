package dev.nori.music.data

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.Build
import android.provider.MediaStore
import android.util.Size
import dev.nori.music.ffi.DeviceMusic
import dev.nori.music.ffi.local.DevicePlaylist
import dev.nori.music.ffi.local.FoundFile
import java.io.File

/**
 * The device's music as MediaStore lists it, which local profiles read (the core's `DeviceMusic`): null
 * while the music permission is not granted. The core opens each file at its path. MediaStore also gives
 * what the music permission alone can't reach: an album's picture from its folder (`cover.jpg`,
 * `folder.jpg`), and the `.m3u` playlists in the music folders.
 */
class MediaLibrary(context: Context) : DeviceMusic {
    private val context = context.applicationContext

    override fun list(): List<FoundFile>? {
        if (!granted(context)) return null
        val columns = arrayOf(MediaStore.Audio.Media.DATA, MediaStore.Audio.Media.SIZE, MediaStore.Audio.Media.DATE_MODIFIED)
        val found = ArrayList<FoundFile>()
        val cursor = context.contentResolver.query(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, columns, "${MediaStore.Audio.Media.IS_MUSIC} != 0", null, null) ?: return null
        cursor.use { c ->
            while (c.moveToNext()) {
                val path = c.getString(0) ?: continue
                found += FoundFile(path, c.getLong(1).coerceAtLeast(0).toULong(), c.getLong(2) * 1000)
            }
        }
        return found
    }

    /**
     * MediaStore's picture of the album of the file at [path] (its own, else its folder's), saved under the
     * app's files by album. Its thumbnails came in with Android 10: before, there is none.
     */
    override fun albumArt(path: String): String? {
        if (Build.VERSION.SDK_INT < 29 || !granted(context)) return null
        val columns = arrayOf(MediaStore.Audio.Media._ID, MediaStore.Audio.Media.ALBUM_ID)
        val (id, album) = context.contentResolver.query(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, columns, "${MediaStore.Audio.Media.DATA} = ?", arrayOf(path), null)
            ?.use { c -> if (c.moveToFirst()) c.getLong(0) to c.getLong(1) else null } ?: return null
        val picture = runCatching {
            context.contentResolver.loadThumbnail(ContentUris.withAppendedId(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, id), Size(ART_PX, ART_PX), null)
        }.getOrNull() ?: return null
        val out = File(context.filesDir, "local-covers/$album.jpg")
        out.parentFile?.mkdirs()
        out.outputStream().use { picture.compress(Bitmap.CompressFormat.JPEG, 90, it) }
        return out.path
    }

    @Suppress("DEPRECATION") // MediaStore's playlists are read-only from Android 12 on, which is all this reads.
    override fun playlists(): List<DevicePlaylist> {
        if (!granted(context)) return emptyList()
        val resolver = context.contentResolver
        val columns = arrayOf(MediaStore.Audio.Playlists._ID, MediaStore.Audio.Playlists.NAME, MediaStore.Audio.Playlists.DATA)
        val lists = resolver.query(MediaStore.Audio.Playlists.EXTERNAL_CONTENT_URI, columns, null, null, null)?.use { c ->
            buildList { while (c.moveToNext()) add(Triple(c.getLong(0), c.getString(1).orEmpty(), c.getString(2).orEmpty())) }
        }.orEmpty()
        return lists.map { (id, name, path) ->
            val members = MediaStore.Audio.Playlists.Members.getContentUri("external", id)
            val songs = resolver.query(members, arrayOf(MediaStore.Audio.Playlists.Members.DATA), null, null, MediaStore.Audio.Playlists.Members.PLAY_ORDER)?.use { c ->
                buildList { while (c.moveToNext()) c.getString(0)?.let(::add) }
            }.orEmpty()
            DevicePlaylist(path, name, songs)
        }
    }

    companion object {
        /** The permission that lets the app read the device's music. */
        val permission: String = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_AUDIO else Manifest.permission.READ_EXTERNAL_STORAGE

        /** The side of an album picture MediaStore is asked for, px: a full player's cover on a phone. */
        private const val ART_PX = 1024

        fun granted(context: Context): Boolean = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
    }
}
