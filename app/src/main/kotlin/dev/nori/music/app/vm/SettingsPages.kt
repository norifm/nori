package dev.nori.music.app.vm

import dev.nori.music.text.Fmt
import android.content.res.Resources
import dev.nori.music.app.R
import dev.nori.music.ffi.library.TextIndex
import dev.nori.music.ffi.model.MusicFolder
import dev.nori.music.ffi.settings.BeatModel
import dev.nori.music.ffi.settings.EqLevel
import dev.nori.music.ffi.settings.PageFacts
import dev.nori.music.ffi.settings.RowItem
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.settings.SectionId
import dev.nori.music.ffi.settings.SettingKind
import dev.nori.music.ffi.settings.SettingsAction
import dev.nori.music.ffi.settings.SettingsClient
import dev.nori.music.ffi.settings.SettingsLink
import dev.nori.music.ffi.settings.SettingsNote
import dev.nori.music.ffi.settings.SettingsPart
import dev.nori.music.ffi.settings.SettingsState
import dev.nori.music.ffi.settings.SettingSpec
import dev.nori.music.app.ui.providerStrings
import dev.nori.music.playback.DacState
import dev.nori.music.ffi.settings.StoredPrefs
import kotlin.math.roundToInt
import dev.nori.music.settings.label
import dev.nori.music.update.Updates
import dev.nori.music.net.said

/*
 * The settings screen's words and rows. Which pages, sections and rows there are, which are shown and
 * live, and every setting's options and range are the core's (nori-settings, settings_pages.rs, for the
 * phone); this words them, from res/values/strings.xml, and turns them into the screen's rows.
 */

/** A settings group: its own page, reached from the root of Settings. The icon is the screen's, by [id]. */
data class SettingsGroup(val id: String, val title: String, val summary: String)

/** One search result: the page it opens, the row it points at, and the line under its title. */
data class SettingsHit(val group: String, val key: String, val title: String, val detail: String)

/** One of a server's libraries, under it: whether it is shown, and the `musicFolders` value a tap sets. */
data class Pick(val name: String, val label: String, val on: Boolean, val tapped: String)

/** One choice in a list of options: what it says, and the value the core takes for it. */
data class SettingOption(val label: String, val value: String)

/**
 * One row of a settings page. [key] is what search lands on; `name` is the setting it changes, sent to
 * the core with the value picked; `enabled = false` is a setting the app is going to ignore right now.
 */
sealed interface SettingRow {
    data class Toggle(val key: String, val name: String, val title: String, val detail: String, val on: Boolean, val enabled: Boolean) : SettingRow
    /** [shown] is the chosen option's label (or the value itself when no option is it). */
    data class Choice(val key: String, val name: String, val title: String, val options: List<SettingOption>, val shown: String, val enabled: Boolean) : SettingRow
    /** A line of explanation among the rows. */
    data class Note(val text: String) : SettingRow
    /** A row that opens something ([action]), with a status at its end; [dimmed]: it is not reaching the sound now. */
    data class Link(val key: String, val title: String, val status: String, val dimmed: Boolean, val action: String, val divider: Boolean) : SettingRow
    /** A line of text and a button at its end; [error]: the line is a failure. */
    data class Action(val key: String, val title: String, val detail: String, val button: String, val enabled: Boolean, val error: Boolean, val action: String) : SettingRow
    data class Info(val key: String, val title: String, val detail: String) : SettingRow
    /** [level]: dragged through the core's in-place level edit instead of by [name]. */
    data class Slider(val name: String, val label: String, val value: Float, val min: Float, val max: Float, val centred: Boolean, val level: EqLevel?) : SettingRow
    /** Colour swatches, ARGB; the chosen one is drawn larger. */
    data class Palette(val name: String, val colours: List<Long>, val chosen: Long) : SettingRow
    /** A saved server; the one in use carries its [libraries] when it has more than one, under [note]. */
    data class Server(val id: String, val label: String, val detail: String, val active: Boolean, val libraries: List<Pick> = emptyList(), val note: String = "") : SettingRow
    data class Button(val title: String, val action: String) : SettingRow
    /** One of a ranked list (the lyrics services), switched where it stands and picked up to move. */
    data class Ranked(val key: String, val name: String, val id: String, val title: String, val detail: String, val on: Boolean) : SettingRow
    /** The compressor's gain reduction, read from the player only while the page is on screen. */
    data class CompressionMeter(val key: String) : SettingRow
    /** One setting with a few values, all on view as chips, the one it has lit. */
    data class ChipChoice(val key: String, val name: String, val title: String, val detail: String, val options: List<SettingOption>, val chosen: String) : SettingRow
    /** Text typed in (a service's key), hidden when [secret]. */
    data class Text(val key: String, val name: String, val title: String, val detail: String, val value: String, val secret: Boolean) : SettingRow
}

data class SettingsSection(val title: String, val rows: List<SettingRow>)

/** Empty [sections] for a page the screen draws itself (About, Licences). */
data class SettingsPage(val title: String, val sections: List<SettingsSection>)

/** A question a settings action asks before it is done. */
data class ActionAsk(val title: String, val text: String, val confirm: String)

/** What a page depends on besides the settings, from this phone. */
data class SettingsFacts(
    val dac: DacState = DacState(),
    /** Wallpaper colours need Android 12. */
    val wallpaperColours: Boolean = false,
    /** Songs AutoMix has measured. */
    val analysed: Int = 0,
    val sync: SyncUi = SyncUi(),
    val storage: StorageUi = StorageUi(),
    /** The active server's music folders. */
    val folders: List<MusicFolder> = emptyList(),
    /** Where the app's own update stands, and whether this build installs one. */
    val update: Updates.State = Updates.State.Idle,
    val installsUpdates: Boolean = false,
)

/** A row's key: its title, lowercased, with every run of anything but a-z and 0-9 made one '-'. Search lands on a row by it. */
fun settingKey(title: String): String {
    val key = StringBuilder(title.length)
    for (c in title.lowercase()) {
        if (c in 'a'..'z' || c in '0'..'9') key.append(c) else if (key.isEmpty() || key.last() != '-') key.append('-')
    }
    return key.toString().trim('-')
}

