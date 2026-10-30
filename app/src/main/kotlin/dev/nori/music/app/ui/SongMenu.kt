package dev.nori.music.app.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.ExitTransition
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.height
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
import androidx.compose.ui.unit.dp
import dev.nori.music.app.vm.ActionsViewModel
import dev.nori.music.app.vm.CardItem
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
    var picking by remember(open) { mutableStateOf(false) }
    var details by remember(open) { mutableStateOf(false) }
    var sleeping by remember(open) { mutableStateOf(false) }
    var timing by remember(open) { mutableStateOf(false) }
    NoriSheet(player?.takeIf { open && sleeping }, onDismiss) { p -> SleepChoices(p, onDismiss) }
    NoriSheet(player?.takeIf { open && timing }?.let { p -> song?.let { p to it.id } }, onDismiss) { (p, id) -> LyricsSheet(p, id) }
    NoriDialog(song?.takeIf { details }, onDismiss) { s -> TrackInfo(s, actions, onDismiss) }
    NoriDialog(song?.takeIf { picking }, onDismiss) { s -> PlaylistPicker(listOf(s), actions, onDismiss) }

    // A sheet with a half-open stage swallows the first back gesture to collapse itself, which reads
    // as the menu refusing to close. There is only ever one stage here, so back always dismisses.
    NoriSheet(song?.takeIf { !picking && !details && !sleeping && !timing }, onDismiss) { song ->
        val downloads by actions.downloads.collectAsState()
        // No stretch past the ends and no lift, here or in the other sheets: a menu that fits moved under every swipe.
        Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
            // The track leads the sheet, the way the row it came from looked.
            SheetHead(actions.cover(song.coverArt, CoverSize.ROW), song.title, remember(song) { say.songLine(song.artist, song.album) }) {
                // The album page's way of saying it, in its own case: "FLAC 16/44.1", "MP3 320 kbps".
                remember(song) { say.quality(song) }?.let { Caption(it, caps = false) }
            }
            // What someone opened this menu to do: queue it, keep it, or leave for where it came
            // from. Everything one reaches for perhaps once a month comes after it.
            // The heart comes first because it is the one thing here that is about the song rather than
            // about the queue, and it reads this session's marks rather than the snapshot the song was
            // handed over with: one starred a moment ago from the bar above still says so, without
            // waiting for the server's list to come round again. The marks are collected here rather
            // than taken from LocalStarMarks because the sheet is put up outside the provider, at the
            // top of the app, so that it outlives the row or the player that asked for it.
            val marks by actions.starMarks.collectAsState()
            val starred = marks.effectiveStar(dev.nori.music.data.StarKind.SONG, song.id, song.starred)
            val download = when (song.id) {
                in downloads.doneIds -> dev.nori.music.ffi.library.SongDownload.DONE
                in downloads.pendingIds -> dev.nori.music.ffi.library.SongDownload.PENDING
                else -> dev.nori.music.ffi.library.SongDownload.NONE
            }
            // What the menu offers and in what order is the core's (`menus::song_menu`); the words are this
            // app's (`Say.songAction`), made once when the menu opens. This draws each line with its icon.
            val remote: dev.nori.music.app.vm.RemoteViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
            val jamCan by remote.canStartJam.collectAsState()
            // A jam guest's Play next and Add to queue ask the host (ActionsViewModel).
            val jam = when {
                LocalRules.current.asks -> dev.nori.music.ffi.library.JamOffer.GUEST
                jamCan -> dev.nori.music.ffi.library.JamOffer.START
                else -> null
            }
            val rules = LocalRules.current
            val lyrics = player?.let { p -> val l by p.lyrics.collectAsState(); hasWords(l, song.id) } == true
            val items = remember(song, starred, download, player != null, lyrics, rules, jam, page) { dev.nori.music.ffi.library.songMenu(song, starred, download, player != null, lyrics, rules.stars, rules.account, rules.downloads, rules.radio, jam, page) }
            val labels = remember(items) { items.map { say.songAction(it.action) } }
            @Composable fun line(i: dev.nori.music.ffi.library.SongMenuItem, label: String, divider: Boolean) = when (val a = i.action) {
                is dev.nori.music.ffi.library.SongAction.Favourite -> Item(label, if (a.on) Kanji.Seal else Kanji.Sealed, divider = divider) { actions.star(song, a.on); onDismiss() }
                dev.nori.music.ffi.library.SongAction.PlayNext -> Item(label, Iconoir.PlaylistPlay, divider = divider) { actions.playNext(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.AddToQueue -> Item(label, Iconoir.Playlist, divider = divider) { actions.enqueue(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.AddToPlaylist -> Item(label, Iconoir.PlusCircle, divider = divider) { picking = true }
                dev.nori.music.ffi.library.SongAction.RemoveDownload -> Item(label, Iconoir.Trash, divider = divider) { actions.removeDownloads(listOf(song.id)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.StopDownload -> Item(label, Iconoir.Xmark, divider = divider) { actions.cancelDownloads(listOf(song)); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Download -> Item(label, Iconoir.Download, divider = divider) { actions.download(listOf(song)); onDismiss() }
                is dev.nori.music.ffi.library.SongAction.GoToAlbum -> { val nav = LocalNav.current; Item(label, Iconoir.CompactDisc, divider = divider) { nav.album(a.id); onDismiss() } }
                // The song's own cover stands in for a lone artist's until their page has one.
                is dev.nori.music.ffi.library.SongAction.GoToArtist -> { val nav = LocalNav.current; Item(label, Iconoir.User, divider = divider) {
                    nav.artist(a.id, Artist(a.id, a.name, song.coverArt.takeIf { song.artists.size <= 1 }, null, 0u, false, false)); onDismiss()
                } }
                dev.nori.music.ffi.library.SongAction.AddToLibrary -> Item(label, Iconoir.FolderPlus, divider = divider) { actions.addToLibrary(song.id, isAlbum = false); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Lyrics -> Item(label, Iconoir.QuoteMessage, enabled = i.enabled, divider = divider) { timing = true }
                dev.nori.music.ffi.library.SongAction.SleepTimer -> Item(label, Iconoir.HalfMoon, divider = divider) { sleeping = true }
                dev.nori.music.ffi.library.SongAction.StartRadio -> Item(label, Iconoir.AntennaSignal, divider = divider) { actions.startRadio(song); onDismiss() }
                dev.nori.music.ffi.library.SongAction.InstantMix -> Item(label, Iconoir.Sparks, divider = divider) { actions.instantMix(song); onDismiss() }
                dev.nori.music.ffi.library.SongAction.ExcludeFromMixes -> Item(label, Iconoir.Prohibition, divider = divider) { actions.excludeFromMixes(song); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Share -> Item(label, Iconoir.ShareIos, divider = divider) { actions.share(song.id); onDismiss() }
                dev.nori.music.ffi.library.SongAction.Details -> Item(label, Iconoir.InfoCircle, divider = divider) { details = true }
                // Plays the song and opens the queue, where the jam's invite is.
                dev.nori.music.ffi.library.SongAction.StartJam -> { val nav = LocalNav.current; Item(label, Iconoir.Community, divider = divider) {
                    actions.play(listOf(song)); remote.jamStart(); onDismiss(); nav.player(Panel.QUEUE)
                } }
            }
            val main = items.indices.filter { !items[it].more }
            val folded = items.indices.filter { items[it].more }
            SheetRows(head = true) {
                main.forEachIndexed { k, n -> line(items[n], labels[n], divider = k < main.lastIndex) }
                // The rest follows in full after a hairline (a jam guest's menu has none): a menu shows everything.
                if (folded.isNotEmpty()) Hairline(startIndent = GROUP_INSET)
                folded.forEachIndexed { k, n -> line(items[n], labels[n], divider = k < folded.lastIndex) }
                PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.SONG, dev.nori.music.ffi.plugins.Items.Songs(listOf(song)), onDismiss)
            }
        }
    }
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
            SheetRows(head = true) {
                if (!LocalRules.current.asks) Item(say.play, Iconoir.Play) { actions.play(card); onDismiss() }
                Item(say.menuPlayNext, Iconoir.PlaylistPlay) { actions.queue(card, next = true); onDismiss() }
                val deletes = card is CardItem.OfPlaylist && actions.editable(card.playlist)
                Item(say.addToQueue, Iconoir.Playlist, divider = deletes) { actions.queue(card, next = false); onDismiss() }
                if (deletes) Item(say.delete, Iconoir.Trash, divider = false) { deleting = (card as CardItem.OfPlaylist).playlist; onDismiss() }
                when (card) {
                    is CardItem.OfAlbum -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.ALBUM, dev.nori.music.ffi.plugins.Items.Albums(listOf(card.album)), onDismiss)
                    is CardItem.OfPlaylist -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.PLAYLIST, dev.nori.music.ffi.plugins.Items.Playlists(listOf(card.playlist)), onDismiss)
                    is CardItem.OfArtist -> PluginItems(actions, dev.nori.music.ffi.plugins.ItemKind.ARTIST, dev.nori.music.ffi.plugins.Items.Artists(listOf(card.artist)), onDismiss)
                }
            }
        }
    }
}

/** The actions plugins add to [kind]'s menu, after nori's own, each run on [items]; read before the menu opened. */
@Composable
private fun PluginItems(actions: ActionsViewModel, kind: dev.nori.music.ffi.plugins.ItemKind, items: dev.nori.music.ffi.plugins.Items, onDismiss: () -> Unit) {
    val menus by actions.pluginMenus.collectAsState()
    val entries = menus[kind].orEmpty()
    if (entries.isNotEmpty()) Hairline(startIndent = GROUP_INSET)
    entries.forEachIndexed { i, e ->
        Item(e.label, glyphIcon(e.glyph), divider = i < entries.lastIndex) { actions.runPluginMenu(e, items); onDismiss() }
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

/** The song or record a menu is about, right under the sheet's handle: its cover, its name, a line, and [more]. */
@Composable
private fun SheetHead(cover: String?, title: String, line: String, round: Boolean = false, more: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = SHEET_EDGE, end = SHEET_EDGE, bottom = 10.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
        Cover(cover, 52.dp, radius = if (round) 26.dp else 8.dp)
        Column(Modifier.weight(1f).padding(start = 12.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            more()
        }
    }
}

/**
 * A sheet's lines, on the sheet itself rather than on a card inside it: one surface, the lines parted by
 * hairlines, their glyphs [SHEET_EDGE] from the sheet's sides. Under a [head] (the song or record), a hairline first.
 */
@Composable
private fun SheetRows(head: Boolean = false, content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) {
    Column(Modifier.padding(horizontal = SHEET_EDGE - GROUP_INSET)) {
        if (head) Hairline(startIndent = GROUP_INSET)
        content()
    }
}

/** Song [id]'s lyrics as [l] has them, when it has words. */
private fun hasWords(l: dev.nori.music.data.ForSong<dev.nori.music.app.vm.Load<dev.nori.music.data.FoundLyrics>>, id: String): Boolean =
    (l.of(id, dev.nori.music.app.vm.Load.Loading) as? dev.nori.music.app.vm.Load.Ready)?.data?.lyrics?.lines?.isNotEmpty() == true

/**
 * The playing song's lyrics: where they came from, how finely they are timed, who set their timing and
 * by how much (the core's `LyricsTiming`), and a step earlier or later, kept for these words.
 */
@Composable
private fun LyricsSheet(player: dev.nori.music.app.vm.PlayerViewModel, id: String) {
    val l by player.lyrics.collectAsState()
    val found = (l.of(id, dev.nori.music.app.vm.Load.Loading) as? dev.nori.music.app.vm.Load.Ready)?.data ?: return
    val lyrics = found.lyrics
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
        SectionTitle(say.lyrics)
        Text(
            remember(found.source, lyrics.synced, lyrics.wordTimed) { say.lyricsSource(found.source, lyrics.synced, lyrics.wordTimed) },
            Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), style = MaterialTheme.typography.bodyLarge,
        )
        if (!lyrics.synced) return@Column
        Text(
            remember(lyrics.timing) { say.lyricsTiming(lyrics.timing) },
            Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // Centred, the value in a slot of its own width so − and + stay put as it changes.
        Row(
            Modifier.fillMaxWidth().padding(vertical = 8.dp),
            horizontalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
        ) {
            TextAction("−", { player.nudgeLyrics(id, -1) })
            Text(
                say.lyricsShift(dev.nori.music.ffi.lyrics.lyricsOffsetMs(lyrics.timing)),
                Modifier.widthIn(min = 96.dp), style = MaterialTheme.typography.titleMedium,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
            TextAction("+", { player.nudgeLyrics(id, 1) })
        }
        if (lyrics.timing is dev.nori.music.ffi.model.LyricsTiming.Yours) {
            Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.Center) {
                TextAction(say.reset, { player.nudgeLyrics(id, 0) })
            }
        }
    }
}

/** The sleep choices on their own sheet, so the song's menu is not buried under eleven of them. */
@Composable
private fun SleepChoices(player: dev.nori.music.app.vm.PlayerViewModel, onDone: () -> Unit) {
    val state by player.state.collectAsState()
    Column(Modifier.nestedScroll(KeepUpwardDrags).verticalScroll(rememberScrollState(), overscrollEffect = null).windowInsetsPadding(gestureBarRoom)) {
        SectionTitle(say.sleepTimer)
        val running = state.sleepAt > 0 || state.sleepAtEndOfTrack
        // The choices are the core's (`menus::sleep_choices`); "Off" is all zeros.
        val choices = remember(running) { dev.nori.music.ffi.library.sleepChoices(running) }
        val words = remember(choices) { choices.map(say::sleepChoice) }
        SheetRows {
            choices.forEachIndexed { n, c -> Item(words[n], divider = n < choices.lastIndex) { player.sleep(c.minutes.toInt(), c.endOfTrack, c.songs.toInt()); onDone() } }
        }
    }
}

@Composable
fun PlaylistPicker(songs: List<Song>, actions: ActionsViewModel, onDone: () -> Unit) {
    var playlists by remember { mutableStateOf<List<Playlist>?>(null) }
    var name by remember { mutableStateOf("") }
    LaunchedEffect(Unit) { playlists = actions.playlists() }
    AlertCard(
        title = { Text(say.addToPlaylist) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                FormField(name, { name = it }, Modifier.fillMaxWidth(), label = { Text(say.newPlaylist) }, singleLine = true)
                playlists?.forEach { p -> Text(p.name, Modifier.fillMaxWidth().clickable { actions.addToPlaylist(p, songs); onDone() }.padding(vertical = 12.dp)) }
            }
        },
        confirmButton = { TextAction(say.create, { actions.addToNewPlaylist(name.trim(), songs); onDone() }, enabled = name.isNotBlank()) },
        dismissButton = { TextAction(say.cancel, onDone) },
    )
}

/** Everything the server said about one file, then what plugins add, each under its name, a line with a link opening it. */
@Composable
fun TrackInfo(song: Song, actions: ActionsViewModel, onDone: () -> Unit) {
    // Which rows there are, in what order and how each reads is nori-core's (`fmt::track_info`).
    val rows = remember(song) { say.trackInfo(song) }
    val groups by androidx.compose.runtime.produceState(emptyList<dev.nori.music.ffi.plugins.InfoGroup>(), song) { value = actions.pluginSongInfo(song) }
    val uri = androidx.compose.ui.platform.LocalUriHandler.current
    AlertCard(
        title = { Text(say.details) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                rows.forEach { (label, value) -> Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant); Text(value, Modifier.padding(bottom = 8.dp)) }
                // Asked as the sheet opens: the plugins' lines come in under nori's own, never in place of them.
                AnimatedVisibility(groups.isNotEmpty(), enter = AppMotion.grow(), exit = ExitTransition.None) {
                    Column {
                        groups.forEach { g ->
                            Text(g.name, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            g.lines.forEach { l ->
                                val link = l.link
                                if (link == null) Text(l.words, Modifier.padding(bottom = 4.dp))
                                else Text(l.words, Modifier.padding(bottom = 4.dp).clickable { runCatching { uri.openUri(link) } }, textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline)
                            }
                            androidx.compose.foundation.layout.Spacer(Modifier.height(4.dp))
                        }
                    }
                }
            }
        },
        confirmButton = { TextAction(say.close, onDone) },
    )
}

/** Shown instead of nothing while songs are selected: the batch actions. */
@Composable
fun SelectionBar(actions: ActionsViewModel) {
    val selection by actions.selection.collectAsState()
    // Above the early return, so the picker can leave after the selection it cleared is gone.
    var picking by remember { mutableStateOf(false) }
    NoriDialog(selection.takeIf { picking && it.isNotEmpty() }, { picking = false }) { songs ->
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

/** How far a menu sheet's head and lines stand in from its sides. */
private val SHEET_EDGE = 16.dp
/** A menu sheet's line: Android's 48 dp touch target, so a long menu fits the screen. */
private val SHEET_ROW = 48.dp
