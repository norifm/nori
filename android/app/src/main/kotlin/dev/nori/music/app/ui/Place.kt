package dev.nori.music.app.ui

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.findRootCoordinates

// Compose's positionInRoot, positionInWindow and localToWindow read an offset it caches per layout node, and
// that offset can be wrong: for a duet's right-hand lyric line it left out the lane the line's row is padded
// by (53 px where the line was drawn at 179), so the line's island drew it a lane to the left. These walk
// every layout step from the root instead, as boundsInWindow does.

/** Where this is in the root, its top left. */
fun LayoutCoordinates.placeInRoot(): Offset = findRootCoordinates().localPositionOf(this, Offset.Zero)

/** Where this is in the window, its top left. */
fun LayoutCoordinates.placeInWindow(): Offset = findRootCoordinates().let { it.localToWindow(it.localPositionOf(this, Offset.Zero)) }