/** The pages by the id the screen's routes use, with their titles and summaries. */
private val GROUPS = listOf(
    Triple("servers", dev.nori.music.ffi.settings.SettingsPage.SERVERS, R.string.settings_group_servers to R.string.settings_group_servers_summary),
    Triple("playing", dev.nori.music.ffi.settings.SettingsPage.PLAYING, R.string.settings_group_playing to R.string.settings_group_playing_summary),
    Triple("sound", dev.nori.music.ffi.settings.SettingsPage.SOUND, R.string.settings_group_sound to R.string.settings_group_sound_summary),
    Triple("look", dev.nori.music.ffi.settings.SettingsPage.LOOK, R.string.settings_group_look to R.string.settings_group_look_summary),
    Triple("lyrics", dev.nori.music.ffi.settings.SettingsPage.LYRICS, R.string.settings_group_lyrics to R.string.settings_group_lyrics_summary),
    Triple("lyrics-sources", dev.nori.music.ffi.settings.SettingsPage.LYRICS_SOURCES, R.string.settings_page_lyrics_sources to 0),
    Triple("library", dev.nori.music.ffi.settings.SettingsPage.LIBRARY, R.string.settings_group_library to R.string.settings_group_library_summary),
    Triple("data", dev.nori.music.ffi.settings.SettingsPage.DATA, R.string.settings_group_data to R.string.settings_group_data_summary),
    Triple("about", dev.nori.music.ffi.settings.SettingsPage.ABOUT, R.string.settings_group_about to R.string.settings_group_about_summary),
)

/** The groups the root of Settings lists, in the core's order; those the profile opens ([parts]). */
fun settingsGroups(res: Resources, parts: List<SettingsPart>): List<SettingsGroup> {
    val pages = dev.nori.music.ffi.settings.settingsPages(SettingsClient.PHONE, parts)
    return GROUPS.filter { it.second in pages && it.third.second != 0 }.map { (id, _, words) -> SettingsGroup(id, res.getString(words.first), res.getString(words.second)) }
}

/** A page's title: a group's, or one of the pages reached from inside another (`page:<id>`). */
private fun pageTitle(id: String): Int? = GROUPS.firstOrNull { it.first == id }?.third?.first ?: if (id == "licences") R.string.settings_page_licences else null

/** Each lyrics service's name and what it is good at, by the id the core stores it under. */
private val SERVICES: Map<String, Pair<Int, Int>> = mapOf(
    "BINILYRICS" to (R.string.lyrics_service_binilyrics to R.string.lyrics_service_binilyrics_about),
    "BETTER_LYRICS" to (R.string.lyrics_service_better_lyrics to R.string.lyrics_service_better_lyrics_about),
    "PAXSENIX" to (R.string.lyrics_service_paxsenix to R.string.lyrics_service_paxsenix_about),
    "LYRICS_PLUS" to (R.string.lyrics_service_lyrics_plus to R.string.lyrics_service_lyrics_plus_about),
    "PORTATO" to (R.string.lyrics_service_portato to R.string.lyrics_service_portato_about),
    "PAXSENIX_MUSIXMATCH" to (R.string.lyrics_service_paxsenix_musixmatch to R.string.lyrics_service_paxsenix_musixmatch_about),
    "SIMPMUSIC" to (R.string.lyrics_service_simpmusic to R.string.lyrics_service_simpmusic_about),
    "UNISON" to (R.string.lyrics_service_unison to R.string.lyrics_service_unison_about),
    "NETEASE" to (R.string.lyrics_service_netease to R.string.lyrics_service_netease_about),
    "KUGOU" to (R.string.lyrics_service_kugou to R.string.lyrics_service_kugou_about),
    "LRCLIB" to (R.string.lyrics_service_lrclib to R.string.lyrics_service_lrclib_about),
    "PAXSENIX_SPOTIFY" to (R.string.lyrics_service_paxsenix_spotify to R.string.lyrics_service_paxsenix_spotify_about),
    "YOUTUBE_CAPTIONS" to (R.string.lyrics_service_youtube_captions to R.string.lyrics_service_youtube_captions_about),
    "MEGALOBIZ" to (R.string.lyrics_service_megalobiz to R.string.lyrics_service_megalobiz_about),
    "YOUTUBE_MUSIC" to (R.string.lyrics_service_youtube_music to R.string.lyrics_service_youtube_music_about),
    "GENIUS" to (R.string.lyrics_service_genius to R.string.lyrics_service_genius_about),
)

/** What search finds a service by, besides its name. */
private val SERVICE_HINTS: Map<String, Int> = mapOf(
    "BINILYRICS" to R.string.settings_hint_service_apple, "BETTER_LYRICS" to R.string.settings_hint_service_apple, "PAXSENIX" to R.string.settings_hint_service_apple,
    "LYRICS_PLUS" to R.string.settings_hint_service_lyrics_plus, "PORTATO" to R.string.settings_hint_service_portato,
    "PAXSENIX_MUSIXMATCH" to R.string.settings_hint_service_musixmatch, "SIMPMUSIC" to R.string.settings_hint_service_simpmusic,
    "UNISON" to R.string.settings_hint_service_unison, "NETEASE" to R.string.settings_hint_service_netease, "KUGOU" to R.string.settings_hint_service_kugou,
    "LRCLIB" to R.string.settings_hint_service_lrclib, "PAXSENIX_SPOTIFY" to R.string.settings_hint_service_spotify,
    "YOUTUBE_CAPTIONS" to R.string.settings_hint_service_lines, "MEGALOBIZ" to R.string.settings_hint_service_lines,
    "YOUTUBE_MUSIC" to R.string.settings_hint_service_untimed, "GENIUS" to R.string.settings_hint_service_untimed,
)

