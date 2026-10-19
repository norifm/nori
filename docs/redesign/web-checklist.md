# Web client redesign: checklist

Client: `web/` (Svelte 5 + Vite, `web/src/ui/*.svelte`, `web/src/pages/*.svelte`, `web/src/app.css`, state in
`web/src/state/*.svelte.ts`) over the Rust core compiled for the browser (`crates/web`, `web/build.sh`).
Contract: `docs/redesign-contract.md`. Long form: `docs/macos-redesign.md`. Mockup of the target (the web client
looks like the Mac one, in a browser tab): `~/Desktop/nori-macos-mockup.html`. Branch/worktree:
`feat/web-redesign` in `~/Documents/Projects/Apps/nori-web`.
Goal: the web client gets the same treatment as the Mac redesign: no sidebar, outlined top pill, outlined player pill,
season palettes, seal, bare play glyph with ripple, Material icons, Inter + Literata, CSS blur like Android.

## 0. Setup
- [x] Read AGENTS.md, the contract, the mockup, `web/build.sh`, `web/serve.py`, `docs/site/developers/` web notes.
      Build and serve the web client (`web/build.sh`, `python3 web/serve.py` or as documented) against the local
      test server (`tools/dev-server.sh`) and screenshot the current UI (headless Chrome:
      `~/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing --headless=new --screenshot`).
- [x] Find how the web client gets colours today and whether `nori-look` is exposed through `crates/web`.

## 1. Theme from Rust (C1-C5)
- [x] Expose `palette(...)`/`roles(...)` (and `season_now`) from `crates/web` (wasm-bindgen / the existing rpc
      mechanism, see `web/src/rpc.ts`) if not already; add a Rust test. The web client must not hard-code hexes.
- [x] A `theme` module in `web/src/state/` sets CSS custom properties on `:root` (`--page --surface --ink --accent
      --on --ink2 --hair --outline --glass`) from the core palette, for the `season` setting (Follow or fixed),
      appearance (light/dark/AMOLED; "system" follows `prefers-color-scheme`), the date and hemisphere
      (`Intl.DateTimeFormat().resolvedOptions().timeZone` → core `southern`). Re-evaluate on load, on visibility
      change and on setting change; never on a timer.
- [x] Replace every colour in `app.css` and the components with those variables; remove the accent picker and any
      hard-coded accent. Accent only for "you, now" (C5); no accent-filled selected rows or buttons.
- [x] Settings page: season picker (tiles 春 夏 秋 冬 + English names), Follow, appearance, wired to the core settings.
- [x] `<meta name="theme-color">` follows the page colour.

## 2. Type and icons (C10, C11)
- [x] Self-host Inter (variable) and Literata (the files in `app/src/main/res/font/`; copy into `web/` assets at
      build time, not forked in git) with `@font-face`; page titles Literata with -0.3px tracking.
- [x] `Icon.svelte` glyphs → Material Rounded/Filled paths (same shapes per action as Android); seal paths from
      `core/src/main/res/drawable/seal*.xml`; `Heart.svelte` becomes the seal (C7).

## 3. Chrome and layout (C12, C6)
- [x] Remove the sidebar (see `Shell.svelte`, `PaneView.svelte`); top pill component: Home | Albums, Artists,
      Songs, Playlists, Favorites (+ Genres if the web has it, under a library menu), search field right
      (widens on focus, results popover), settings button (account, season, devices, jam). Playlists tab opens a
      popover of playlists. Routes unchanged.
- [x] Player pill (`PlayerBar.svelte`/`MiniPlayer.svelte`): song left; shuffle, previous, play, next, repeat + seek
      bar middle; seal, speaker (Play On / devices), lyrics, queue, volume right. Max ~1040 px centred, fixed to
      the bottom. Jam rules as today.
- [x] Mobile width (< 720 px): Android's upright layout instead (the pill holds mini player over tabs; tabs at the
      bottom, search as a tab) — the web client is used on phones, so the narrow layout is the Android upright chrome.
- [x] Full player (`NowPlaying.svelte`): cover, title + seal, seek, transport, volume left; Lyrics / Queue / Devices
      panel right on wide screens, stacked/sheet on narrow.
- [x] Content padding so pages start under the top pill and scroll under both pills with the 24 px edge fade
      (`mask-image` linear-gradient), pill margins swallow pointer events.

## 4. Blur and press (C6, C8, C9, C14)
- [x] Pills: `backdrop-filter: blur(24px)` over `background: color-mix(in srgb, var(--page) 55%, transparent)`,
      1px outline `var(--outline)`; fallback without backdrop-filter (`@supports not`) page at 88 %.
- [x] Play/pause bare glyph with ripple (CSS only, ::before scale/opacity), glyph cross-fade 180 ms; seal stamp
      keyframes (scale 1.5→1, tilt -5° when liked); `prefers-reduced-motion` disables flourishes.
