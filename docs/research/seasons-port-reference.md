# Seasons on Android: notes for porting

Temporary. Delete once the other clients have caught up.

This lists what changed on Android in the `seasons-android` branch, so the desktop, web, terminal and
iPod clients can be brought in line. Android is the reference: if another client does something
differently from what's written here (an option, a name, an order, a behavior), the other client
changes, not Android. Where something is marked "core", the logic already lives in Rust and a client
only has to call it.

## Using Android as the reference

Don't port from this file alone. It's a map; the app is the source of truth.

- Run it. `./gradlew :app:installDebug -PrustTargets=arm64-v8a` on the emulator, with the local server
  from `tools/dev-server.sh`. `tools/app.sh open <route>` jumps to a screen (`home`, `library`, `player`,
  `settings/look`, ...) and `tools/app.sh play` starts something, so the same state can be set up on
  both clients.
- Compare screens side by side: take `adb exec-out screencap -p` of the Android screen and a screenshot
  of the client being worked on, at the same state. For anything that moves (the player opening,
  panels dissolving), record it with `adb shell screenrecord` and step through the frames.
- Read the Compose code for the screen before writing the port. Most of it is in
  `app/src/main/kotlin/dev/nori/music/app/ui/` (`HomeScreen.kt`, `LibraryScreen.kt`, `HeroPage.kt`,
  `PlayerScreen.kt`, `SettingsScreen.kt`, `Design.kt` for tokens). The comments explain why things are
  the way they are, which usually matters more than the numbers.
- Take words from `app/src/main/res/values/strings_ui.xml` and `strings.xml`, and put them in the
  client's own word file (`words.rs` on the desktop, `text.rs` in the terminal). Same text, same case.
- If Android gets a list, an order or a decision from the core, call the same function. If you find
  logic in Kotlin that another client would need too, move it to Rust first rather than copying it.
- When done, check the screen against Android again, including Day, Night and Midnight and at least
  two seasons.

## Ground rules

- Appearance has four modes: System, Day, Night, Midnight. Midnight is true black and has no cover
  colors. There's no separate cover colors or pure black setting.
- Seasons are Spring, Summer, Autumn and Winter (春 夏 秋 冬), picked from four tiles that show each one's
  paper, accent and on colors.
- Pages are paper, never plain white. Five roles carry a theme: page, surface, ink, accent, on. Other
  greys are fixed shares of ink. Chrome has no shadows; a step in paper and a hairline separate things.
- The accent marks only what's "yours, now": the current tab, the playing song, shuffle or repeat when
  on, a liked seal, a focused field. A selected row is raised, not filled with the accent.
- A favorite is the seal 好. Not a heart, not a star.
- Inter everywhere; Literata only for page titles.

## Sizes

| what | size |
|---|---|
| corner radius: cover, card, tile, sheet, pill | 8, 12, 18, 26 dp, fully round |
| side margin | 20 dp |
| gaps: tight, row, section | 8, 12, 26 dp |
| title rising into the hero artwork | 34 dp |
| grid: narrowest card, gap | 170, 12 dp |
| mix card | 150 x 206 dp |
| song row cover | 44 dp, with the hairline starting at the title |
| icons in grouped rows (library places, settings categories) | 20 dp |
| tab bar icons | 21 dp |
| back arrow and toolbar icons | 22 dp (the selection bar's close is 20) |
| player lyrics and queue buttons | 24 dp |
| player output line | 22 dp icon, 15 sp text |
| player seal and more buttons | 28 dp in a 44 dp target |

Home row cards, full views of a row and an artist's "see all" grid all use the same card size.

## Icons

Icons are Iconoir (MIT). `tools/glyphs.py` holds the list and generates Android's `Iconoir.kt` plus the
widget and car drawables; other clients should use the same glyph names from it. Line icons are 1.5
wide with round ends. Filled icons are only used for the transport (play, pause, rewind, forward).
Material icons are gone.

The ones most likely to be needed: `home-simple`, `album-list`, `settings`, `search`,
`emoji-sing-right-note` (lyrics), `playlist` (queue), `shuffle`, `repeat`, `repeat-once`, `more-horiz`,
`xmark`, `arrow-left`, `nav-arrow-right`, `clock-rotate-right` (history), `download`, `sparks` (smart
playlists), `antenna-signal` (radio). Devices: `smartphone-device`, `computer`, `terminal`, `user` for a
guest.

The output button shows what's actually playing. The core's `output_look` gives a glyph: phone speaker
is `smartphone-device`, wired and USB are `headset`, Bluetooth is `bluetooth`, anything else is
`airplay`. When another nori device is playing, show that device's icon instead, lit.

Device icons are redrawn by `tools/glyphs.py` to the seal's height (the seal's frame runs from 3 to 21 in
the 24 grid) with the line width left at 1.5, so next to the seal in the mini player they're the same
height and weight. Use those generated paths, not Iconoir's originals.