/** A setting's title, and the words search also finds it by (0: none). */
private val SETTING_WORDS: Map<String, Pair<Int, Int>> = mapOf(
    "crossfadeSec" to (R.string.settings_crossfade to R.string.settings_hint_crossfade),
    "autoMix" to (R.string.settings_automix to R.string.settings_hint_automix),
    "autoMixBetterBeats" to (R.string.settings_better_beats to R.string.settings_hint_better_beats),
    "fadeMs" to (R.string.settings_fade to R.string.settings_hint_fade),
    "previousAlwaysSkips" to (R.string.settings_previous to R.string.settings_hint_previous),
    "speed" to (R.string.settings_speed to 0),
    "pitch" to (R.string.settings_pitch to 0),
    "skipSilence" to (R.string.settings_skip_silence to R.string.settings_hint_skip_silence),
    "skipExplicit" to (R.string.settings_skip_explicit to R.string.settings_hint_skip_explicit),
    "autoFill" to (R.string.settings_auto_fill to R.string.settings_hint_auto_fill),
    "autoFillKind" to (R.string.settings_auto_fill_kind to R.string.settings_hint_auto_fill_kind),
    "autoFillBasis" to (R.string.settings_auto_fill_basis to R.string.settings_hint_auto_fill_basis),
    "autoFillRemote" to (R.string.settings_auto_fill_remote to R.string.settings_hint_auto_fill_remote),
    "bridgeOffline" to (R.string.settings_bridge_offline to R.string.settings_hint_bridge_offline),
    "autoEqAuto" to (R.string.settings_autoeq_auto to R.string.settings_hint_autoeq_auto),
    "autoEqDownload" to (R.string.settings_autoeq_list to R.string.settings_hint_autoeq_list),
    "profilePerOutput" to (R.string.settings_per_device to R.string.settings_hint_per_device),
    "soundBypass" to (R.string.settings_sound_bypass to R.string.settings_sound_bypass_detail),
    "headphoneSound" to (R.string.settings_headphone_sound to R.string.settings_hint_headphone_sound),
    "compressor" to (R.string.settings_compressor to R.string.settings_compressor_detail),
    "compressorPreset" to (R.string.settings_compressor_preset to 0),
    "replayGain" to (R.string.settings_volume_leveling to R.string.settings_hint_volume_leveling),
    "loudnessLevel" to (R.string.settings_level to R.string.settings_hint_level),
    "volumeBoostDb" to (R.string.settings_volume_boost_title to R.string.settings_boost_note),
    "hiRes" to (R.string.settings_hi_res to R.string.settings_hint_hi_res),
    "maxRate" to (R.string.settings_max_rate to R.string.settings_hint_max_rate),
    "bitPerfect" to (R.string.settings_bit_perfect to R.string.settings_hint_bit_perfect),
    "offload" to (R.string.settings_offload to R.string.settings_hint_offload),
    "theme" to (R.string.settings_theme to R.string.settings_hint_theme),
    "amoled" to (R.string.settings_amoled to R.string.settings_hint_amoled),
    "dynamicColor" to (R.string.settings_wallpaper to R.string.settings_hint_wallpaper),
    "accent" to (R.string.settings_accent to 0),
    "coverColors" to (R.string.settings_cover_colours to R.string.settings_hint_cover_colours),
    "motionArtwork" to (R.string.settings_moving_covers to R.string.settings_hint_moving_covers),
    "motionArtworkMobile" to (R.string.settings_moving_covers_mobile to R.string.settings_hint_moving_covers_mobile),
    "hideStatusBar" to (R.string.settings_hide_status_bar to R.string.settings_hint_hide_status_bar),
    "hideGestureBar" to (R.string.settings_hide_gesture_bar to R.string.settings_hint_hide_gesture_bar),
    "keepAwake" to (R.string.settings_keep_awake to R.string.settings_hint_keep_awake),
    "burnInProtection" to (R.string.settings_burn_in to R.string.settings_hint_burn_in),
    "dimWhenIdle" to (R.string.settings_dim_idle to R.string.settings_hint_dim_idle),
    "dimLevel" to (R.string.settings_dim_level to 0),
    "controlsSide" to (R.string.settings_controls_side to R.string.settings_hint_controls_side),
    "motion" to (R.string.settings_motion to R.string.settings_hint_motion),
    "lyricsSweep" to (R.string.settings_lyrics_sweep to R.string.settings_hint_lyrics_sweep),
    "lyricsSize" to (R.string.settings_lyrics_size to 0),
    "lyricsTranslation" to (R.string.settings_lyrics_translation to R.string.settings_hint_lyrics_translation),
    "lyricsTimingButton" to (R.string.settings_lyrics_timing_button to R.string.settings_hint_lyrics_timing_button),
    "lyricsKeepScreenOn" to (R.string.settings_lyrics_screen_on to R.string.settings_hint_lyrics_screen_on),
    "lyricsOnline" to (R.string.settings_lyrics_online to R.string.settings_hint_lyrics_online),
    "lyricsPreferWords" to (R.string.settings_lyrics_words to R.string.settings_hint_lyrics_words),
    "paxSenixKey" to (R.string.settings_paxsenix_key to R.string.settings_hint_paxsenix_key),
    "betterLyricsKey" to (R.string.settings_betterlyrics_key to R.string.settings_hint_betterlyrics_key),
    "tapAction" to (R.string.settings_tap_action to R.string.settings_hint_tap_action),
    "swipeRight" to (R.string.settings_swipe_right to 0),
    "swipeLeft" to (R.string.settings_swipe_left to 0),
    "playlistNotes" to (R.string.settings_playlist_notes to R.string.settings_hint_playlist_notes),
    "tasteModel" to (R.string.settings_taste_model to R.string.settings_hint_taste_model),
    "scrobble" to (R.string.settings_scrobble to R.string.settings_hint_scrobble),
    "queueOffer" to (R.string.settings_queue_offer to 0),
    "thirdPartyLookups" to (R.string.settings_lookups to R.string.settings_hint_lookups),
    "remoteControl" to (R.string.settings_remote_control to R.string.settings_hint_remote_control),
    "wifi" to (R.string.settings_quality_wifi to 0),
    "mobile" to (R.string.settings_quality_mobile to 0),
    "download" to (R.string.settings_quality_download to 0),
    "cacheMb" to (R.string.settings_cache_size to R.string.settings_hint_cache_size),
    "altMaxBitRate" to (R.string.settings_alt_bitrate to 0),
    "updateCheck" to (R.string.settings_update_auto to R.string.settings_update_auto_detail),
    "updateBeta" to (R.string.settings_update_beta to R.string.settings_update_beta_detail),
)

/** The words of a setting's values that are names: by setting, in the options' order. */
private val NAMED: Map<String, IntArray> = mapOf(
    "autoFillKind" to intArrayOf(R.string.settings_auto_fill_kind_songs, R.string.settings_auto_fill_kind_albums),
    "autoFillBasis" to intArrayOf(R.string.settings_auto_fill_basis_similar, R.string.settings_auto_fill_basis_artist, R.string.settings_auto_fill_basis_genre, R.string.settings_auto_fill_basis_era),
    "headphoneSound" to intArrayOf(R.string.settings_off, R.string.settings_headphone_natural, R.string.settings_headphone_wide),
    "compressorPreset" to intArrayOf(R.string.settings_compressor_light, R.string.settings_compressor_car, R.string.settings_compressor_night),
    "replayGain" to intArrayOf(R.string.settings_off, R.string.settings_leveling_song, R.string.settings_leveling_album, R.string.settings_leveling_auto),
    "loudnessLevel" to intArrayOf(R.string.settings_level_quiet, R.string.settings_level_normal, R.string.settings_level_loud),
    "maxRate" to intArrayOf(R.string.settings_max_rate_auto, R.string.settings_max_rate_48, R.string.settings_max_rate_96, R.string.settings_max_rate_192),
    "theme" to intArrayOf(R.string.settings_theme_system, R.string.settings_theme_light, R.string.settings_theme_dark),
    "coverColors" to intArrayOf(R.string.settings_cover_colours_everywhere, R.string.settings_cover_colours_pages, R.string.settings_cover_colours_player, R.string.settings_off),
    "motion" to intArrayOf(R.string.settings_motion_full, R.string.settings_motion_reduced, R.string.settings_motion_eink),
    "hideStatusBar" to HIDE_BAR,
    "hideGestureBar" to HIDE_BAR,
    "keepAwake" to intArrayOf(
        R.string.settings_keep_awake_never, R.string.settings_keep_awake_sideways, R.string.settings_keep_awake_charging,
        R.string.settings_keep_awake_sideways_charging, R.string.settings_keep_awake_always,
    ),
    "dimWhenIdle" to intArrayOf(R.string.settings_dim_idle_never, R.string.settings_dim_idle_half_minute, R.string.settings_dim_idle_minute, R.string.settings_dim_idle_five_minutes),
    "dimLevel" to intArrayOf(R.string.settings_dim_level_light, R.string.settings_dim_level_medium, R.string.settings_dim_level_strong),
    "controlsSide" to intArrayOf(R.string.settings_controls_side_driver, R.string.settings_controls_side_left, R.string.settings_controls_side_right),
    "tapAction" to intArrayOf(R.string.settings_tap_play_list, R.string.settings_tap_play_one, R.string.settings_tap_queue, R.string.settings_tap_play_next),
    "swipeRight" to SWIPES,
    "swipeLeft" to SWIPES,
)

