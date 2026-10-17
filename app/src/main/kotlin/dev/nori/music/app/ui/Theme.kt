package dev.nori.music.app.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import dev.nori.music.ffi.settings.StoredPrefs
import dev.nori.music.ffi.settings.ThemeMode

/**
 * Material You: the wallpaper's colours on Android 12+, otherwise a scheme grown from one accent colour.
 * AMOLED replaces every dark surface with true black, so those pixels are simply off.
 */
@Composable
fun NoriTheme(prefs: StoredPrefs, content: @Composable () -> Unit) {
    val context = LocalContext.current
    val system = isSystemInDarkTheme()
    // Light, dark or the phone's: the core's rule (nori_look::theme::is_dark), the same for every screen.
    val dark = remember(prefs.theme, system) { dev.nori.music.ffi.settings.themeIsDark(prefs.theme, system) }
    val scheme = remember(dark, prefs.dynamicColor, prefs.accent, prefs.amoled) { schemeOf(context, prefs, dark) }
    // Everything dressed in the theme's own colours - the plates behind the buttons, the chrome, the
    // status bar - worked out once per scheme in Rust (nori_look::dress) and only looked up after.
    val look = remember(scheme) { FixedLook(plainLook(scheme)) }
    // Sizes follow Android's own display size and font scale.
    androidx.compose.runtime.CompositionLocalProvider(LocalLook provides look) {
        MaterialTheme(colorScheme = scheme, typography = NoriTypography, content = content)
    }
}

/**
 * The scheme the app wears: the wallpaper's colours on Android 12+ when asked for, otherwise grown from the
 * accent, with AMOLED black over a dark one. Outside composition too, for the home-screen widgets.
 */
fun schemeOf(context: android.content.Context, prefs: StoredPrefs, dark: Boolean): ColorScheme {
    val base = when {
        prefs.dynamicColor && Build.VERSION.SDK_INT >= 31 && dark -> dynamicDarkColorScheme(context)
        prefs.dynamicColor && Build.VERSION.SDK_INT >= 31 -> dynamicLightColorScheme(context)
        else -> seeded(Color(prefs.accent), dark)
    }
    return if (dark && prefs.amoled) base.black() else base
}

/** Everything dressed in [scheme]'s own colours (nori_look::dress), as the core lays a look out. */
fun plainLook(scheme: ColorScheme): IntArray = dev.nori.music.look.CoverLook.plain(
    intArrayOf(
        scheme.background.toArgb(), scheme.onSurface.toArgb(), scheme.onSurfaceVariant.toArgb(), scheme.primary.toArgb(),
        scheme.onPrimary.toArgb(), scheme.surfaceVariant.toArgb(), scheme.surfaceContainer.toArgb(),
        scheme.surfaceContainerHigh.toArgb(), scheme.secondaryContainer.toArgb(), scheme.outlineVariant.toArgb(),
    ),
)

/** A light or dark scheme from one colour: tones of the same hue, worked out in Rust (`nori_look::theme::seeded`). */
private fun seeded(seed: Color, dark: Boolean): ColorScheme {
    val tones = dev.nori.music.look.CoverLook.tones(seed.toArgb(), dark)
    val t = List(tones.size) { Color(tones[it]) }
    return if (dark) darkColorScheme(
        primary = t[0], onPrimary = t[1], primaryContainer = t[2], onPrimaryContainer = t[3],
        secondary = t[4], secondaryContainer = t[5], onSecondaryContainer = t[6],
        surface = t[7], background = t[8], surfaceVariant = t[9], onSurfaceVariant = t[10],
    ) else lightColorScheme(
        primary = t[0], onPrimary = t[1], primaryContainer = t[2], onPrimaryContainer = t[3],
        secondary = t[4], secondaryContainer = t[5], onSecondaryContainer = t[6],
        surface = t[7], background = t[8], surfaceVariant = t[9], onSurfaceVariant = t[10],
    )
}

/** AMOLED black: the dark surfaces nori_look puts in their place (`dress::AMOLED`), so those pixels are off. */
private fun ColorScheme.black(): ColorScheme {
    val a = dev.nori.music.look.CoverLook.amoled()
    return copy(
        background = Color(a[0]), surface = Color(a[1]), surfaceDim = Color(a[2]),
        surfaceContainerLowest = Color(a[3]), surfaceContainerLow = Color(a[4]), surfaceContainer = Color(a[5]),
        surfaceContainerHigh = Color(a[6]), surfaceContainerHighest = Color(a[7]),
    )
}