A button's tap area is the whole button (48 dp minimum on touch), never just the drawn outline.

## Screens

### Home

The Continue card is tinted with its cover and has Resume and a close button; swiping it away also
dismisses it. Once a queue has been resumed, closed or swiped away it isn't offered again, even after a
restart, until a different one shows up (core: `queue_offer`, `queue_offer_done`).

For you cards sit on the season's paper tinted with the mix (core: `mix_card_colours`), with a 2x2
cover mosaic fading into the card. Once the first cover's colors are known the card uses those, so it
matches the mix page it opens.

Every row title has a small arrow that opens the whole row as a grid.

### Library

A card at the top holds Favorites, Downloads (with its size), History and Shuffle. Sections are chips.
The Albums page's sort options are pills below the top card.

Pills start with Favorites, as a plain word with no star or icon. Pages that share a choice use the
same word for it in the same place:

- Albums: Favorites, Title, Artist, Year, Added, Most played, Random (core: `album_sorts`). Artist
  groups albums into shelves per artist. There's no Played option anymore.
- Songs: Favorites (a filter), Title, Artist, Album, Year, Added, Most played, Longest.

Artists have no A-Z index. Playlist lists reload after a playlist is created, added to or deleted,
including Home's Playlists row.

### Album, playlist, artist and mix pages

The cover runs to the edges and melts into the page over its bottom 38%, and the title rises 34 dp into
it. Pages without an artist line put the title a bit lower. The page uses the season's paper tinted with
the cover (not in Midnight).

The Play pill is in the page's ink on its paper. Other buttons are plain glyphs without backgrounds.
The info line ("2019 · 8 songs · 36:28 · FLAC") and its glyphs (download, add to queue, a mix's
Refresh) sit at the head of the list, and come from the core (`page_actions`). There's no caption under
the title. Disc headings keep normal row spacing.

On an artist page, each shelf (Albums, Singles, ...) has the arrow, which opens that group as a grid.

A mix page has Save as playlist next to Play. The playlist is called "nori <mix name>", or "nori <mix
name> 2" if that's taken (core: `saved_mix_name`). Refresh is in the list head.

### Player

From top to bottom: the title row with the seal and more, the seek bar, then lyrics, previous,
play/pause, next and queue, then the output line with the device icon and name ("Phone speaker") as one
button. The mini player has the same device icon, the seal and play/pause.

The artwork reaches down behind the title and melts into the page. Behind the title and controls it's
blurred.

Opening and closing: one cover moves between the mini player's thumbnail and the full artwork; there's
never a second copy. Whatever part of it is behind the title and controls stays blurred the whole time,
and only the part above them is sharp. The corners round off as it gets near the thumbnail. There's no
dark strip at its bottom.

Switching between artwork, lyrics and queue is a single dissolve, with the cover moving into the lyrics
thumbnail and back.

Dragging the open player only takes downward drags, so a tap that slips upward still presses the button
under the finger.

In landscape the artwork is the left half, with a short soft edge on its right.

### Sheets, dialogs and menus

Sheets and dialogs are opaque paper with a hairline edge, with buttons in ink. Every menu uses the same
popover. The song menu's rows are grouped. The selection bar is an outlined pill. The tab bar and the
mini player are one outlined pill each, with the page blurred inside them and content scrolling
underneath. The update banner is an outlined card without a shadow.

### Settings

Settings are grouped cards with small-caps labels. Each category has a 20 dp icon and a one-line
summary. Appearance shows the four modes, then the season tiles. Chip settings have the same space
above and below their pills. The storage bar uses shades of the page's ink. Playlists can no longer be
exported as M3U (importing still works).