/** Both bars' choices, worded in the core's order (`HideBar`). */
private val HIDE_BAR get() = intArrayOf(
    R.string.settings_hide_bar_never, R.string.settings_hide_bar_sideways, R.string.settings_hide_bar_upright,
    R.string.settings_hide_bar_always, R.string.settings_hide_bar_protected,
)
private val SWIPES get() = intArrayOf(R.string.settings_swipe_none, R.string.settings_swipe_queue, R.string.settings_swipe_play_next, R.string.settings_swipe_favourite, R.string.settings_swipe_download)

/** What a link, action or info row is called, and what search also finds it by. */
private fun itemWords(item: RowItem): Pair<Int, Int>? = when (item) {
    is RowItem.Link -> when (item.to) {
        SettingsLink.EQUALIZER -> R.string.settings_equalizer to 0
        SettingsLink.SYSTEM_EFFECTS -> R.string.settings_system_effects to 0
        SettingsLink.HOME_ROWS -> R.string.settings_home_rows to 0
        SettingsLink.LIBRARY_SECTIONS -> R.string.settings_library_sections to 0
        SettingsLink.LYRICS_SOURCES -> R.string.settings_lyrics_sources to R.string.settings_hint_lyrics_sources
        SettingsLink.DOWNLOADS -> R.string.settings_downloads to 0
    }
    is RowItem.Action -> when (item.action) {
        SettingsAction.MEASURE_AGAIN -> R.string.settings_measured to R.string.settings_hint_measured
        SettingsAction.SYNC_LIBRARY -> R.string.settings_offline_search to R.string.settings_hint_offline_search
        SettingsAction.DOWNLOAD_LIBRARY -> R.string.settings_download_library to 0
        SettingsAction.ANALYSE_DOWNLOADS -> R.string.settings_analyse_downloads to R.string.settings_hint_analyse_downloads
        SettingsAction.CLEAR_STREAM -> R.string.settings_streamed to R.string.settings_hint_streamed
        SettingsAction.CLEAR_COVERS -> R.string.settings_covers to R.string.settings_hint_covers
        SettingsAction.CLEAR_LYRICS -> R.string.settings_lyrics_cache to R.string.settings_hint_lyrics_cache
        SettingsAction.CHECK_UPDATE -> R.string.settings_update_check_now to R.string.settings_hint_updates
        SettingsAction.ADD_SERVER, SettingsAction.JOIN_JAM -> null
    }
    is RowItem.Stored -> R.string.settings_stored to R.string.settings_hint_stored
    else -> null
}

/** The core's facts for a page, from this phone's. */
private fun pageFacts(res: Resources, f: SettingsFacts, parts: List<SettingsPart>) = PageFacts(
    parts = parts,
    wallpaperColors = f.wallpaperColours,
    car = res.configuration.uiMode and android.content.res.Configuration.UI_MODE_TYPE_MASK == android.content.res.Configuration.UI_MODE_TYPE_CAR,
    folders = f.folders.map { it.id },
    analysed = f.analysed.toUInt(),
    indexedSongs = f.sync.indexed.songs,
    syncing = f.sync.running,
    streamBytes = f.storage.streamBytes,
    coverBytes = f.storage.coverBytes,
    lyricsBytes = f.storage.lyricsBytes,
    downloadSongs = f.storage.downloadSongs.toUInt(),
    storageBusy = f.storage.busy,
)

/**
 * The settings search, over every row of every page in this [res]'s language (the core's rows, as the
 * pages have them, so the entry and the row cannot drift apart): titles first, then anything whose words
 * mention the query, each in the order of the pages (the core's `TextIndex`, one call per query).
 */
class SettingsSearch(private val res: Resources, p: StoredPrefs, s: SettingsState, f: SettingsFacts, parts: List<SettingsPart>) {
    private val entries: List<Triple<String, String, String>> = GROUPS.flatMap { (id, page, _) ->
        dev.nori.music.ffi.settings.settingsPage(page, SettingsClient.PHONE, p, s, pageFacts(res, f, parts)).flatMap { it.rows }.mapNotNull { row ->
            val item = row.item
            val (title, hint) = when (item) {
                is RowItem.Setting -> SETTING_WORDS[item.spec.name]?.let { (t, h) -> res.getString(t) to (if (h == 0) "" else res.getString(h)) }
                is RowItem.LyricsSource -> s.lyricsSources.getOrNull(item.index.toInt())?.let { src ->
                    SERVICES[src.id]?.let { (t, _) -> res.getString(t) to (SERVICE_HINTS[src.id]?.let(res::getString) ?: "") }
                }
                else -> itemWords(item)?.let { (t, h) -> res.getString(t) to (if (h == 0) "" else res.getString(h)) }
            } ?: return@mapNotNull null
            Triple(id, title, hint)
        }
    } + listOf(Triple("about", res.getString(R.string.settings_page_licences), res.getString(R.string.settings_hint_licences)))
    private val text: TextIndex by lazy { TextIndex(entries.map { (_, t, h) -> listOf(t, h) }) }

    fun find(query: String): List<SettingsHit> = text.ranked(query.trim()).map { i ->
        val (group, title, hint) = entries[i.toInt()]
        val page = pageTitle(group)?.let(res::getString).orEmpty()
        SettingsHit(group, settingKey(title), title, if (hint.isEmpty()) page else res.getString(R.string.settings_hit_detail, page, hint))
    }
}

