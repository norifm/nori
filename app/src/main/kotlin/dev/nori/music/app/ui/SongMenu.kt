package dev.nori.music.app.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.nori.music.app.vm.ActionsViewModel
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
    GroupRow(text, if (enabled) onClick else null, Modifier.alpha(if (enabled) 1f else 0.38f), icon = icon, divider = divider)
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
    NoriDialog(song?.takeIf { details }, onDismiss) { s -> TrackInfo(s, onDismiss) }
    NoriDialog(song?.takeIf { picking }, onDismiss) { s -> PlaylistPicker(listOf(s), actions, onDismiss) }

    // A sheet with a half-open stage swallows the first back gesture to collapse itself, which reads
    // as the menu refusing to close. There is only ever one stage here, so back always dismisses.
    NoriSheet(song?.takeIf { !picking && !details && !sleeping && !timing }, onDismiss) { song ->
        val downloads by actions.downloads.collectAsState()
        var more by remember { mutableStateOf(false) }
        Column(Modifier.verticalScroll(rememberScrollState()).windowInsetsPadding(gestureBarRoom)) {
            // The track leads the sheet, the way the row it came from looked.
            Row(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 4.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Cover(actions.cover(song.coverArt, CoverSize.ROW), 52.dp, radius = 8.dp)
                Column(Modifier.weight(1f).padding(start = 12.dp)) {
                    Text(song.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    Text(remember(song) { say.songLine(song.artist, song.album) }, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    if (song.suffix.isNotEmpty()) Caption(remember(song) { say.songFormat(song) })
                }
            }
            // What someone opened this menu to do: queue it, keep it, or leave for where it came
            // from. Everything one reaches for perhaps once a month waits under "More" below.
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
            val items = remember(song, starred, download, player != null, lyrics, rules, jam) { dev.nori.music.ffi.library.songMenu(song, starred, download, player != null, lyrics, rules.stars, rules.account, rules.downloads, rules.radio, jam) }
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
                dev.nori.music.ffi.library.SongAction.Lyrics -> Item(label, Iconoir.EmojiSingRightNote, enabled = i.enabled, divider = divider) { timing = true }
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
            GroupCard(Modifier.padding(top = 8.dp)) {
                main.forEachIndexed { k, n -> line(items[n], labels[n], divider = k < main.lastIndex || folded.isNotEmpty()) }
                // A jam guest's menu has nothing under "More".
                if (folded.isNotEmpty()) {
                    val ms = if (reduceMotion()) 0 else 220
                    val turn by animateFloatAsState(if (more) 180f else 0f, animationSpec = tween(ms), label = "more")
                    GroupRow(
                        say.more, { more = !more }, icon = Iconoir.MoreHoriz, divider = more,
                        // The chevron turns over rather than swapping for its opposite, so the row says which
                        // way it is about to move before it moves.
                        trailing = { Icon(Iconoir.NavArrowDown, null, Modifier.size(22.dp).graphicsLayer { rotationZ = turn }, tint = MaterialTheme.colorScheme.onSurfaceVariant) },
                    )
                    AnimatedVisibility(
                        more,
                        enter = if (ms == 0) EnterTransition.None else fadeIn(tween(ms)) + expandVertically(tween(ms)),
                        exit = if (ms == 0) ExitTransition.None else fadeOut(tween(ms)) + shrinkVertically(tween(ms)),
                    ) {
                        Column { folded.forEachIndexed { k, n -> line(items[n], labels[n], divider = k < folded.lastIndex) } }
                    }
                }
            }
        }
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
    Column(Modifier.verticalScroll(rememberScrollState()).windowInsetsPadding(gestureBarRoom)) {
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
    Column(Modifier.verticalScroll(rememberScrollState()).windowInsetsPadding(gestureBarRoom)) {
        SectionTitle(say.sleepTimer)
        val running = state.sleepAt > 0 || state.sleepAtEndOfTrack
        // The choices are the core's (`menus::sleep_choices`); "Off" is all zeros.
        val choices = remember(running) { dev.nori.music.ffi.library.sleepChoices(running) }
        val words = remember(choices) { choices.map(say::sleepChoice) }
        GroupCard {
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

/** Everything the server said about one file. */
@Composable
fun TrackInfo(song: Song, onDone: () -> Unit) {
    // Which rows there are, in what order and how each reads is nori-core's (`fmt::track_info`).
    val rows = remember(song) { say.trackInfo(song) }
    AlertCard(
        title = { Text(say.details) },
        text = { Column(Modifier.verticalScroll(rememberScrollState())) { rows.forEach { (label, value) -> Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant); Text(value, Modifier.padding(bottom = 8.dp)) } } },
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
