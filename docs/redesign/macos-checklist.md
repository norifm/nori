# Mac client redesign: checklist

Client: `crates/desktop` (Slint + wgpu/Skia, `crates/desktop/ui/app.slint`, `src/compositor.rs`, `src/app.rs`).
Contract: `docs/redesign-contract.md`. Long form and rationale: `docs/macos-redesign.md`. Mockup (owner-approved,
open it for pixel intent): `~/Desktop/nori-macos-mockup.html`.
Branch/worktree: `feat/macos-redesign` in `~/Documents/Projects/Apps/nori-macos`.
Goal: the Mac app is Android's upright UI made wide. Sidebar gone, one outlined top pill, one outlined player
pill, season palettes, the seal, bare play glyph with ripple, Material icons, Inter + Literata, blur like Android.

## 0. Setup
- [ ] Read AGENTS.md, the contract, `docs/macos-redesign.md`, and the mockup. Run `cargo run -p nori-desktop` once
      (log in to the test server as the README says; `tools/dev-server.sh` gives a local Navidrome) and screenshot
      the current app to know the starting point.
- [ ] List every screen/overlay in `app.slint` (views 0-11, queue, lyrics, devices, jam, equalizer, settings,
      login, popovers, menus) in a "Screens" table at the bottom of this file with a status column.

## 1. Theme from Rust (C1-C5)
- [ ] Add a small shim in `crates/desktop/src/` that asks `nori_look` for the palette (`palette(season, month, day,
      southern, dark, amoled)`, `roles`) and sets a Slint global `Palette` (page, surface, ink, accent, on, quiet
      ink, outline, hairline, glass tint). Drive it from the `season` and `theme` settings; recompute when
      settings change and once at the start of the day (no ticking timer: recompute on window focus and on resume).
- [ ] Replace `Theme.ground/text/dim/faint/hairline/hover/chosen/fill/panel/accent` (the Music-measured greys,
      `#fa2d48`) with roles derived from `Palette`; keep the property names where used widely, change their source.
- [ ] Remove the accent setting and swatch picker (view 10 / `on_accent_chosen`, `settings::accent_shown`); the
      season replaces it (the core already dropped the setting; remove the dead desktop code).
- [ ] Accent rule: audit every `Theme.accent` use (about 40). Allowed: current tab, playing song, lit toggle,
      liked seal, focus ring/caret, lit icon. Remove accent fills behind selected rows, hover fills, buttons.
      A selected row is the surface raised by ink 5-9 % (`roles()` container_high).
- [ ] Cover-page rule: album/artist/player pages wear the cover's colours via the `dress` tables, accent through
      `accent_on`; the chrome keeps the page it is on (see Android `rememberChromeLook`).
- [ ] Settings page: season picker (tiles marked 春 夏 秋 冬 with the English names), Follow the seasons, light /
      dark / AMOLED. Wire to the core's `season` and `theme` settings (see `crates/settings/src/settings.rs`).

## 2. Icons and fonts (C10, C11)
- [ ] Bundle Inter (variable) and Literata from `app/src/main/res/font/` (or a shared assets path; do not fork the
      files) and register them with Slint; remove the macOS `System Font` special case in `main.rs`.
- [ ] Replace the `Icons` global's hand-drawn stroke paths with Material Rounded/Filled paths (convert from the
      Material icon SVGs once; keep names). Map: play, pause, next, previous, shuffle, repeat, repeat-one, home,
      library-music, search, settings, album, person, music-note, queue-music, speaker, close, more, add, check.
      Speaker: outlined while playing here, filled while another device plays.
- [ ] The seal: add `seal` (filled with cut-out) and `seal-outline` paths from `core/src/main/res/drawable/seal*.xml`.
- [ ] Page titles in Literata with -0.3 tracking; large titles tracked tighter; body Inter at Android's scale.

## 3. Chrome and layout (C12, C6)
- [ ] Delete `SidebarContent`, `SidebarWindow`, `native-sidebar`, `SIDEBAR_W`, `TitleStrip` sidebar bits, the
      `show_glass` sidebar path, and the sidebar playlist virtualisation; keep drag/zoom on the top bar's empty area.
