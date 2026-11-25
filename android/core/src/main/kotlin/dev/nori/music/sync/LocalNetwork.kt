package dev.nori.music.sync

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

/**
 * Android 17 asks the user before an app reaches the local network. nori asks only once this phone is
 * paired or the user opened pairing: its paired devices on the same network then talk directly. Refused,
 * they talk through the sync server, and a pairing code travels by camera or link.
 */
object LocalNetwork {
    /** The permission, which the platform's SDK names from Android 17. */
    const val PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

    /** Whether to ask for it now: on Android 17 and later, not granted yet. */
    fun wanted(context: Context): Boolean =
        Build.VERSION.SDK_INT >= ANDROID_17 && context.checkSelfPermission(PERMISSION) != PackageManager.PERMISSION_GRANTED

    private const val ANDROID_17 = 37
}