/** One group's page for these settings, facts and the core's state; null for a group there is not, or one the profile does not open ([parts]). */
fun settingsPage(res: Resources, id: String, p: StoredPrefs, f: SettingsFacts, s: SettingsState, parts: List<SettingsPart>): SettingsPage? {
    val (_, page, words) = GROUPS.firstOrNull { it.first == id } ?: return null
    if (page !in dev.nori.music.ffi.settings.settingsPages(SettingsClient.PHONE, parts)) return null
    val b = PageBuilder(res, p, f, s)
    val sections = dev.nori.music.ffi.settings.settingsPage(page, SettingsClient.PHONE, p, s, pageFacts(res, f, parts))
    // A server's libraries are drawn inside the server in use, not as rows of their own.
    val libraries = sections.firstOrNull { it.id == SectionId.MUSIC_FOLDERS }?.rows.orEmpty().filter { it.shown }.mapNotNull { r ->
        val item = r.item as? RowItem.MusicFolder ?: return@mapNotNull null
        f.folders.getOrNull(item.index.toInt())?.let { Pick("musicFolders", it.name, item.on, item.tapped) }
    }
    val shown = sections.filter { it.id != SectionId.MUSIC_FOLDERS }.map { sec ->
        SettingsSection(res.getString(b.sectionTitle(sec.id)), sec.rows.filter { it.shown }.mapNotNull { b.row(it, libraries) })
    }.filter { it.rows.isNotEmpty() }
    return SettingsPage(res.getString(words.first), shown)
}

/** One of a DAC's modes, or what is playing: "44.1 kHz / 24 bit" (the rate in the phone's number style). */
fun dacModeWords(res: Resources, rate: UInt, bits: UInt): String =
    res.getString(R.string.settings_dac_mode, dev.nori.music.text.Fmt.kiloHertz(rate.toInt()), bits.toInt())

/** What the AudioTrack was opened with: "96.0 kHz / 32 bit, offloaded to the audio chip". */
fun dacTrackWords(res: Resources, t: dev.nori.music.playback.DacTrack): String {
    val mode = dacModeWords(res, t.rate.toUInt(), t.bits.toUInt())
    return if (t.offloaded) res.getString(R.string.settings_dac_offloaded, mode) else mode
}

/** Why a DAC plays nothing bit-perfect, in words. */
fun dacBlockWords(res: Resources, b: dev.nori.music.ffi.model.DacBlock): String = when (b) {
    is dev.nori.music.ffi.model.DacBlock.NoModeAtRate -> res.getString(R.string.settings_dac_no_mode, dev.nori.music.text.Fmt.kiloHertz(b.rate.toInt()))
    is dev.nori.music.ffi.model.DacBlock.NeedsExclusive -> {
        val depths = listOf(1 to 16, 2 to 24, 4 to 32).filter { (bit, _) -> b.depths.toInt() and bit != 0 }
            .joinToString(res.getString(R.string.settings_dac_or)) { (_, bits) -> res.getString(R.string.settings_dac_bits, bits) }
        res.getString(R.string.settings_dac_needs_exclusive, depths, dev.nori.music.text.Fmt.kiloHertz(b.rate.toInt()))
    }
    dev.nori.music.ffi.model.DacBlock.PlatformTooOld -> res.getString(R.string.settings_dac_old_android)
    dev.nori.music.ffi.model.DacBlock.Refused -> res.getString(R.string.settings_dac_refused)
}

/** Whether clearing [action] asks first, and what it says; null for one that is done at once. */
fun settingsActionAsks(res: Resources, action: String, f: SettingsFacts): ActionAsk? = when (action) {
    // What is gone is fetched again from somebody else's services, song by song.
    "clear-lyrics" -> ActionAsk(
        res.getString(R.string.settings_clear_lyrics_title),
        res.getString(R.string.settings_clear_lyrics_text, Fmt.bytes(f.storage.lyricsBytes)),
        res.getString(R.string.settings_clear),
    )
    else -> null
}

/**
 * What an update's state says, for About's row and the banner: the version found, the download's progress,
 * what is missing or what went wrong. [current] is this build's version.
 */
fun updateWords(res: Resources, s: Updates.State, current: String, installs: Boolean): String = when (s) {
    Updates.State.Idle -> res.getString(R.string.update_version, current)
    Updates.State.Checking -> res.getString(R.string.update_checking)
    is Updates.State.UpToDate -> res.getString(R.string.update_latest, current)
    is Updates.State.Available ->
        if (installs) res.getString(R.string.update_available, s.update.version, Fmt.bytes(s.update.apkBytes.toLong()))
        else res.getString(R.string.update_available_elsewhere, s.update.version)
    is Updates.State.NoApk -> res.getString(R.string.update_no_apk, s.version)
    is Updates.State.CheckFailed -> res.getString(R.string.update_check_failed, s.error.said.orEmpty())
    is Updates.State.Downloading -> res.getString(R.string.update_downloading, Fmt.bytes(s.done), Fmt.bytes(s.total))
    is Updates.State.Installing -> res.getString(R.string.update_installing)
    is Updates.State.NeedsPermission -> res.getString(R.string.update_needs_permission)
    is Updates.State.Failed -> when (val why = s.why) {
        is Updates.Failure.Download -> res.getString(R.string.update_failed_download, why.error.said.orEmpty())
        is Updates.Failure.Size -> res.getString(R.string.update_failed_size, Fmt.bytes(why.got), Fmt.bytes(why.expected))
        Updates.Failure.NotThisApp -> res.getString(R.string.update_failed_not_nori)
        // Android's own message is for the log (it says INSTALL_FAILED_... in capitals): each kind is worded here.
        is Updates.Failure.Install -> res.getString(
            when (why.status) {
                android.content.pm.PackageInstaller.STATUS_FAILURE_CONFLICT -> R.string.update_failed_signature
                android.content.pm.PackageInstaller.STATUS_FAILURE_STORAGE -> R.string.update_failed_storage
                android.content.pm.PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> R.string.update_failed_incompatible
                android.content.pm.PackageInstaller.STATUS_FAILURE_BLOCKED -> R.string.update_failed_blocked
                else -> R.string.update_failed_install
            },
        )
    }
}

/** The word on the update's main button in state [s] (see SettingsViewModel.updateNow); null for none. */
fun updateButton(res: Resources, s: Updates.State, installs: Boolean): String? = when (s) {
    is Updates.State.Available -> res.getString(if (installs) R.string.update_now else R.string.update_view)
    is Updates.State.NoApk -> res.getString(R.string.update_view)
    // Another try cannot get past a copy signed with another key: the release's page can.
    is Updates.State.Failed -> res.getString(if ((s.why as? Updates.Failure.Install)?.conflict == true) R.string.update_view else R.string.update_try_again)
    is Updates.State.NeedsPermission -> res.getString(R.string.update_allow)
    is Updates.State.Downloading -> res.getString(R.string.update_cancel)
    Updates.State.Checking, is Updates.State.Installing -> null
    else -> res.getString(R.string.update_check)
}

/** Turns the core's rows into the screen's, in words. */
private class PageBuilder(val res: Resources, val p: StoredPrefs, val f: SettingsFacts, val s: SettingsState) {
    fun str(id: Int) = res.getString(id)
    fun str(id: Int, vararg args: Any) = res.getString(id, *args)

