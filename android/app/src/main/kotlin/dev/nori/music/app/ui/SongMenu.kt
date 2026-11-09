package dev.nori.music.app.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.ExitTransition
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.ui.draw.alpha
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.withLink
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.CardItem
import dev.nori.music.data.wordsOf
import dev.nori.music.ffi.model.Artist
import dev.nori.music.ffi.model.Playlist
import dev.nori.music.ffi.model.Song

/**
 * One line of a sheet. The icon is what keeps a column of these from reading as a wall of text, so
 * every row of the song menu carries one; the sleep timer's own numbers are a list of values rather
 * than a list of actions, and they are left plain.
 */
@Composable
private fun Item(text: String, icon: ImageVector? = null, enabled: Boolean = true, divider: Boolean = true, onClick: () -> Unit) {
    // Not offered now (the core's `SongMenuItem::enabled`): there, greyed, and a tap does nothing.
    GroupRow(text, if (enabled) onClick else null, Modifier.alpha(if (enabled) 1f else 0.38f), icon = icon, divider = divider, minHeight = SHEET_ROW)
}

/** What a song's menu shows: its sheet, or one of what comes up over it. */
private enum class MenuStage { MENU, PLAYLIST, DETAILS, SLEEP, LYRICS }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SongMenu(
    /** The song the menu is for; null closes it (it slides away showing the last one). */
    song: Song?,
    actions: ActionsViewModel,
    onDismiss: () -> Unit,
    /**
     * Set when the player's own ⋯ opened this. The sleep timer is a property of the evening, not of
     * the song, so it has no business on a row's menu in a list - but the player needs it somewhere,
     * and the bottom of that screen belongs to the output switcher, the way Apple's does.
     */
    player: dev.nori.music.app.vm.PlayerViewModel? = null,
    /** The album or artist page it was opened on, which it does not offer to go to. */
    page: dev.nori.music.ffi.model.PageOrigin? = null,
) {
    // Which of the menu's stages is up, fresh each time the menu opens. The sheet slides down as the
    // sleep choices, the details or the playlist picker come up over it, and each of those leaves with
    // the menu when it is done.
    val open = song != null
    var stage by remember(open) { mutableStateOf(MenuStage.MENU) }
    NoriSheet(player?.takeIf { open && stage == MenuStage.SLEEP }, onDismiss) { p -> SleepChoices(p, onDismiss) }
    NoriSheet(player?.takeIf { open && stage == MenuStage.LYRICS }?.let { p -> song?.let { p to it.id } }, onDismiss) { (p, id) -> LyricsSheet(p, id) }
    NoriSheet(song?.takeIf { stage == MenuStage.DETAILS }, onDismiss) { s -> TrackInfo(s, actions) }
    NoriSheet(song?.takeIf { stage == MenuStage.PLAYLIST }, onDismiss) { s -> PlaylistPicker(listOf(s), actions, onDismiss) }

    // A sheet with a half-open stage swallows the first back gesture to collapse itself, which reads
    // as the menu refusing to close. There is only ever one stage here, so back always dismisses.
    NoriSheet(song?.takeIf { stage == MenuStage.MENU }, onDismiss) { song ->
        val downloads by actions.downloads.collectAsState()
        // No stretch past the ends and no lift, here or in the other sheets: a menu that fits moved under every swipe.
        Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
            // What the menu offers, in what order and where, is the core's (`menus::song_menu`); the words are
            // this app's (`Say.songAction`), made once when the menu opens.
            // The marks are collected here rather than taken from LocalStarMarks because the sheet is put up
            // outside the provider, at the top of the app, so that it outlives the row or the player that asked for it.
            val marks by actions.starMarks.collectAsState()
            val starred = marks.effectiveStar(dev.nori.music.data.StarKind.SONG, song.id, song.starred)
            val download = when (song.id) {
                in downloads.doneIds -> dev.nori.music.ffi.library.SongDownload.DONE
                in downloads.pendingIds -> dev.nori.music.ffi.library.SongDownload.PENDING
                else -> dev.nori.music.ffi.library.SongDownload.NONE
            }
            val rules = LocalRules.current
            val nav = LocalNav.current
            val lyrics = player?.let { p -> val l by p.lyrics.collectAsState(); hasWords(l, song.id) } == true
            var excluded by remember(song.id) { mutableStateOf(false) }
            LaunchedEffect(song.id) { excluded = actions.excludedFromMixes(song) }
            val items = remember(song, starred, excluded, download, player != null, lyrics, rules, page) { dev.nori.music.ffi.library.songMenu(song, starred, excluded, download, player != null, lyrics, rules.stars, rules.account, rules.downloads, rules.radio, rules.asks, page) }
            val labels = remember(items) { items.map { say.songAction(it.action) } }
            val at = { place: dev.nori.music.ffi.library.SongMenuPlace -> items.indices.filter { items[it].place == place } }
            @Composable fun titleGlyph(n: Int) = when (val a = items[n].action) {
                is dev.nori.music.ffi.library.SongAction.Favourite -> SheetGlyph(if (a.on) Kanji.Seal else Kanji.Sealed, labels[n], lit = !a.on) { actions.star(song, a.on) }
                else -> SheetGlyph(Iconoir.InfoCircle, labels[n]) { stage = MenuStage.DETAILS }
            }
            @Composable fun glyph(n: Int) = when (items[n].action) {
                dev.nori.music.ffi.library.SongAction.RemoveDownload -> SheetGlyph(Iconoir.Trash, labels[n], quiet = true) { actions.undownload(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.StopDownload -> SheetGlyph(Iconoir.Xmark, labels[n], quiet = true) { actions.cancelDownloads(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Download -> SheetGlyph(Iconoir.Download, labels[n], quiet = true) { actions.download(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Radio -> SheetGlyph(Iconoir.AntennaSignal, labels[n], quiet = true) { actions.startRadio(song); onDismiss() }
                else -> SheetGlyph(Iconoir.ShareIos, labels[n], quiet = true) { actions.share(song.id); onDismiss() }
            }
            @Composable fun line(n: Int, divider: Boolean) = when (val a = items[n].action) {
                dev.nori.music.ffi.library.SongAction.PlayNext -> Item(labels[n], Iconoir.PlaylistPlay, divider = divider) { actions.playNext(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.AddToQueue -> Item(labels[n], Iconoir.Playlist, divider = divider) { actions.enqueue(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.AddToPlaylist -> Item(labels[n], Iconoir.PlusCircle, divider = divider) { stage = MenuStage.PLAYLIST }
                // Stays open: the line turns to say the other way.
                is dev.nori.music.ffi.library.SongAction.ExcludeFromMixes -> Item(labels[n], Iconoir.Prohibition, divider = divider) { actions.excludeFromMixes(song, a.on); excluded = a.on }
                dev.nori.music.ffi.library.SongAction.Lyrics -> Item(labels[n], Iconoir.QuoteMessage, enabled = items[n].enabled, divider = divider) { stage = MenuStage.LYRICS }
                dev.nori.music.ffi.library.SongAction.SleepTimer -> Item(labels[n], Iconoir.HalfMoon, divider = divider) { stage = MenuStage.SLEEP }
                else -> Item(labels[n], Iconoir.FolderPlus, divider = divider) { actions.addToLibrary(song.id, isAlbum = false); onDismiss() }
            }
            val names = at(dev.nori.music.ffi.library.SongMenuPlace.NAMES).map { items[it].action }
            SheetHead(actions.cover(song.coverArt, CoverSize.ROW), song.title, "", trailing = { at(dev.nori.music.ffi.library.SongMenuPlace.TITLE).forEach { titleGlyph(it) } }) {
                SongNames(song, names) { a ->
                    when (a) {
                        is dev.nori.music.ffi.library.SongAction.GoToAlbum -> nav.album(a.id)
                        // The song's own cover stands in for a lone artist's until their page has one.
                        is dev.nori.music.ffi.library.SongAction.GoToArtist -> nav.artist(a.id, Artist(a.id, a.name, song.coverArt.takeIf { song.artists.size <= 1 }, null, 0u, false, false))
                        else -> Unit
                    }
                    onDismiss()
                }
            }
            HeadLine()
            // The album page's way of saying it, in its own case: "FLAC 16/44.1", "MP3 320 kbps".
            Row(Modifier.fillMaxWidth().heightIn(min = 40.dp).padding(start = SHEET_INSET, end = SHEET_GLYPH_END), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text(
                    remember(song) { say.quality(song) }.orEmpty(), Modifier.weight(1f), style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
                at(dev.nori.music.ffi.library.SongMenuPlace.GLYPHS).forEach { glyph(it) }
            }
            // What is done with the song, then what opens more or switches a state (a jam guest's menu has
            // neither): a menu shows everything.
            val lines = at(dev.nori.music.ffi.library.SongMenuPlace.LINE)
            for (group in listOf(lines.filter { !items[it].more }, lines.filter { items[it].more })) {
                if (group.isNotEmpty()) SheetGroup { group.forEachIndexed { k, n -> line(n, divider = k < group.lastIndex) } }
            }
            PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.SONG, dev.nori.music.ffi.plugins.Items.Songs(listOf(song)), onDismiss)
        }
    }
}

/** A glyph of the song menu, in the columns the title's and the line under it share; drawn the size of a line's icon. */
@Composable
private fun SheetGlyph(
    icon: ImageVector, description: String, modifier: Modifier = Modifier,
    enabled: Boolean = true, lit: Boolean = false, quiet: Boolean = false, onClick: () -> Unit,
) = GlyphButton(icon, description, modifier.size(SHEET_GLYPH), enabled = enabled, lit = lit, quiet = quiet, iconSize = SHEET_GLYPH_ICON, onClick = onClick)

/**
 * The song's artists, then its album: each name in [names] in ink, opening its page through [open]; what
 * stands between them, and a name of the page the menu is on, stays quiet.
 */
@Composable
private fun SongNames(song: Song, names: List<dev.nori.music.ffi.library.SongAction>, open: (dev.nori.music.ffi.library.SongAction) -> Unit) {
    val ink = MaterialTheme.colorScheme.onSurface
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    val link = androidx.compose.ui.text.TextLinkStyles(androidx.compose.ui.text.SpanStyle(color = ink, fontWeight = androidx.compose.ui.text.font.FontWeight.Medium))
    val style = MaterialTheme.typography.bodySmall.copy(color = quiet)
    val artists = remember(song) { dev.nori.music.ffi.library.artistPieces(song) }
    val line = androidx.compose.ui.text.buildAnnotatedString {
        artists.forEachIndexed { i, piece ->
            if (i > 0 && piece.id != null && artists[i - 1].id != null) append(", ")
            val go = names.firstOrNull { it is dev.nori.music.ffi.library.SongAction.GoToArtist && it.id == piece.id }
            if (go == null) append(piece.text)
            else withLink(androidx.compose.ui.text.LinkAnnotation.Clickable(piece.text, link) { open(go) }) { append(piece.text) }
        }
    }
    if (line.isNotEmpty()) Text(line, style = style, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
    val album = names.firstOrNull { it is dev.nori.music.ffi.library.SongAction.GoToAlbum }
    // A link, not a clickable: a clickable's touch area would reach up over the line above.
    val albumLine = androidx.compose.ui.text.buildAnnotatedString {
        if (album == null) append(song.album)
        else withLink(androidx.compose.ui.text.LinkAnnotation.Clickable(song.album, androidx.compose.ui.text.TextLinkStyles(androidx.compose.ui.text.SpanStyle(color = ink))) { open(album) }) { append(song.album) }
    }
    if (albumLine.isNotEmpty()) Text(albumLine, style = style, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
}

/**
 * An album's, playlist's or artist's card menu: play it, play it next, add it to the queue, and delete a
 * playlist of the account's own. A jam guest only asks.
 */
@Composable
fun CardMenu(card: CardItem?, actions: ActionsViewModel, onDismiss: () -> Unit) {
    var deleting by remember { mutableStateOf<dev.nori.music.ffi.model.Playlist?>(null) }
    DeleteQuestion(deleting, { it.name }, { actions.deletePlaylist(it.id) }, { deleting = null })
    NoriSheet(card, onDismiss) { card ->
        val (title, line, art) = when (card) {
            is CardItem.OfAlbum -> Triple(card.album.name, card.album.subtitle, card.album.coverArt)
            is CardItem.OfPlaylist -> Triple(card.playlist.name, card.playlist.owner.orEmpty(), card.playlist.coverArt)
            is CardItem.OfArtist -> Triple(card.artist.name, card.artist.albumCount?.takeIf { it > 0u }?.let { say.albums(it.toInt()) }.orEmpty(), card.artist.coverArt)
        }
        Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
            SheetHead(actions.cover(art, CoverSize.ROW), title, line, round = card is CardItem.OfArtist)
            SheetGroup {
                if (!LocalRules.current.asks) Item(say.play, Iconoir.Play) { actions.play(card); onDismiss() }
                Item(say.menuPlayNext, Iconoir.PlaylistPlay) { actions.queue(card, next = true); onDismiss() }
                Item(say.addToQueue, Iconoir.Playlist, divider = false) { actions.queue(card, next = false); onDismiss() }
            }
            if (card is CardItem.OfPlaylist && actions.editable(card.playlist)) SheetGroup {
                Item(say.delete, Iconoir.Trash, divider = false) { deleting = card.playlist; onDismiss() }
            }
            when (card) {
                is CardItem.OfAlbum -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.ALBUM, dev.nori.music.ffi.plugins.Items.Albums(listOf(card.album)), onDismiss)
                is CardItem.OfPlaylist -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.PLAYLIST, dev.nori.music.ffi.plugins.Items.Playlists(listOf(card.playlist)), onDismiss)
                is CardItem.OfArtist -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.ARTIST, dev.nori.music.ffi.plugins.Items.Artists(listOf(card.artist)), onDismiss)
            }
        }
    }
}

/** The actions plugins add to [kind]'s menu, after nori's own, each run on [items]; read before the menu opened. */
@Composable
private fun PluginItems(actions: ActionsViewModel, kind: dev.nori.music.ffi.plugins.ItemKind, items: dev.nori.music.ffi.plugins.Items, onDismiss: () -> Unit) {
    val menus by actions.pluginMenus.collectAsState()
    val entries = menus[kind].orEmpty()
    if (entries.isNotEmpty()) SheetGroup {
        entries.forEachIndexed { i, e ->
            Item(e.label, glyphIcon(e.glyph), divider = i < entries.lastIndex) { actions.runPluginMenu(e, items); onDismiss() }
        }
    }
}

/** A plugin's glyph as the app's own icon. */
@Composable
private fun glyphIcon(g: dev.nori.music.ffi.plugins.Glyph): ImageVector = when (g) {
    dev.nori.music.ffi.plugins.Glyph.SPARKLE -> Iconoir.Sparks
    dev.nori.music.ffi.plugins.Glyph.INFO -> Iconoir.InfoCircle
    dev.nori.music.ffi.plugins.Glyph.SHARE -> Iconoir.ShareIos
    dev.nori.music.ffi.plugins.Glyph.LINK -> Iconoir.ShareAndroid
    dev.nori.music.ffi.plugins.Glyph.SEARCH -> Iconoir.Search
    dev.nori.music.ffi.plugins.Glyph.HEART -> Kanji.Seal
    dev.nori.music.ffi.plugins.Glyph.DOWNLOAD -> Iconoir.Download
    dev.nori.music.ffi.plugins.Glyph.ADD -> Iconoir.PlusCircle
    dev.nori.music.ffi.plugins.Glyph.PLAY -> Iconoir.Play
    dev.nori.music.ffi.plugins.Glyph.TUNE -> Iconoir.ControlSlider
}

/**
 * Upward drags and flings the sheet's lines had no use for, kept from the sheet: an open sheet otherwise
 * lifts under them and springs back. Downward ones still reach it, so a swipe down closes it.
 */
private val KeepUpwardDrags = object : androidx.compose.ui.input.nestedscroll.NestedScrollConnection {
    override fun onPostScroll(consumed: androidx.compose.ui.geometry.Offset, available: androidx.compose.ui.geometry.Offset, source: androidx.compose.ui.input.nestedscroll.NestedScrollSource) =
        androidx.compose.ui.geometry.Offset(0f, available.y.coerceAtMost(0f))
    override suspend fun onPostFling(consumed: androidx.compose.ui.unit.Velocity, available: androidx.compose.ui.unit.Velocity) =
        androidx.compose.ui.unit.Velocity(0f, available.y.coerceAtMost(0f))
}

/** The song or record a menu is about, right under the sheet's handle: its cover, its name, a line, [more], and [trailing] at the right. */
@Composable
private fun SheetHead(cover: String?, title: String, line: String, round: Boolean = false, trailing: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit = {}, more: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = SHEET_INSET, end = SHEET_GLYPH_END, bottom = 8.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
        Cover(cover, 44.dp, radius = if (round) 22.dp else 6.dp)
        Column(Modifier.weight(1f).padding(start = 12.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            more()
        }
        trailing()
    }
}

/** A sheet's lines that belong together, on a card of their own as the library's groups sit on its page. */
@Composable
private fun SheetGroup(content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) = GroupCard(content = content)

/** The hairline under a sheet's head (the song or record), as wide as the cards under it. */
@Composable
private fun HeadLine() = Box(Modifier.padding(horizontal = SHEET_SIDE)) { Hairline(startIndent = 0.dp, endIndent = 0.dp) }

/** Song [id]'s lyrics as [l] has them, when it has words. */
private fun hasWords(l: dev.nori.music.ffi.lyrics.LyricsNow, id: String): Boolean =
    (l as? dev.nori.music.ffi.lyrics.LyricsNow.Shown)?.let { it.song == id && it.pick.lyrics.lines.isNotEmpty() } == true

/**
 * The playing song's lyrics: where they came from, how finely they are timed, who set their timing and
 * by how much (the core's `LyricsTiming`), and a step earlier or later, kept for these words.
 */
@Composable
private fun LyricsSheet(player: dev.nori.music.app.vm.PlayerViewModel, id: String) {
    val l by player.lyrics.collectAsState()
    val found = remember(l, id) { l.wordsOf(id) } ?: return
    val lyrics = found.lyrics
    val pluginNames by player.pluginNames.collectAsState()
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
        SheetTitle(say.lyrics)
        SheetGroup {
            GroupRow(
                remember(found.source, lyrics.synced, lyrics.wordTimed, pluginNames) { say.lyricsSource(found.source, lyrics.synced, lyrics.wordTimed, pluginNames) },
                subtitle = if (lyrics.synced) remember(lyrics.timing) { say.lyricsTiming(lyrics.timing) } else null,
                minHeight = SHEET_ROW, divider = lyrics.synced,
            )
            // The value in a slot of its own width, so − and + stay put as it changes.
            if (lyrics.synced) GroupRow(say.lyricsOffset, minHeight = SHEET_ROW, divider = false, trailing = {
                SheetGlyph(Iconoir.Minus, say.later) { player.nudgeLyrics(id, -1) }
                Text(
                    say.lyricsShift(dev.nori.music.ffi.lyrics.lyricsOffsetMs(lyrics.timing)),
                    Modifier.widthIn(min = 76.dp), style = MaterialTheme.typography.titleMedium,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                )
                SheetGlyph(Iconoir.Plus, say.lyricsEarlier) { player.nudgeLyrics(id, 1) }
            })
        }
        if (lyrics.synced && lyrics.timing is dev.nori.music.ffi.model.LyricsTiming.Yours) SheetGroup {
            Item(say.reset, Iconoir.Refresh, divider = false) { player.nudgeLyrics(id, 0) }
        }
    }
}

/** The sleep choices on their own sheet, so the song's menu is not buried under eleven of them. */
@Composable
private fun SleepChoices(player: dev.nori.music.app.vm.PlayerViewModel, onDone: () -> Unit) {
    val state by player.state.collectAsState()
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
        SheetTitle(say.sleepTimer)
        val running = state.sleep !is dev.nori.music.ffi.queue.Sleep.Off
        // The choices are the core's (`menus::sleep_choices`); "Off" is all zeros.
        val choices = remember(running) { dev.nori.music.ffi.library.sleepChoices(running) }
        val words = remember(choices) { choices.map(say::sleepChoice) }
        SheetGroup {
            choices.forEachIndexed { n, c -> Item(words[n], divider = n < choices.lastIndex) { player.sleep(c.minutes.toInt(), c.endOfTrack, c.songs.toInt()); onDone() } }
        }
    }
}

/**
 * Adding [songs] to a playlist, a sheet's content: New playlist, which turns into the new one's name in
 * place, then the playlists the account may change, each saying what it has of them already (the core's
 * `playlist_choices`). One that has them all is checked and takes nothing.
 */
@Composable
fun PlaylistPicker(songs: List<Song>, actions: ActionsViewModel, onDone: () -> Unit) {
    var choices by remember { mutableStateOf<List<dev.nori.music.ffi.library.PlaylistChoice>?>(null) }
    var naming by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    LaunchedEffect(Unit) { choices = actions.playlistChoices(songs) }
    val create = { if (name.isNotBlank()) { actions.addToNewPlaylist(name.trim(), songs); onDone() } }
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom).imePadding()) {
        SheetTitle(say.addToPlaylist)
        SheetGroup {
            if (naming) NameLine(name, { name = it }, create) else Item(say.newPlaylist, Iconoir.Plus, divider = false) { naming = true }
        }
        val listed = choices.orEmpty()
        if (choices == null) LoadingDots(Modifier.fillMaxWidth().padding(vertical = 16.dp).wrapContentWidth())
        if (listed.isNotEmpty()) SheetGroup {
            listed.forEachIndexed { n, c -> PlaylistChoiceRow(c, songs.size, actions, divider = n < listed.lastIndex) { actions.addToPlaylist(c); onDone() } }
        }
    }
}

/** New playlist turned into the new one's name, in the line's place: its glyph, the field, and Create. */
@Composable
private fun NameLine(name: String, onName: (String) -> Unit, create: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Row(Modifier.fillMaxWidth().heightIn(min = SHEET_ROW).padding(start = GROUP_INSET), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
        Icon(Iconoir.Plus, null, Modifier.size(20.dp), tint = scheme.onSurface)
        Box(Modifier.weight(1f).padding(start = GROUP_INSET)) {
            if (name.isEmpty()) Text(say.playlistName, style = MaterialTheme.typography.bodyLarge, color = scheme.onSurfaceVariant)
            androidx.compose.foundation.text.BasicTextField(
                name, onName, Modifier.fillMaxWidth().takesFocus(), singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = scheme.onSurface),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(scheme.primary),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(imeAction = androidx.compose.ui.text.input.ImeAction.Done),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { create() }),
            )
        }
        TextAction(say.create, create, enabled = name.isNotBlank())
    }
}

/** A playlist offered in [PlaylistPicker], for [asked] songs: its cover, its name, and what it has of them. */
@Composable
private fun PlaylistChoiceRow(c: dev.nori.music.ffi.library.PlaylistChoice, asked: Int, actions: ActionsViewModel, divider: Boolean, onClick: () -> Unit) {
    val full = c.adds.isEmpty()
    val scheme = MaterialTheme.colorScheme
    Column {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 52.dp).then(if (full) Modifier else Modifier.clickable(onClick = onClick))
                .padding(horizontal = GROUP_INSET, vertical = 8.dp),
            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
        ) {
            Cover(actions.cover(c.playlist.coverArt, CoverSize.ROW), PICKED_COVER, radius = 6.dp)
            Column(Modifier.weight(1f).padding(horizontal = GROUP_INSET)) {
                Text(c.playlist.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                Text(say.playlistHolds(c.holds.toInt(), asked, c.playlist.songCount.toInt()), style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1)
            }
            if (full) Icon(Iconoir.Check, null, Modifier.size(20.dp), tint = scheme.onSurface)
        }
        if (divider) Hairline(startIndent = GROUP_INSET * 2 + PICKED_COVER)
    }
}

