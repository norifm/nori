# Mac client redesign: checklist

Client: `crates/desktop` (Slint + wgpu/Skia, `crates/desktop/ui/app.slint`, `src/compositor.rs`, `src/app.rs`).
Contract: `docs/redesign-contract.md`. Long form and rationale: `docs/macos-redesign.md`. Mockup (owner-approved,
open it for pixel intent): `~/Desktop/nori-macos-mockup.html`.
Branch/worktree: `feat/macos-redesign` in `~/Documents/Projects/Apps/nori-macos`.
Goal: the Mac app is Android's upright UI made wide. Sidebar gone, one outlined top pill, one outlined player
pill, season palettes, the seal, bare play glyph with ripple, Material icons, Inter + Literata, blur like Android.

## 0. Setup
- [x] Read AGENTS.md, the contract, `docs/macos-redesign.md`, and the mockup. Run `cargo run -p nori-desktop` once
      (log in to the test server as the README says; `tools/dev-server.sh` gives a local Navidrome) and screenshot
      the current app to know the starting point.
- [x] List every screen/overlay in `app.slint` (views 0-11, queue, lyrics, devices, jam, equalizer, settings,
      login, popovers, menus) in a "Screens" table at the bottom of this file with a status column.

## 1. Theme from Rust (C1-C5)
- [x] Add a small shim in `crates/desktop/src/` that asks `nori_look` for the palette (`palette(season, month, day,
      southern, dark, amoled)`, `roles`) and sets a Slint global `Palette` (page, surface, ink, accent, on, quiet
      ink, outline, hairline, glass tint). Drive it from the `season` and `theme` settings; recompute when
      settings change and once at the start of the day (no ticking timer: recompute on window focus and on resume).
- [x] Replace `Theme.ground/text/dim/faint/hairline/hover/chosen/fill/panel/accent` (the Music-measured greys,
      `#fa2d48`) with roles derived from `Palette`; keep the property names where used widely, change their source.
- [x] Remove the accent setting and swatch picker (view 10 / `on_accent_chosen`, `settings::accent_shown`); the
      season replaces it (the core already dropped the setting; remove the dead desktop code).
- [x] Accent rule: audit every `Theme.accent` use (about 40). Allowed: current tab, playing song, lit toggle,
      liked seal, focus ring/caret, lit icon. Remove accent fills behind selected rows, hover fills, buttons.
      A selected row is the surface raised by ink 5-9 % (`roles()` container_high).
- [x] Cover-page rule: album/artist/player pages wear the cover's colours via the `dress` tables, accent through
      `accent_on`; the chrome keeps the page it is on (see Android `rememberChromeLook`).
- [x] Settings page: season picker (tiles marked 春 夏 秋 冬 with the English names), Follow the seasons, light /
      dark / AMOLED. Wire to the core's `season` and `theme` settings (see `crates/settings/src/settings.rs`).

## 2. Icons and fonts (C10, C11)
- [x] Bundle Inter (variable) and Literata from `app/src/main/res/font/` (or a shared assets path; do not fork the
      files) and register them with Slint; remove the macOS `System Font` special case in `main.rs`.
- [x] Replace the `Icons` global's hand-drawn stroke paths with Material Rounded/Filled paths (convert from the
      Material icon SVGs once; keep names). Map: play, pause, next, previous, shuffle, repeat, repeat-one, home,
      library-music, search, settings, album, person, music-note, queue-music, speaker, close, more, add, check.
      Speaker: outlined while playing here, filled while another device plays.
- [x] The seal: add `seal` (filled with cut-out) and `seal-outline` paths from `core/src/main/res/drawable/seal*.xml`.
- [x] Page titles in Literata with -0.3 tracking; large titles tracked tighter; body Inter at Android's scale.

## 3. Chrome and layout (C12, C6)
- [x] Delete `SidebarContent`, `SidebarWindow`, `native-sidebar`, `SIDEBAR_W`, `TitleStrip` sidebar bits, the
      `show_glass` sidebar path, and the sidebar playlist virtualisation; keep drag/zoom on the top bar's empty area.