- [ ] Top pill (new component `TopPill`): Home | Albums, Artists, Songs, Playlists, Favorites; current tab named and
      in the accent; guest mode hides Playlists/Favorites as the sidebar did. Left: traffic-light room (inset 44 → new
      top margin). Right: search field (widens on focus, results popover) and a settings button (account, server,
      season, equalizer, other devices, jam entry points). Playlists tab opens a popover list of playlists.
- [ ] Player pill: song left (cover, title, artist · album links), transport middle with seek bar and times under it,
      right cluster in this order: seal, speaker/devices, lyrics, queue, volume. Max width ~1040, centred. Jam
      guest/host rules as in `PlayerControls` today (hide shuffle/repeat/volume for a guest; jam button replaces the speaker).
- [ ] Remove the 720 px capsule geometry and `PLAYER_*` constants in `compositor.rs`; layout comes from Slint.
- [ ] Pages get top/bottom padding so content starts under the pills and can scroll under them (C6).
- [ ] Full player: cover + title + seal + seek + transport (+ shuffle, repeat) + volume left; panel with Lyrics /
      Queue / Devices tabs right; pressing a panel's button again closes it; the page wears the cover's colours.
- [ ] Narrow window (< 900 px): top pill collapses to icons; player pill drops the volume slider, then the panel buttons.

## 4. Blur (C6, C14)
- [ ] In `compositor.rs`, keep the page layer; add a pass that blurs it once per page frame at 24 logical px and
      draws that, clipped to the two pills' rounded rects, with the page colour at 55 % over it and a 1 px outline
      in the chrome outline colour. Delete the sidebar layer and the player glass shader path that is now unused.
- [ ] Recompute the blur only when the page layer changed (dirty flag); idle = zero frames. Verify with a
      frame counter in a debug log that an idle window draws nothing.
- [ ] Linux/no-blur fallback: thicker tint (page colour at ~88 %), same outline.
- [ ] Edge fades: ramp from page colour to clear, 24 px, under both pills, starting late enough that the glass shows;
      items at rest unchanged. Pill margins swallow clicks (`keepsTouches` equivalent).

## 5. The seal and the press (C7, C8)
- [ ] `FavoriteSeal` component: outline upright (quiet ink) vs filled accent tilted -5°, stamp animation
      (scale 1.5→1 spring), no animation when reduced motion; used in the player pill, the full player, song rows,
      the Favorites page. Replace every heart.
- [ ] Press ripple on every icon button (ink 16 %, half size → full, ~400 ms fade, pressed state starts it);
      play/pause = bare glyph, 26 px on the pill, large in the full player; glyph cross-fade 180 ms scale .8;
      spinner after 300 ms. Remove `PlayPause` filled circle.

## 6. Pages
- [ ] Home "Listen now": continue card (halfway between title and "For you", swipe not needed), shelves like Android.
- [ ] Albums / Artists / Songs / Playlists / Favorites as Android's library sections (tile min 170, gap 12;
      artists round); section chips if Android has them.
- [ ] Detail pages (album, artist, playlist, mix, genre), search results, lyrics, queue, devices, jam, equalizer,
      settings: restyled with the palette, outline popovers, Material icons; no behaviour changes.
- [ ] Menus/popovers: surface background, hairline outline, 18-22 px radius, ink 6 % hover.

## 7. Motion (C9)
- [ ] Page push/pop like Android (new page slides in from the right opaque, old moves a third left under a scrim); tab
      roots cross-fade; player sheet rises from the pill; chrome colour cross-fade 340 ms; no idle animation.
- [ ] Reduced motion (system setting) turns every flourish into a direct change.

## 8. Docs, tests, run
- [ ] `docs/site/clients/desktop.md` (or the existing desktop page) describes the new look; remove sidebar text.
- [ ] Rust tests for any new logic (palette shim mapping, blend date handling); `cargo test -j4 -p nori-desktop`.
- [ ] `cargo clippy -j4 -p nori-desktop --all-targets` no new warnings.
- [ ] Build release app with `tools/desktop-app.sh` only when the owner asks; for testing run
      `cargo run -p nori-desktop --release` (the lead will launch it).

## 9. Screens
(table filled in during step 0: screen | restyled | notes)

## 10. Design notes (written last: for each contract rule C1-C14, how this client did it, or why not)