private val PICKED_COVER = 40.dp

/**
 * Everything the server said about one file, a sheet's content: each line its name and value, on a card;
 * then what plugins add, each group on its own card under its name, a line with a link opening it.
 */
@Composable
fun TrackInfo(song: Song, actions: ActionsViewModel) {
    // Which rows there are, in what order and how each reads is nori-core's (`fmt::track_info`).
    val rows = remember(song) { say.trackInfo(song) }
    val groups by androidx.compose.runtime.produceState(emptyList<dev.nori.music.ffi.plugins.InfoGroup>(), song) { value = actions.pluginSongInfo(song) }
    val uri = androidx.compose.ui.platform.LocalUriHandler.current
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
        SheetTitle(say.details)
        // A path or an id is there to be copied.
        androidx.compose.foundation.text.selection.SelectionContainer {
            SheetGroup { rows.forEachIndexed { n, (label, value) -> InfoLine(label, value, divider = n < rows.lastIndex) } }
        }
        // Asked as the sheet opens: the plugins' lines come in under nori's own, never in place of them.
        AnimatedVisibility(groups.isNotEmpty(), enter = AppMotion.grow(), exit = ExitTransition.None) {
            Column {
                groups.forEach { g ->
                    GroupLabel(g.name)
                    SheetGroup {
                        g.lines.forEachIndexed { n, l ->
                            val link = l.link
                            GroupRow(l.words, link?.let { { runCatching { uri.openUri(it) } } }, chevron = link != null, minHeight = SHEET_ROW, divider = n < g.lines.lastIndex)
                        }
                    }
                }
            }
        }
    }
}