    fun sectionTitle(id: SectionId): Int = when (id) {
        SectionId.ACCOUNTS -> R.string.settings_section_accounts
        SectionId.MUSIC_FOLDERS, SectionId.THIS_SERVER -> R.string.settings_section_this_server
        SectionId.BETWEEN -> R.string.settings_section_between
        SectionId.CONTROLS -> R.string.settings_section_controls
        SectionId.QUEUE -> R.string.settings_section_queue
        SectionId.EQUALIZER -> R.string.settings_section_equalizer
        SectionId.EFFECTS -> R.string.settings_section_effects
        SectionId.VOLUME -> R.string.settings_section_volume
        SectionId.OUTPUT -> R.string.settings_section_output
        SectionId.THEME -> R.string.settings_section_theme
        SectionId.COVER -> R.string.settings_section_cover
        SectionId.HOME -> R.string.settings_section_home
        SectionId.SCREEN -> R.string.settings_section_screen
        SectionId.MOTION -> R.string.settings_section_motion
        SectionId.DISPLAY -> R.string.settings_section_display
        SectionId.SOURCES -> R.string.settings_section_sources
        SectionId.ASKED_ORDER -> R.string.settings_section_asked_order
        SectionId.KEYS -> R.string.settings_section_keys
        SectionId.LISTS -> R.string.settings_section_lists
        SectionId.PLAYLISTS -> R.string.settings_section_playlists
        SectionId.SEARCH -> R.string.settings_section_search
        SectionId.HISTORY -> R.string.settings_section_history
        SectionId.ONLINE -> R.string.settings_section_online
        SectionId.DEVICES -> R.string.settings_section_devices
        SectionId.STREAMING -> R.string.settings_section_streaming
        SectionId.DOWNLOADS -> R.string.settings_section_downloads
        SectionId.STORAGE -> R.string.settings_section_storage
        SectionId.UPDATES -> R.string.settings_updates
    }

    // How values read.
    fun offOr(v: String, words: (String) -> String) = if (v == "0") str(R.string.settings_off) else words(v)
    fun seconds(v: String) = str(R.string.settings_seconds, v)
    fun millis(v: String) = str(R.string.settings_millis, v)
    fun percent(v: String) = str(R.string.settings_percent, v)

    /** A setting value in words. */
    fun valueWords(name: String, v: String, options: List<String>): String {
        NAMED[name]?.let { labels -> return options.indexOf(v).takeIf { it in labels.indices }?.let { str(labels[it]) } ?: v }
        return when (name) {
            "crossfadeSec" -> offOr(v, ::seconds)
            "fadeMs" -> offOr(v) { if (it.toInt() % 1000 == 0) seconds((it.toInt() / 1000).toString()) else millis(it) }
            "speed" -> if (v == "1") str(R.string.settings_normal) else str(R.string.settings_times, v)
            "pitch" -> {
                val pct = ((v.toFloatOrNull() ?: 1f) - 1f).times(100f).roundToInt()
                when {
                    pct == 0 -> str(R.string.settings_normal)
                    pct < 0 -> percent("−${-pct}")
                    else -> percent("+$pct")
                }
            }
            "lyricsSize" -> when (v) {
                "0" -> str(R.string.settings_lyrics_size_small)
                "1" -> str(R.string.settings_lyrics_size_medium)
                "2" -> str(R.string.settings_lyrics_size_large)
                else -> v
            }
            "wifi", "mobile", "download" -> {
                val (rate, format) = v.split(':', limit = 2).let { it[0] to it.getOrElse(1) { "" } }
                when (format) {
                    "" -> str(R.string.settings_quality_original)
                    "mp3" -> str(R.string.settings_quality_mp3, rate.toInt())
                    "opus" -> str(R.string.settings_quality_opus, rate.toInt())
                    else -> "$rate $format"
                }
            }
            "cacheMb" -> v.toInt().let { mb -> if (mb >= 1024 && mb % 1024 == 0) str(R.string.settings_gigabytes, mb / 1024) else str(R.string.settings_megabytes, mb) }
            "altMaxBitRate" -> if (v == "0") str(R.string.settings_alt_bitrate_none) else str(R.string.settings_kbps, v.toInt())
            else -> v
        }
    }

    /** The line under a setting, where it has one. */
    fun detail(name: String): String = when (name) {
        "crossfadeSec" -> str(R.string.settings_crossfade_detail)
        "autoMix" -> str(R.string.settings_automix_detail)
        "autoMixBetterBeats" -> {
            val mb = s.beatModelMb.toInt()
            if (!p.autoMixBetterBeats) str(R.string.settings_better_beats_off, mb) else when (val model = s.beatModel) {
                is BeatModel.Failed -> str(R.string.settings_better_beats_failed, str(when (model.why) {
                    dev.nori.music.ffi.automix.BeatFailure.NETWORK -> R.string.settings_better_beats_network
                    dev.nori.music.ffi.automix.BeatFailure.WRONG_FILE -> R.string.settings_better_beats_wrong_file
                    dev.nori.music.ffi.automix.BeatFailure.STORAGE -> R.string.settings_better_beats_storage
                }))
                BeatModel.WaitingForWifi -> str(R.string.settings_better_beats_waiting, mb)
                BeatModel.Downloading -> str(R.string.settings_better_beats_downloading, mb)
                BeatModel.Ready -> str(R.string.settings_better_beats_ready)
                else -> str(R.string.settings_better_beats_absent, mb)
            }
        }
        "previousAlwaysSkips" -> str(R.string.settings_previous_detail)
        "skipSilence" -> str(if (s.untouched) R.string.settings_skip_silence_held else R.string.settings_skip_silence_detail)
        "skipExplicit" -> str(R.string.settings_skip_explicit_detail)
        "autoFill" -> str(R.string.settings_auto_fill_detail)
        "autoFillKind" -> str(R.string.settings_hint_auto_fill_kind)
        "autoFillRemote" -> str(R.string.settings_auto_fill_remote_detail)
        "bridgeOffline" -> str(R.string.settings_bridge_offline_detail)
        "autoEqAuto" -> str(R.string.settings_autoeq_auto_detail)
        "autoEqDownload" -> str(R.string.settings_autoeq_list_detail)
        "profilePerOutput" -> str(R.string.settings_per_device_detail)
        "soundBypass" -> str(R.string.settings_sound_bypass_detail)
        "headphoneSound" -> str(R.string.settings_headphone_sound_detail)
        "compressor" -> str(R.string.settings_compressor_detail)
        "compressorPreset" -> str(R.string.settings_compressor_preset_detail)
        "replayGain" -> str(R.string.settings_volume_leveling_detail)
        "loudnessLevel" -> str(R.string.settings_level_detail)
        "hiRes" -> str(R.string.settings_hi_res_detail)
        "bitPerfect" -> bitPerfectWords(f.dac)
        "offload" -> str(if (f.dac.device != null) R.string.settings_offload_usb else if (s.offloadPaused) R.string.settings_offload_paused else R.string.settings_offload_detail)
        "theme" -> str(R.string.settings_hint_theme)
        "amoled" -> str(R.string.settings_amoled_detail)
        "dynamicColor" -> str(R.string.settings_wallpaper_detail)
        "coverColors" -> str(R.string.settings_cover_colours_detail)
        "motionArtwork" -> str(if (s.values["motionArtwork"] == "true" && p.motion != dev.nori.music.ffi.settings.Motion.FULL) R.string.settings_moving_covers_still else R.string.settings_moving_covers_detail)
        "motionArtworkMobile" -> str(R.string.settings_moving_covers_mobile_detail)
        "hideStatusBar" -> str(R.string.settings_hide_status_bar_detail)
        "hideGestureBar" -> str(R.string.settings_hide_gesture_bar_detail)
        "keepAwake" -> str(R.string.settings_keep_awake_detail)
        "burnInProtection" -> str(R.string.settings_burn_in_detail)
        "dimWhenIdle" -> str(R.string.settings_dim_idle_detail)
        "dimLevel" -> str(R.string.settings_dim_level_detail)
        "controlsSide" -> str(R.string.settings_controls_side_detail)
        "motion" -> str(R.string.settings_motion_detail)
        "lyricsSweep" -> str(R.string.settings_lyrics_sweep_detail)
        "lyricsTranslation" -> str(R.string.settings_lyrics_translation_detail)
        "lyricsTimingButton" -> str(R.string.settings_lyrics_timing_button_detail)
        "lyricsKeepScreenOn" -> str(R.string.settings_lyrics_screen_on_detail)
        "lyricsOnline" -> str(R.string.settings_lyrics_online_detail)
        "lyricsPreferWords" -> str(R.string.settings_lyrics_words_detail)
        "paxSenixKey" -> str(R.string.settings_paxsenix_key_detail)
        "betterLyricsKey" -> str(R.string.settings_betterlyrics_key_detail)
        "tapAction" -> str(R.string.settings_hint_tap_action)
        "playlistNotes" -> str(R.string.settings_playlist_notes_detail)
        "tasteModel" -> str(R.string.settings_taste_model_detail)
        "scrobble" -> str(R.string.settings_scrobble_detail)
        "queueOffer" -> str(R.string.settings_queue_offer_detail)
        "thirdPartyLookups" -> str(R.string.settings_lookups_detail)
        "remoteControl" -> str(R.string.settings_remote_control_detail)
        "updateCheck" -> str(R.string.settings_update_auto_detail)
        "updateBeta" -> str(R.string.settings_update_beta_detail)
        else -> ""
    }