- [x] No new timers/animation loops; `requestAnimationFrame` only while the player is open as today.

## 5. Pages
- [x] Home, Library sections, Album/Artist/Playlist/Genre pages, Search, Settings, Jam, Devices/Play On, Lyrics,
      Queue, prompts/sheets: restyled; behaviour unchanged. Page push/pop: crossfade or Android-like slide if
      cheap; reduced motion = none.

## 6. Tests, docs, run
- [x] `npm run check` (svelte-check, warnings are errors) green; `web/check/*.mjs` updated where they depend on markup
      (selectors for sidebar/heart) and run if the harness works here; screenshots with `web/check/shots.mjs`.
- [x] `docs/site` web client page updated; changelog line comes from the commit subjects.
- [x] Headless screenshots of Home, Albums, Songs, full player (wide + 390 px), each season, light/dark/AMOLED,
      saved under the scratchpad for the lead's comparison.

Not done / not verified: no page push/pop transition (none is the reduced-motion answer; skipped as not cheap); `web/check/check.mjs` and `jam.mjs` selectors were updated (`#panel` became `#full`, the seek times are `.seek .time`) but the suites were not run; the Settings page's Appearance section and the jam were not screenshotted.

## 7. Design notes (written last: for each contract rule C1-C14, how this client did it, or why not)

- C1 Palettes from Rust: `Nori.theme` (crates/web) calls `nori_settings::settings::theme_palette` (tested in crates/settings) and `nori_look::theme::roles`; the shell holds no season hexes, only CSS variables set by `state/theme.ts`. Before the core answers the system colors (Canvas/CanvasText) or the last answer (localStorage) are worn.
- C2 Four seasons: `ui/Look.svelte` (gear menu and Settings > Appearance): tiles 春 夏 秋 冬 with English names, "Follow the seasons" switch, light/dark/system chips, pure black. The core's `season`, `theme`, `amoled` settings; the Look settings page is now offered to the web client.
- C3 Follow: `palette(None, ...)` blends over 14 days in the core; the page passes the browser's date and zone (`southern`), re-evaluated on load, visibility and scheme change, never on a timer.
- C4 Roles: page, surface, ink, ink2 (62 %), outline (36 %), hair (14 %), accent, on, on-accent come from the core's `roles`; the chrome outline is the ink at 21 % in CSS, the glass the page at 55 %.
- C5 Accent: only the current tab, the playing song's title, lit shuffle/repeat, the liked seal, a focused search field and the chosen season tile. Solid buttons are ink-filled, chips on = ink fill, no accent fills.
- C6 One outline: `.chrome` (blur 24 px, page at 55 %, 21 % ink hairline, no shadow; `@supports not` raises the page to 88 %); `#main` scrolls under both bars with `mask-image` fades (`.fade`); the top bar's grid and the bars swallow clicks on their margins.
- C7 Seal: `ui/Seal.svelte`, the core's seal paths; outline and upright, or filled accent tilted -5 degrees and stamped (scale 1.5 to 1, 420 ms overshoot curve standing for the spring) only when the click liked it; off is immediate; reduced motion removes the animation (global rule).
- C8 Play/pause: the bare glyph, a CSS ripple (`.round::before`, ink at 16 %, grows from half size) on every round button, 180 ms glyph cross-fade with scale from .8, a spinner after 300 ms of buffering.
- C9 Nothing animates unless touched: the color cross-fade is 340 ms through registered `@property` colors; no idle animation; the only timers are the existing seek ones and the 300 ms spinner delay; reduced motion turns every animation and transition off.
- C10 Type: Inter and Literata variable fonts from the Android files, copied by `web/build.sh` into `web/src/fonts` (git-ignored); Literata only for page titles (h1, tracking -0.3 px) and the wordmark.
- C11 Icons: Material Rounded/Filled paths (jsDelivr's @material-design-icons/svg, the same shapes as Android) in the sprite in `web/index.html`; outlined speaker while this browser plays, filled while another device does; seal paths from `core/src/main/res/drawable/seal*.xml`.
- C12 Layout: wide: top bar (Home | Albums Artists Songs Playlists Favorites, search with a results popover, settings menu with account, season, Genres, Play On), player bar (song | shuffle prev play next repeat + seek | seal, speaker, lyrics, queue, volume), full player (cover, title and seal, seek, transport, volume left; Lyrics/Queue/Play On panel right). Below 720 px: one bottom bar with the mini player over the tabs, the full player stacked. Deviations: Genres lives in the settings menu, not a tab; jam stays in the Play On pane; the player bar's speaker hides when remote control is off, as before.
- C13 Rust first: the palette choice (`theme_palette`) and the Look page's rows are in nori-settings with tests; volume is the session's (`Nori.volume`, `now.volume`).
- C14 Battery: no new loops; blur is the browser's `backdrop-filter` (recomputed when the page behind it paints); the mask fades are static gradients.
