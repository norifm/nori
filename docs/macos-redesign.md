# The Mac client, redrawn from Android

Internal planning note, not published. The terminal client follows once the Mac one is done.

The Mac app today is laid out like Apple Music (a sidebar, a glass player capsule, system Liquid Glass,
Apple red). The goal is the opposite: **the Mac app is Android's upright UI made wide**. Same chrome, same
outline, same blur, same accent rules, same motion, with the Mac's extra room used for the things Android
has no space for (shuffle, repeat and volume on the bar, the player's panel beside the cover).

The mockup is `~/Desktop/nori-macos-mockup.html` (kept with the owner's other design pages; copy it into
`docs/` if it should live with the code). It is the reference for layout; this note is the reference for
behavior.

## 1. What changes, at a glance

| Area | Mac today | Mac after |
|---|---|---|
| Navigation | Sidebar (search, home, library, playlists, settings, account) | No sidebar. One outlined pill along the top |
| Player | Glass capsule, 54 px, 720 px wide | One outlined pill, wider, same outline and blur as the top pill |
| Blur | System Liquid Glass sidebar window plus a glass shader for the player | One blur of the page, drawn inside both pills, as Android does |
| Accent | A fixed red (`#fa2d48`) or a picked swatch | The season theme's accent, made to read on any page, used only for "you, now" |
| Theme | Dark grey ground, white at fixed alphas | The four season palettes in light, dark and AMOLED, blended around the calendar's turns |
| Favorite | A heart | The seal 好 (an outline, upright, stamped and tilted when liked) |
| Play / pause | A filled circle | The bare glyph; the circle only appears as a press ripple |
| Fades | Hard edges where content meets chrome | Content fades out before the pills |

## 2. The design language to mirror

Everything below already exists on Android; the Mac app copies the rule, not the code. The shared parts
live in Rust (`crates/look`: `theme.rs`, `dress.rs`, `compose.rs`, `cover.rs`) and the Mac app must call
them rather than re-deriving anything.

### Paper, not a UI toolkit
- A page is a **paper** tinted by the season, never white or neutral grey. Dark mode is the same paper at
  night. `Season::palette` has the five roles: page, surface, ink, accent, on.
- Surfaces are one quiet step off the page (sheets, grouped rows, popovers); lifted states veil the ink
  over them (`roles()`: 5 %, 9 %, 12 %), they do not use new greys.
- Text is the ink at fixed alphas of the page (`veil`): secondary 65 %, outline 36 %, hairline 14 %.
- Headings are a serif (Georgia-like, the Android app's display face); body is the system sans. Titles are
  large and tight.
- Corners are generous: 12 on covers, 18 to 22 on cards and sheets, a full pill on the chrome.

### One outline
- The chrome (tabs, player) is **one outline**, not a bar: the page blurred inside it, a hairline round it,
  no shadow, no Material indicator, nothing past its edge (`Pill` in `Chrome.kt`).
- The hairline is the page's ink at about 20 % (`CHROME_OUTLINE`). Inside it, a hairline between the player
  row and the tab row at 45 % of that.
- The chrome takes the touches that land on its margins (`keepsTouches`), so a click beside a pill never
  opens the item under it.

### Blur
- The page is recorded once into a layer (Android: `Backdrop`). Each pill draws that layer, blurred, at its
  own place, with the **page's color over it at 55 %** (`BLURRED_TINT`) and the blur at **24 dp**
  (`BACKDROP_BLUR`). Where there is no blur the tint is thicker, not absent.
- The blur is worked out again only when the page under it is drawn again. It is never animated.
- Content scrolls **under** the chrome and fades out before it: `edgeFade`, `EDGE_REACH` 24 dp, a ramp from
  the page's colour to clear, starting late enough that the glass shows through the pill's margins before
  the content is gone. Items at rest are untouched; only what is under the fade changes.
- On the sides of the window and at rest nothing is blurred; blur only lives inside the two pills and the
  player's sleeve wash (see `docs/player-colour-band.md`).

### Accent
- There is **one accent**: the theme's (`Palette::accent`), and it means "you, now": the current tab, the
  song playing, a lit toggle (shuffle, repeat), a liked seal, a focused field. It is never a decoration.
- The second color (`on`) is only for a switch that is on.
- On a page with its own cover (an album, an artist, the player) the page wears the cover's colors, but the
  accent stays the theme's, **moved in its own hue until it reads as text there** (`accent_on`,
  `dress::accented`), else the page's ink. Pages with no cover wear the theme's page.
- The chrome follows the page it sits on, but only a page that is *about* one cover wears that cover. Playing
  a record must not turn the whole app red.

### Seasons
- Four themes, selectable or following the calendar. Names stay Spring, Summer, Autumn, Winter; the picker
  marks them 春 夏 秋 冬.
- The calendar turns on 4 Feb, 5 May, 7 Aug and 7 Nov (half a year round in the south). Each season blends
  in from a week before its date to a week after it, over 14 days (`coming_in`, `BLEND_DAYS`).
- Light, dark and AMOLED (black page, surface a third of the way down) are one rule applied to each palette.

### Icons
- The Mac app draws its own hand-made stroke icons (`Icons` in `app.slint`: thin outlines, sharp joins).
  Android uses Material's **Rounded** (and a few **Filled**) icon set, plus nori's own seal and season marks.
  The Mac app switches to the same set, so the same action has the same shape on both:
  - Transport: `PlayArrow`, `Pause`, `SkipNext`, `SkipPrevious`, `Shuffle`, `Repeat` / `RepeatOne`, all
    Rounded and filled shapes, not outlines.
  - Tabs: `Home`, `LibraryMusic`, `Search` and `Settings` (Filled, as the Android tabs), and the library
    sections' icons from `LibraryScreen.sectionIcon` (`Album`, `Person`, `MusicNote`, `QueueMusic`) with
    the seal for Favorites.
  - Player: lyrics, queue and the speaker/output glyphs as on the player screen; the speaker is the
    outlined one while this computer plays and filled while another device plays.
  - Sizes follow Android's: 26 on the bar's play, 24 on its other glyphs, 20 inside plates.
- The paths come from the Material icon set's SVGs, converted once into the Slint `Icons` global; the seal
  is core's `seal.xml` path. Nothing else is drawn by hand.

### Fonts
- Android uses two typefaces and nothing else: **Inter** (variable, weights 400 to 700, real instances,
  never synthetic bold) for all text, and **Literata** (display cut, weights 500 to 800) only for what a
  page is called (`titled()`: tracked at -0.3 sp). Headings and titles use negative tracking (-0.2 to -1.5
  sp by size); body is plain; captions are small and grey.
- The Mac app today uses the system font (SF Pro) on macOS and Inter elsewhere. It switches to the bundled
  **Inter everywhere** and adds **Literata** for page titles ("Listen now", Albums, the player's title), so
  the two clients are set in the same type. The files are Android's own (`app/src/main/res/font/inter.ttf`,
  `literata.ttf`), shared rather than copied: one asset path for both clients.
- Sizes follow Android's scale (body 15, titles 20 to 34), scaled once for the window's density.
- The terminal cannot choose its font; it keeps the weights (bold titles, quiet captions) and nothing else.

### The seal
- A favorite is a seal: a square chop with 好 cut out. **Not liked: its outline, upright, in the chrome's
  quiet ink. Liked: filled with the accent, stamped at a slight tilt (about -5 degrees)**, landing from half
  as big again and settling on a firm spring (`Stamp`: scale 1.5 to 1, damping 0.5, stiffness 700). Taking
  a like off is immediate. A seal that changes by itself (another song, another device) is drawn, not
  stamped.
- On the Mac the stamp has no haptic; the click's sound is the system's. Reduced motion draws it directly.

### Play, pause and the press
- Play and pause are the **bare glyph**, 26 dp on the bar and large in the player. There is no filled circle.
  A circle exists only as the press feedback: a round ripple in the ink at about 16 % that grows from half
  size and fades out in about 400 ms. Every transport button shows it.
- The glyph changes by a 180 ms cross-fade with a scale from 0.8 (`PlayPauseGlyph`); a spinner only after
  300 ms of waiting, and the wait counts as "playing" until then so a skip never flashes "paused".

### Motion
- **Nothing animates unless the user touched it.** No idle motion, no ticking, no looping effects. The
  seek bar updates only while the player is on screen. Reduced motion turns every flourish into a direct
  change.
- Pages move like a stack: a push slides the new page in from the right, opaque, over the old one, which
  moves a third of the way left under a scrim; a pop is the reverse (`docs/motion.md`, item 1). Tab roots
  cross-fade.
- The player is a sheet rising from the bar, not a window: it covers the chrome as one piece.
- The chrome changes color with the page, one cross-fade of 340 ms (`CHROME_FADE_MS`), shared by the bar and
  tabs; it skips the page in between when pages change quickly.
- Season changes are the calendar's, not an animation.

## 3. The layout

### Top pill
Home, Albums, Artists, Songs, Playlists and Favorites, centered, one outline, icons plus labels; the current
tab is named and lit in the accent, the others are in the quiet ink. A hairline separates Home from the
library's sections. Playlists opens the user's playlists in a popover under it (this replaces the sidebar's
playlist list). On the left: the traffic lights and, later, back and forward. On the right: the **search**
field (it widens on focus and shows results in a popover under it) and the gear. The gear's menu holds the
account (name, server), the season and appearance, the equalizer, other devices and settings. A jam
guest's top pill drops what a guest cannot use (Playlists), as the sidebar does today.

### Player pill
Along the bottom, centered, up to about 1040 px, the same outline and blur as the top pill.
- **Left:** the cover and what is playing (title, artist and album, each a link as today). A click, or the
  upward gesture on a trackpad, opens the player.
- **Middle:** shuffle, previous, play or pause, next, repeat, with the seek bar and both times under them.
  Shuffle and repeat are lit in the accent when on; repeat shows a 1 for repeat-one.
- **Right, in this order:** the seal, the speaker (outputs and other devices; filled while another device
  plays), lyrics, queue, and the volume slider. A jam on stands in the speaker's place. A jam guest's
  shuffle, repeat and volume are the host's and are hidden.

### The player
Android's upright player made wide: the cover, title with its seal, the seek bar, the transport row with
shuffle and repeat, and a volume slider on the left; the panel beside it with Lyrics, Queue and Devices
as tabs. Pressing a panel's button again returns to the artwork. The page behind wears the cover's
colors, with the sleeve melting into the page as on Android.

## 4. Work, in order

1. **Theme in the client.** Call `nori_look::palette`, `roles` and the `dress` tables for every colour; drop
   `Theme.accent`'s fixed red, the swatch picker and the Music-measured greys in `app.slint`. The accent
   setting goes (the season replaces it); the season setting and the follow-the-calendar default come
   from the existing core settings.
2. **Icons and fonts.** Convert Material's Rounded and Filled glyphs into the `Icons` global, bundle Inter and
   Literata from Android's `res/font`, set the sizes and tracking above. Visible everywhere, moves nothing.
3. **Chrome and layout.** Remove `SidebarContent`, `SidebarWindow`, `native-sidebar` and the traffic-lights
   strip; add the top pill; rebuild the player capsule as the player pill with the layout above.
4. **Blur.** Replace the per-window glass (system Liquid Glass sidebar, `glass.wgsl` capsule) with the
   Android scheme: one page layer, blurred at 24 logical px under each pill's rect, tinted by the page's
   colour at 55 %, a 1 px outline in `CHROME_OUTLINE`. `compositor.rs` already renders the page into its
   own layer and has `blur_page`; the work is to blur it once per page frame and to clip it to the pills,
   and to delete the sidebar layer. Measure on the host before adding anything per-frame (AGENTS.md).
5. **Edge fades** under both pills with the Android constants, and the late start so the glass is visible in
   the pill's margins.
6. **The seal and the press.** Add the seal drawable (core's `seal.xml` as a path), the stamp animation
   and the ripple to every icon button; remove the filled play circle.
7. **Pages.** Home as "Listen now" with the continue card, Albums, Artists, Songs, Playlists and Favorites
   as the library sections on Android (grids and lists the same width rules: 170 dp minimum tiles, 12 dp
   gaps), then the player and its panels.
8. **Motion.** Page push and pop, the sheet, the chrome cross-fade; reduced motion respected.
9. **Settings page.** The season picker (tiles with 春 夏 秋 冬), light, dark and AMOLED.

Each step ships on its own: 1 first (it changes every colour without moving anything), then 3 and 4
together (the sidebar cannot go without the new blur), then the rest.

## 5. Constraints

- Every decision stays in Rust; Slint draws. No palette or accent rule is written twice.
- Battery first: the blur is recomputed only when the page behind a pill is redrawn, never per frame while
  idle; nothing ticks while music plays with the window hidden.
- Linux keeps working: where there is no compositor blur the pills use the thicker tint, as Android does
  without blur.
- A jam guest, remote control, lyrics, the equalizer and the queue keep every feature they have now; they
  move, they do not go.

## 6. Open questions

- The right cluster's order is speaker/lyrics/queue/volume with the seal first (to its left); the seal
  can go beside the title instead if it reads better on the bar.
- Whether Albums, Artists, Songs, Playlists and Favorites all sit in the top pill, or only Home and Library
  do with a section switcher inside the page, as Android has. The mockup shows them all.
- Trackpad gestures for the player sheet (an upward swipe on the bar) and for back (two-finger swipe).