/** A line of [TrackInfo]: its name quiet at the start, its value in ink at the end, wrapping when long. */
@Composable
private fun InfoLine(label: String, value: String, divider: Boolean) {
    val scheme = MaterialTheme.colorScheme
    Column {
        Row(Modifier.fillMaxWidth().heightIn(min = 40.dp).padding(horizontal = GROUP_INSET, vertical = 8.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant, maxLines = 1)
            Text(value, Modifier.weight(1f).padding(start = 16.dp), style = MaterialTheme.typography.bodyMedium, textAlign = androidx.compose.ui.text.style.TextAlign.End)
        }
        if (divider) Hairline(startIndent = GROUP_INSET)
    }
}

/** Shown instead of nothing while songs are selected: the batch actions. */
@Composable
fun SelectionBar(actions: ActionsViewModel) {
    val selection by actions.selection.collectAsState()
    // Above the early return, so the picker can leave after the selection it cleared is gone.
    var picking by remember { mutableStateOf(false) }
    NoriSheet(selection.takeIf { picking && it.isNotEmpty() }, { picking = false }) { songs ->
        PlaylistPicker(songs, actions) { picking = false; actions.clearSelection() }
    }
    LaunchedEffect(selection.isEmpty()) { if (selection.isEmpty()) picking = false }
    // Rises in and out on the app's clock (NoriBar), never on one frame; while it leaves it shows the
    // selection it had.
    NoriBar(selection.takeIf { it.isNotEmpty() }) { songs ->
        androidx.compose.material3.Surface(
            Modifier.padding(horizontal = 12.dp, vertical = 8.dp), shape = PillShape, color = MaterialTheme.colorScheme.surfaceContainer,
            contentColor = MaterialTheme.colorScheme.onSurface, border = hairlineBorder(),
        ) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                SelectedCount(songs.size, Modifier.weight(1f).padding(start = 8.dp))
                BarAction(say.play) { actions.play(songs); actions.clearSelection() }
                BarAction(say.next) { actions.playNext(songs); actions.clearSelection() }
                BarAction(say.queue) { actions.enqueue(songs); actions.clearSelection() }
                if (LocalRules.current.account) BarAction(say.playlist) { picking = true }
                if (LocalRules.current.downloads) BarAction(say.get) { actions.download(songs); actions.clearSelection() }
                IconButton(actions::clearSelection) { Icon(Iconoir.Xmark, say.clearSelection, Modifier.size(20.dp)) }
            }
        }
    }
}