    /** The screen's row for one of the core's; null for one this screen does not draw. */
    fun row(r: dev.nori.music.ffi.settings.SettingsRow, libraries: List<Pick>): SettingRow? = when (val item = r.item) {
        is RowItem.Setting -> setting(item.spec, item.value, item.chips, r.enabled)
        is RowItem.Link -> link(item.to)
        is RowItem.Action -> action(item.action, r.enabled)
        is RowItem.Note -> when (item.note) {
            SettingsNote.HELD_BY_DAC -> SettingRow.Note(str(R.string.settings_held_by_dac))
            SettingsNote.BOOST -> SettingRow.Note(str(R.string.settings_boost_note))
            SettingsNote.DAC -> dacDetail(f.dac)?.let { SettingRow.Note(it) }
            SettingsNote.LYRICS_SOURCES -> SettingRow.Note(str(if (s.values["lyricsOnline"] == "true") R.string.settings_lyrics_sources_note else R.string.settings_lyrics_sources_note_off))
        }
        is RowItem.Stored -> {
            val st = f.storage
            val bytes = Fmt::bytes
            val t = str(R.string.settings_stored)
            SettingRow.Info(
                settingKey(t), t,
                str(
                    R.string.settings_stored_detail, bytes(st.streamBytes), bytes(st.coverBytes), bytes(st.lyricsBytes),
                    res.getQuantityString(R.plurals.settings_stored_downloads, st.downloadSongs, st.downloadSongs, bytes(st.downloadBytes)), bytes(st.indexBytes),
                ),
            )
        }
        is RowItem.Server -> p.servers.getOrNull(item.index.toInt())?.let { sv ->
            val active = sv.id == p.activeServerId
            SettingRow.Server(sv.id, serverLabel(sv), serverDetail(sv, active), active, if (active) libraries else emptyList(), str(R.string.settings_music_folder_detail))
        }
        is RowItem.LyricsSource -> s.lyricsSources.getOrNull(item.index.toInt())?.let { src ->
            val (title, about) = SERVICES[src.id] ?: return null
            val t = str(title)
            SettingRow.Ranked(settingKey(t), "lyricsService:${src.id}", src.id, t, str(about), src.on)
        }
        is RowItem.CompressionMeter -> SettingRow.CompressionMeter("compression-meter")
        is RowItem.MusicFolder -> null
    }

    fun setting(spec: SettingSpec, value: String, chips: Boolean, enabled: Boolean): SettingRow? {
        val name = spec.name
        val title = SETTING_WORDS[name]?.first?.let(::str) ?: name
        val key = settingKey(title)
        return when (spec.kind) {
            SettingKind.SWITCH -> SettingRow.Toggle(key, name, title, detail(name), value == "true", enabled)
            SettingKind.CHOICE -> {
                val options = spec.options.map { SettingOption(valueWords(name, it, spec.options), it) }
                if (chips) SettingRow.ChipChoice(key, name, title, detail(name), options, value)
                else SettingRow.Choice(key, name, title, options, options.firstOrNull { it.value == value }?.label ?: valueWords(name, value, spec.options), enabled)
            }
            SettingKind.LEVEL -> {
                val db = value.toFloatOrNull() ?: spec.min
                val shown = if (db <= 0f) str(R.string.settings_off) else str(R.string.settings_db, Fmt.signedDb(db))
                SettingRow.Slider(name, str(R.string.settings_volume_boost, shown), db, spec.min, spec.max, false, if (name == "volumeBoostDb") EqLevel.VOLUME_BOOST else null)
            }
            SettingKind.TEXT -> SettingRow.Text(key, name, title, detail(name), value, secret = true)
            SettingKind.COLOUR -> SettingRow.Palette(name, spec.options.map { it.toLong() }, p.accent)
        }
    }

