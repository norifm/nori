# Web client redesign: checklist

Client: `web/` (Svelte 5 + Vite, `web/src/ui/*.svelte`, `web/src/pages/*.svelte`, `web/src/app.css`, state in
`web/src/state/*.svelte.ts`) over the Rust core compiled for the browser (`crates/web`, `web/build.sh`).
Contract: `docs/redesign-contract.md`. Long form: `docs/macos-redesign.md`. Mockup of the target (the web client
looks like the Mac one, in a browser tab): `~/Desktop/nori-macos-mockup.html`. Branch/worktree:
`feat/web-redesign` in `~/Documents/Projects/Apps/nori-web`.
Goal: the web client gets the same treatment as the Mac redesign: no sidebar, outlined top pill, outlined player pill,
season palettes, seal, bare play glyph with ripple, Material icons, Inter + Literata, CSS blur like Android.

## 0. Setup
- [ ] Read AGENTS.md, the contract, the mockup, `web/build.sh`, `web/serve.py`, `docs/site/developers/` web notes.
      Build and serve the web client (`web/build.sh`, `python3 web/serve.py` or as documented) against the local
      test server (`tools/dev-server.sh`) and screenshot the current UI (headless Chrome:
      `~/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing --headless=new --screenshot`).
- [ ] Find how the web client gets colours today and whether `nori-look` is exposed through `crates/web`.

## 1. Theme from Rust (C1-C5)
- [ ] Expose `palette(...)`/`roles(...)` (and `season_now`) from `crates/web` (wasm-bindgen / the existing rpc
      mechanism, see `web/src/rpc.ts`) if not already; add a Rust test. The web client must not hard-code hexes.
- [ ] A `theme` module in `web/src/state/` sets CSS custom properties on `:root` (`--page --surface --ink --accent
      --on --ink2 --hair --outline --glass`) from the core palette, for the `season` setting (Follow or fixed),
      appearance (light/dark/AMOLED; "system" follows `prefers-color-scheme`), the date and hemisphere
      (`Intl.DateTimeFormat().resolvedOptions().timeZone` → core `southern`). Re-evaluate on load, on visibility
      change and on setting change; never on a timer.
- [ ] Replace every colour in `app.css` and the components with those variables; remove the accent picker and any
      hard-coded accent. Accent only for "you, now" (C5); no accent-filled selected rows or buttons.
- [ ] Settings page: season picker (tiles 春 夏 秋 冬 + English names), Follow, appearance, wired to the core settings.
- [ ] `<meta name="theme-color">` follows the page colour.

## 2. Type and icons (C10, C11)
- [ ] Self-host Inter (variable) and Literata (the files in `app/src/main/res/font/`; copy into `web/` assets at
      build time, not forked in git) with `@font-face`; page titles Literata with -0.3px tracking.
- [ ] `Icon.svelte` glyphs → Material Rounded/Filled paths (same shapes per action as Android); seal paths from
      `core/src/main/res/drawable/seal*.xml`; `Heart.svelte` becomes the seal (C7).

## 3. Chrome and layout (C12, C6)
- [ ] Remove the sidebar (see `Shell.svelte`, `PaneView.svelte`); top pill component: Home | Albums, Artists,
      Songs, Playlists, Favorites (+ Genres if the web has it, under a library menu), search field right
      (widens on focus, results popover), settings button (account, season, devices, jam). Playlists tab opens a
      popover of playlists. Routes unchanged.
- [ ] Player pill (`PlayerBar.svelte`/`MiniPlayer.svelte`): song left; shuffle, previous, play, next, repeat + seek
      bar middle; seal, speaker (Play On / devices), lyrics, queue, volume right. Max ~1040 px centred, fixed to
      the bottom. Jam rules as today.
- [ ] Mobile width (< 720 px): Android's upright layout instead (the pill holds mini player over tabs; tabs at the
      bottom, search as a tab) — the web client is used on phones, so the narrow layout is the Android upright chrome.
- [ ] Full player (`NowPlaying.svelte`): cover, title + seal, seek, transport, volume left; Lyrics / Queue / Devices
      panel right on wide screens, stacked/sheet on narrow.
- [ ] Content padding so pages start under the top pill and scroll under both pills with the 24 px edge fade
      (`mask-image` linear-gradient), pill margins swallow pointer events.

## 4. Blur and press (C6, C8, C9, C14)
- [ ] Pills: `backdrop-filter: blur(24px)` over `background: color-mix(in srgb, var(--page) 55%, transparent)`,
      1px outline `var(--outline)`; fallback without backdrop-filter (`@supports not`) page at 88 %.
- [ ] Play/pause bare glyph with ripple (CSS only, ::before scale/opacity), glyph cross-fade 180 ms; seal stamp
      keyframes (scale 1.5→1, tilt -5° when liked); `prefers-reduced-motion` disables flourishes.
- [ ] No new timers/animation loops; `requestAnimationFrame` only while the player is open as today.

## 5. Pages
- [ ] Home, Library sections, Album/Artist/Playlist/Genre pages, Search, Settings, Jam, Devices/Play On, Lyrics,
      Queue, prompts/sheets: restyled; behaviour unchanged. Page push/pop: crossfade or Android-like slide if
      cheap; reduced motion = none.

## 6. Tests, docs, run
- [ ] `npm run check` (svelte-check, warnings are errors) green; `web/check/*.mjs` updated where they depend on markup
      (selectors for sidebar/heart) and run if the harness works here; screenshots with `web/check/shots.mjs`.
- [ ] `docs/site` web client page updated; changelog line comes from the commit subjects.
- [ ] Headless screenshots of Home, Albums, Songs, full player (wide + 390 px), each season, light/dark/AMOLED,
      saved under the scratchpad for the lead's comparison.

## 7. Design notes (written last: for each contract rule C1-C14, how this client did it, or why not)