- [x] Top pill (new component `TopPill`): Home | Albums, Artists, Songs, Playlists, Favorites; current tab named and
      in the accent; guest mode hides Playlists/Favorites as the sidebar did. Left: traffic-light room (inset 44 → new
      top margin). Right: search field (widens on focus, results popover) and a settings button (account, server,
      season with the follow switch, equalizer, other devices, jam entry points). Playlists tab opens the Playlists page.
- [x] Player pill: song left (cover, title, artist · album links), transport middle with seek bar and times under it,
      right cluster in this order: seal, speaker/devices, lyrics, queue, volume. Max width ~1040, centred. Jam
      guest/host rules as in `PlayerControls` today (hide shuffle/repeat/volume for a guest; jam button replaces the speaker).
- [x] Remove the 720 px capsule geometry and `PLAYER_*` constants in `compositor.rs`; layout comes from Slint.
- [x] Pages get top/bottom padding so content starts under the pills and can scroll under them (C6).
- [x] Full player: cover + title + seal + seek + transport (+ shuffle, repeat) + volume left; panel with Lyrics /
      Queue / Devices tabs right; pressing a panel's button again closes it; the page wears the cover's colours.
- [x] Narrow window (< 900 px): top pill collapses to icons; player pill drops the volume slider, then the panel buttons.

## 4. Blur (C6, C14)
- [x] In `compositor.rs`, keep the page layer; add a pass that blurs it once per page frame at 24 logical px and
      draws that, clipped to the two pills' rounded rects, with the page colour at 55 % over it and a 1 px outline
      in the chrome outline colour. Delete the sidebar layer and the player glass shader path that is now unused.
- [x] Recompute the blur only when the page layer changed (dirty flag); idle = zero frames. Verify with a
      frame counter in a debug log that an idle window draws nothing.
- [x] Linux/no-blur fallback: thicker tint (page colour at ~88 %), same outline.
- [x] Edge fades: ramp from page colour to clear, 24 px, under both pills, starting late enough that the glass shows;
      items at rest unchanged. Pill margins swallow clicks (`keepsTouches` equivalent).

## 5. The seal and the press (C7, C8)
- [x] `FavoriteSeal` component: outline upright (quiet ink) vs filled accent tilted -4°, stamp animation
      (scale 1.5→1 spring), no animation when reduced motion; used in the player pill, the full player, song rows,
      the Favorites page. Replace every heart.
- [x] Press ripple on every icon button (ink 16 %, half size → full, ~400 ms fade, pressed state starts it);
      play/pause = bare glyph, 26 px on the pill, large in the full player; glyph cross-fade 180 ms scale .8;
      spinner after 300 ms. Remove `PlayPause` filled circle.

## 6. Pages
- [ ] Home "Listen now": continue card (halfway between title and "For you", swipe not needed), shelves like Android.
- [x] Albums / Artists / Songs / Playlists / Favorites as Android's library sections (tile min 170, gap 12;
      artists round); section chips if Android has them.
- [x] Detail pages (album, artist, playlist, mix, genre), search results, lyrics, queue, devices, jam, equalizer,
      settings: restyled with the palette, outline popovers, Material icons; no behaviour changes.
- [x] Menus/popovers: surface background, hairline outline, 18-22 px radius, ink 6 % hover.

## 7. Motion (C9)
- [x] Page push/pop like Android (new page slides in from the right opaque, old moves a third left under a scrim); tab
      roots cross-fade; player sheet rises from the pill; chrome colour cross-fade 340 ms; no idle animation.
- [x] Reduced motion (system setting) turns every flourish into a direct change.

## 8. Docs, tests, run
- [x] `docs/site/clients/desktop.md` (or the existing desktop page) describes the new look; remove sidebar text.
- [x] Rust tests for any new logic (palette shim mapping, blend date handling); `cargo test -j4 -p nori-desktop`.
- [x] `cargo clippy -j4 -p nori-desktop --all-targets` no new warnings.
- [ ] Build release app with `tools/desktop-app.sh` only when the owner asks; for testing run
      `cargo run -p nori-desktop --release` (the lead will launch it).