    fun link(to: SettingsLink): SettingRow = when (to) {
        SettingsLink.EQUALIZER -> {
            // The chain is taken out of the path by the same rule the transitions are, so the row says so
            // rather than reading "On" over sound it is not touching.
            val status = str(if (s.untouched) R.string.settings_equalizer_off_now else if (s.soundChainOn) R.string.settings_on else R.string.settings_off)
            linkRow(R.string.settings_equalizer, status, s.untouched, "equalizer", divider = true)
        }
        SettingsLink.SYSTEM_EFFECTS -> linkRow(R.string.settings_system_effects, "", false, "system-effects", divider = false)
        SettingsLink.HOME_ROWS -> linkRow(R.string.settings_home_rows, "", false, "home-rows", divider = true)
        SettingsLink.LIBRARY_SECTIONS -> linkRow(R.string.settings_library_sections, "", false, "library-sections", divider = false)
        SettingsLink.LYRICS_SOURCES -> {
            val n = s.lyricsSources.count { it.on }
            val status = if (n == 0) str(R.string.settings_lyrics_sources_none) else res.getQuantityString(R.plurals.settings_lyrics_sources_on, n, n)
            linkRow(R.string.settings_lyrics_sources, status, n == 0, "page:lyrics-sources", divider = false)
        }
        SettingsLink.DOWNLOADS -> actionRow(R.string.settings_downloads, str(R.string.settings_downloads_detail), str(R.string.settings_show), true, "downloads")
    }

    fun action(a: SettingsAction, enabled: Boolean): SettingRow? {
        val clearing = str(if (f.storage.busy) R.string.settings_clearing else R.string.settings_clear)
        return when (a) {
            SettingsAction.MEASURE_AGAIN -> actionRow(R.string.settings_measured, res.getQuantityString(R.plurals.settings_measured_detail, f.analysed, f.analysed), str(R.string.settings_measure_again), enabled, "measure-again")
            SettingsAction.SYNC_LIBRARY -> {
                val sync = f.sync
                val indexed = sync.indexed
                val counts = str(
                    R.string.settings_offline_search_detail,
                    res.getQuantityString(R.plurals.settings_songs, indexed.songs.toInt(), indexed.songs.toInt()),
                    res.getQuantityString(R.plurals.settings_albums, indexed.albums.toInt(), indexed.albums.toInt()),
                    res.getQuantityString(R.plurals.settings_artists, indexed.artists.toInt(), indexed.artists.toInt()),
                )
                val t = str(R.string.settings_offline_search)
                SettingRow.Action(settingKey(t), t, sync.error ?: counts, str(if (sync.running) R.string.settings_updating else R.string.settings_update), enabled, sync.error != null, "sync-library")
            }
            SettingsAction.DOWNLOAD_LIBRARY -> actionRow(R.string.settings_download_library, str(R.string.settings_download_library_detail), str(R.string.settings_download), enabled, "download-library")
            SettingsAction.ANALYSE_DOWNLOADS -> actionRow(R.string.settings_analyse_downloads, str(R.string.settings_analyse_downloads_detail), str(R.string.settings_analyse), enabled, "analyse-downloads")
            SettingsAction.CLEAR_STREAM -> actionRow(R.string.settings_streamed, str(R.string.settings_streamed_detail), clearing, enabled, "clear-stream")
            SettingsAction.CLEAR_COVERS -> actionRow(R.string.settings_covers, str(R.string.settings_covers_detail), clearing, enabled, "clear-covers")
            SettingsAction.CLEAR_LYRICS -> actionRow(R.string.settings_lyrics_cache, str(R.string.settings_lyrics_cache_detail, Fmt.bytes(f.storage.lyricsBytes)), clearing, enabled, "clear-lyrics")
            SettingsAction.ADD_SERVER -> SettingRow.Button(str(R.string.settings_add_server), "add-server")
            SettingsAction.CHECK_UPDATE -> {
                // About's update check, with where it stands and its one button.
                val u = f.update
                val button = updateButton(res, u, f.installsUpdates)
                val act = when (u) {
                    is Updates.State.Downloading -> "update-cancel"
                    is Updates.State.Available, is Updates.State.Failed, is Updates.State.NeedsPermission, is Updates.State.NoApk -> "update-go"
                    else -> "update-check"
                }
                val t = str(R.string.settings_update_check_now)
                val failed = u is Updates.State.Failed || u is Updates.State.CheckFailed
                SettingRow.Action(
                    settingKey(t), t, updateWords(res, u, dev.nori.music.app.BuildConfig.VERSION_NAME, f.installsUpdates),
                    button ?: str(R.string.update_check), button != null, failed, act,
                )
            }
            SettingsAction.JOIN_JAM -> null
        }
    }

    fun actionRow(title: Int, detail: String, button: String, enabled: Boolean, act: String): SettingRow.Action {
        val t = str(title)
        return SettingRow.Action(settingKey(t), t, detail, button, enabled, false, act)
    }

    fun linkRow(title: Int, status: String, dimmed: Boolean, act: String, divider: Boolean): SettingRow.Link {
        val t = str(title)
        return SettingRow.Link(settingKey(t), t, status, dimmed, act, divider)
    }

    /** The bit-perfect switch's second line: what the DAC is doing, why it cannot, or what the switch is for. */
    fun bitPerfectWords(d: DacState): String = when {
        d.bitPerfect -> str(R.string.settings_bit_perfect_on, d.device ?: "null", (d.sampleRate / 1000.0).toString(), d.bits)
        d.blockedBy != null -> str(R.string.settings_bit_perfect_blocked, d.device ?: str(R.string.settings_bit_perfect_usb_dac), dacBlockWords(res, d.blockedBy!!))
        d.device != null && d.supported -> str(R.string.settings_bit_perfect_connected, d.device!!)
        d.device != null -> str(R.string.settings_bit_perfect_unsupported, d.device!!)
        else -> str(R.string.settings_bit_perfect_detail)
    }

    /** "Offers … · playing … · output …"; null when there is nothing to say. */
    fun dacDetail(d: DacState): String? {
        val parts = listOfNotNull(
            d.modes.takeIf { it.isNotEmpty() }?.let { m -> str(R.string.settings_dac_offers, m.joinToString(", ") { dacModeWords(res, it.rate, it.bits) }) },
            d.playing?.let { str(R.string.settings_dac_playing, dacModeWords(res, it.rate, it.bits)) },
            d.track?.let { str(R.string.settings_dac_output, dacTrackWords(res, it)) },
        )
        return if (parts.isEmpty()) null else parts.joinToString(str(R.string.settings_dac_separator))
    }

    /** A profile without a name or an address is called by its kind. */
    private fun serverLabel(sv: SavedServer): String = sv.label.ifBlank { str(providerStrings(sv.kind).name) }

    /** The line under a saved server: who signs in, whether it is in use, and what is special about it. */
    private fun serverDetail(sv: SavedServer, active: Boolean): String {
        val who = sv.user.ifEmpty { str(providerStrings(sv.kind).noUser) }
        val parts = mutableListOf(who, str(if (active) R.string.settings_server_in_use else R.string.settings_server_not_in_use))
        if (sv.wifiOnly) parts += str(R.string.settings_server_wifi_only)
        if (sv.altUrl.isNotBlank()) parts += str(R.string.settings_server_second_address)
        return parts.joinToString(str(R.string.settings_server_separator))
    }
}