/**
 * One of the selection bar's text buttons: Material's, with its padding and its 58 dp least width taken
 * in (a set least width stands in for the default), so the five and the ✕ leave the count room on a
 * phone 360 dp wide.
 */
@Composable
private fun BarAction(label: String, onClick: () -> Unit) =
    TextButton(
        onClick, Modifier.widthIn(min = 40.dp), contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 8.dp),
        colors = androidx.compose.material3.ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurface),
    ) {
        Text(label, maxLines = 1, softWrap = false)
    }

/**
 * "3 selected" on one line, or the count alone where that does not fit: the bar's buttons come first,
 * and the words were squeezed into a column one letter wide.
 */
@Composable
private fun SelectedCount(n: Int, modifier: Modifier) {
    val full = remember(n) { say.selected(n) }
    val short = remember(n) { say.selectedShort(n) }
    val style = androidx.compose.material3.LocalTextStyle.current
    val measurer = androidx.compose.ui.text.rememberTextMeasurer()
    androidx.compose.foundation.layout.BoxWithConstraints(modifier) {
        val room = constraints.maxWidth
        val fits = remember(full, style, room) { measurer.measure(full, style, maxLines = 1, softWrap = false).size.width <= room }
        Text(if (fits) full else short, maxLines = 1, softWrap = false, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
    }
}

/** Where a group's icons and words start: the head's cover and the format line start there too. */
private val SHEET_INSET = Space.gutter
/** A menu sheet's line: Android's 48 dp touch target, so a long menu fits the screen. */
private val SHEET_ROW = 48.dp
/** Narrower than a page's glyphs, so six leave the line under the head room for the format. */
private val SHEET_GLYPH = 40.dp
private val SHEET_GLYPH_ICON = 20.dp
/** The head's glyphs end where a group's words do, their touch room past them. */
private val SHEET_GLYPH_END = SHEET_INSET - (SHEET_GLYPH - SHEET_GLYPH_ICON) / 2
