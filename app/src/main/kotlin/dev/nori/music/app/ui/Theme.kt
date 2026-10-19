package dev.nori.music.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import dev.nori.music.ffi.look.Palette
import dev.nori.music.ffi.settings.StoredPrefs

/**
 * The season's theme (nori_look::theme): its palette in light, dark or AMOLED black, the calendar's on the
 * day when it follows the seasons. The date is read again each time the app comes back to the front, so a
 * season comes in on its own day without anything waiting for midnight.
 */
@Composable
fun NoriTheme(prefs: StoredPrefs, content: @Composable () -> Unit) {
    val system = isSystemInDarkTheme()
    // Light, dark or the phone's: the core's rule (nori_look::theme::is_dark), the same for every screen.
    val dark = remember(prefs.theme, system) { dev.nori.music.ffi.settings.themeIsDark(prefs.theme, system) }
    var today by remember { mutableStateOf(Today.now()) }
    androidx.lifecycle.compose.LifecycleResumeEffect(Unit) {
        Today.now().let { if (it != today) today = it }
        onPauseOrDispose {}
    }
    val scheme = remember(dark, prefs.season, prefs.amoled, today) { schemeOf(prefs, dark, today) }
    val accents = remember(prefs.season, today) { accentsOf(prefs, today) }
    // Everything dressed in the theme's own colors - the plates behind the buttons, the chrome, the
    // status bar - worked out once per scheme in Rust (nori_look::dress) and only looked up after.
    val look = remember(scheme) { FixedLook(plainLook(scheme)) }
    val systemDensity = androidx.compose.ui.platform.LocalDensity.current
    // The shorter side, not the width: the same in both orientations, so text keeps its size on a turn.
    val widthDp = androidx.compose.ui.platform.LocalConfiguration.current.smallestScreenWidthDp
    // dp and sp both follow the scale, so every size keeps its proportion to the screen; Android's font
    // scale is left as it is, that one is the reader's.
    val density = remember(systemDensity, widthDp) {
        val scale = uiScale(widthDp)
        if (scale == 1f) systemDensity else androidx.compose.ui.unit.Density(systemDensity.density * scale, systemDensity.fontScale)
    }
    androidx.compose.runtime.CompositionLocalProvider(
        androidx.compose.ui.platform.LocalDensity provides density, LocalLook provides look, LocalAccents provides accents,
    ) {
        // The status bar's icons follow the theme, not the system's dark mode the window was started in.
        SystemBarIcons(look)
        MaterialTheme(colorScheme = scheme, typography = NoriTypography, content = content)
    }
}

/** The day the theme is worn on, and which half of the world it is in (the seasons are the other way round in the south). */
data class Today(val month: Int, val day: Int, val southern: Boolean) {
    companion object {
        fun now(): Today {
            val d = java.time.LocalDate.now()
            return Today(d.monthValue, d.dayOfMonth, dev.nori.music.ffi.look.southern(java.util.TimeZone.getDefault().id))
        }
    }
}

/** The season's palette on [today], the calendar's when following (nori_look::theme::palette). */
private fun paletteOf(prefs: StoredPrefs, dark: Boolean, today: Today): Palette =
    dev.nori.music.ffi.look.palette(dev.nori.music.ffi.settings.seasonOf(prefs.season), today.month.toUInt(), today.day.toUInt(), today.southern, dark, prefs.amoled)

/**
 * The theme's accent for a light page and for a dark one, which a page dressed in a cover's colors wears
 * in place of the cover's own, whichever its page is (nori_look::dress::accented).
 */
@androidx.compose.runtime.Immutable
data class Accents(val light: Int, val dark: Int)

val LocalAccents = staticCompositionLocalOf { Accents(0, 0) }

fun accentsOf(prefs: StoredPrefs, today: Today = Today.now()): Accents =
    Accents(paletteOf(prefs, false, today).accent.toInt(), paletteOf(prefs, true, today).accent.toInt())

/** The scheme the app wears, from the season's palette. Outside composition too, for the home-screen widgets. */
fun schemeOf(prefs: StoredPrefs, dark: Boolean, today: Today = Today.now()): ColorScheme {
    val r = dev.nori.music.ffi.look.roles(paletteOf(prefs, dark, today))
    fun c(v: UInt) = Color(v.toInt())
    val bg = c(r.background)
    val ink = c(r.onBackground)
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        background = bg, onBackground = ink, surface = bg, onSurface = ink, onSurfaceVariant = c(r.onSurfaceVariant),
        primary = c(r.primary), onPrimary = c(r.onPrimary), primaryContainer = c(r.primaryContainer), onPrimaryContainer = c(r.onPrimaryContainer),
        secondary = c(r.secondary), onSecondary = c(r.onSecondary), secondaryContainer = c(r.secondaryContainer), onSecondaryContainer = ink,
        tertiary = c(r.primary), onTertiary = c(r.onPrimary), inversePrimary = c(r.primary),
        surfaceVariant = c(r.surfaceVariant), outline = c(r.outline), outlineVariant = c(r.outlineVariant),
        surfaceDim = bg, surfaceBright = c(r.container), inverseSurface = ink, inverseOnSurface = bg,
        surfaceContainerLowest = c(r.containerLowest), surfaceContainerLow = c(r.containerLow), surfaceContainer = c(r.container),
        surfaceContainerHigh = c(r.containerHigh), surfaceContainerHighest = c(r.containerHighest), surfaceTint = Color.Transparent,
    )
}

/** Everything dressed in [scheme]'s own colours (nori_look::dress), as the core lays a look out. */
fun plainLook(scheme: ColorScheme): IntArray = dev.nori.music.look.CoverLook.plain(
    intArrayOf(
        scheme.background.toArgb(), scheme.onSurface.toArgb(), scheme.onSurfaceVariant.toArgb(), scheme.primary.toArgb(),
        scheme.onPrimary.toArgb(), scheme.surfaceVariant.toArgb(), scheme.surfaceContainer.toArgb(),
        scheme.surfaceContainerHigh.toArgb(), scheme.secondaryContainer.toArgb(), scheme.outlineVariant.toArgb(),
    ),
)

/** Every size in the app was measured as a share of the width of a phone this many dp wide. */
private const val REFERENCE_WIDTH_DP = 411f

/**
 * How big the interface is drawn: laid out as if the screen were at least [REFERENCE_WIDTH_DP] wide, so a
 * phone set to a large display size (one measured at 358 dp draws everything a seventh larger) keeps the
 * proportions; it only ever shrinks, never below three quarters. A phone's rule, about Android's display
 * size: a desktop window has no such setting to undo.
 */
fun uiScale(screenWidthDp: Int): Float =
    if (screenWidthDp <= 0) 1f else (screenWidthDp / REFERENCE_WIDTH_DP).coerceIn(0.75f, 1f)
