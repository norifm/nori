package dev.nori.music.app.ui

import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel

/** Which shelves Home has, and in what order (Settings, Appearance). */
@Composable
fun HomeRows(settings: SettingsViewModel) {
    val prefs by settings.prefs.collectAsStateWithLifecycle()
    // Which rows are left out, and where one switched on or off goes, are the core's (browse.rs).
    RowEditor(
        say.homeRowsTitle, prefs.homeRows, remember(prefs.homeRows) { dev.nori.music.ffi.library.homeRowsHidden(prefs.homeRows) },
        key = { it.name }, label = say::homeRow,
        onToggle = { row, on -> settings.update { p -> p.copy(homeRows = dev.nori.music.ffi.library.homeRowsToggled(p.homeRows, row, on)) } },
        onMove = settings::moveHomeRow,
    )
}

/**
 * A page's rows to show, hide and order: [shown] in their order, each with its switch, then the [hidden]
 * ones. A row is picked up by holding it anywhere, not by a handle at its edge - there is nothing else
 * to do to a row here, so the whole row may as well be the grip. A long press is what starts it, because
 * the list itself scrolls and a plain drag could not mean both.
 */
@Composable
private fun <T> RowEditor(
    title: String, shown: List<T>, hidden: List<T>, key: (T) -> String, label: (T) -> String,
    onToggle: (T, Boolean) -> Unit, onMove: (Int, Int) -> Unit,
) {
    // Tracked by which row is being held, never by its position: the position changes the instant the
    // list reorders, and a gesture keyed on that is cancelled mid-drag - which is why a row could only
    // be moved one place per press. The offset keeps the held row under the finger while the rest slide
    // past it, so the gap follows the finger instead of the row snapping away from it.
    var held by remember { mutableStateOf<T?>(null) }
    var dragOffset by remember { mutableFloatStateOf(0f) }
    var rowHeight by remember { mutableFloatStateOf(0f) }
    // The list as it is now, for a drag that began when it was another order.
    val order by rememberUpdatedState(shown)
    val haptics = androidx.compose.ui.platform.LocalHapticFeedback.current

    Column(Modifier.fillMaxSize()) {
        LargeTitle(title)
        Caption(say.holdARowToMoveIt, Modifier.padding(start = Space.gutter, bottom = 8.dp))
        val scroll = rememberLazyListState()
        LazyColumn(Modifier.fadesUnderTitle(scroll), scroll, contentPadding = PaddingValues(bottom = LocalChromeInset.current)) {
            items(shown, key = key) { row ->
                val dragged = held == row
                Row(
                    Modifier.fillMaxWidth()
                        .zIndex(if (dragged) 1f else 0f)
                        .graphicsLayer {
                            if (dragged) { translationY = dragOffset; shadowElevation = 14f; scaleX = 1.02f; scaleY = 1.02f }
                        }
                        .onGloballyPositioned { if (rowHeight == 0f) rowHeight = it.size.height.toFloat() }
                        // Keyed on the row, which never changes, so one press can carry it the whole way.
                        .pointerInput(row) {
                            detectDragGesturesAfterLongPress(
                                onDragStart = {
                                    held = row; dragOffset = 0f
                                    haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                                },
                                onDragEnd = { held = null; dragOffset = 0f },
                                onDragCancel = { held = null; dragOffset = 0f },
                            ) { change, drag ->
                                change.consume()
                                dragOffset += drag.y
                                val h = rowHeight.takeIf { it > 0f } ?: return@detectDragGesturesAfterLongPress
                                var at = order.indexOf(row)
                                while (dragOffset >= h && at < order.lastIndex) {
                                    onMove(at, at + 1); at++; dragOffset -= h
                                    haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                                }
                                while (dragOffset <= -h && at > 0) {
                                    onMove(at, at - 1); at--; dragOffset += h
                                    haptics.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                                }
                            }
                        }
                        .padding(horizontal = Space.gutter, vertical = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(label(row), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                    // Turning a row off leaves it in the list below rather than taking it away, so it is
                    // clear where it went and how to have it back.
                    NoriSwitch(true, { _ -> onToggle(row, false) })
                }
                Hairline()
            }
            if (hidden.isNotEmpty()) {
                item(key = "hidden") { SectionTitle(say.notShown) }
                items(hidden, key = key) { row ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(label(row), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        // It comes back at the end of the list, where it can be seen, and can be carried up from there.
                        NoriSwitch(false, { _ -> onToggle(row, true) })
                    }
                    Hairline()
                }
            }
        }
    }
}