## 9. Screens
| screen | restyled | notes |
|---|---|---|
| Home (view 0) | yes | Literata title, shelves and mixes on the palette; the continue card is not drawn: the core gives Rust clients no data for it |
| Search (1) | yes | the top pill's search is a field-shaped button; the field and results are the page's |
| Albums, Playlists, Collections (2, 4, 10) | yes | grids with a Literata title scrolling with them, find field at the right |
| Artists (3) | yes | list and chosen artist, titled |
| Songs (5), Favorites (12) | yes | one list; Favorites is new (`PageAsk::Favorites` in nori-host) and shows starred songs |
| Album / artist / playlist / mix (6) | yes | album and artist wear the cover (dress tables, accent through `accent_on`); playlist and mix stay plain; slide in from the right |
| Login (7) | yes | pill and plate buttons |
| Settings (8), Equalizer (9) | yes | segmented tabs; season tiles with the kanji, System / Light / Dark / AMOLED |
| Mixes (11) | yes | titled grid |
| Queue, lyrics, devices panel | yes | docked right, palette colors |
| Full player | yes | cover page colors, seal, bare large play, volume on the left, Lyrics / Queue / Devices tabs top right |
| Top pill, player pill | yes | own windows laid over the blurred page |
| Settings menu | yes | a popup under the top pill |
| Jam sheet, popovers, menus | yes | surface, chrome outline, no shadow |

## 10. Design notes
- C1: `crates/desktop/src/look.rs` asks `nori_look::theme::palette`, `roles`, `dress::plain/page/accented` and `accents`; the Slint `Theme` global only receives the result. The only literal colors left are Material's baseline error (the palette has none) and the white behind a QR code and on mix tiles.
- C2: season and `theme`/`amoled` settings drive it; the picker marks 春 夏 秋 冬 with the English names; appearance is System / Light / Dark / AMOLED (`appearance_settings`, tested).
- C3: the calendar and blend are `nori_look`'s; the date is the local one (zone from `/etc/localtime` for the hemisphere). Recomputed on window focus and on a system appearance change, never on a timer.
- C4: roles come from `roles()` and the dress table (quiet ink, outline, hairline, chrome outline); hover is the ink at 6 %, selection 10 %.
- C5: accent only on the current tab, the playing song, lit shuffle / repeat / panel buttons, the liked seal, a focused field, active device marks and message actions. Fills behind selection, hover and sliders are ink.
- C6: the page layer is blurred once (the existing mip pyramid, 24 logical px) inside each pill's rounded rect; the pill's own layer lays the page color at 55 % and a hairline over it; the fade (Android's `edgeFade` stops) is drawn under the pills from the unfaded page; the margins around a pill take the pointer. The pyramid is rebuilt only when the page layer was drawn. No blur path exists to fall back from (wgpu and GL both blur), so the thicker tint (`Chrome.tint`) is available but unused.
- C7: `Seal` component; stamp = scale 1.5 to 1 on an overshoot curve (Slint has no spring), only when pressed to like; tilt -4 degrees; reduced motion skips it. The seal paths wind their holes against their fills: Slint's renderer ignores even-odd.
- C8: bare glyph, 26 px on the pill, 52 px in the player; `Ripple` (16 % ink, half size to full, about 400 ms) on every icon button; cross-fade 180 ms from scale .8; spinner after 300 ms.
- C9: no idle animation; the ripple and the look cross-fade run on timers only while they last; the look fades over 340 ms (`dress::mix`); `Motion.reduced` (macOS reduce-motion) turns animations to direct changes.
- C10: Inter Variable everywhere, Literata for titles (-0.5 on large, -0.3 on section titles), from Android's own font files.
- C11: Material Rounded glyphs (speaker: outlined here, filled while another device plays), converted once into `ui/icons.slint`; the seals are core's paths.
- C12: top pill: tabs, search, settings (account, equalizer, devices, jam, season and appearance live in its menu); player pill as specified; full player with the panel beside it. Deviation: the pill's search is a button that opens the search page, which has the field and the results (key events reach only the page's window).
- C13: no rule is written in the client except the pill geometry (`compositor.rs`, with tests) and the fade stops, which Android also keeps client-side.
- C14: blur recomputed only when the page redraws; no new timers outside the ripple, the fade and the spinner.

Not verified: the Linux (OpenGL) path was edited alongside the macOS one but not compiled here; an idle window's frame count was not measured; the full player was not seen running (no song was played in the test session); the seal glyphs were checked only at small sizes.
