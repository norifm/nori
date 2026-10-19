# iPod client: theme picker and season following (colours only): checklist

Client: `ios/` (Swift, UIKit, iOS 12; `ios/Sources/Theme.swift`, `Shell.swift`, `Pages.swift`, `Player.swift`,
`Glyphs.swift`, `Say.swift`) over `crates/ios` / `crates/host` (a C ABI). Docs: `docs/site/developers/ipod-internals.md`.
Contract: `docs/redesign-contract.md` (only C1-C5 apply here). Branch/worktree: `feat/ipod-theme` in
`~/Documents/Projects/Apps/nori-ipod`.
Scope is deliberately small: **color changes only. No layout redesign, no new features, no new fonts or icons, no
blur, no seal.** The iPod keeps its monochrome structure; the theme just gives it the season's colours, and the
season setting works exactly as on the other clients.

## 0. Setup
- [x] Read AGENTS.md, the contract C1-C5, `docs/site/developers/ipod-internals.md`, `ios/Sources/Theme.swift`
      (today: monochrome greys, `Theme.light`, `accent = label`), and how settings reach the iPod (the core lays
      settings pages out; `Pages.swift`/`Shell.swift` render rows from `crates/host`).
- [x] The iPod build is Docker-based (`tools/ipod.sh`); check whether it can build here (Docker running?). If not,
      verify with the Rust side tests and a careful read, and say so.

## 1. Season in the C ABI (C1, C3)
- [x] Expose from `crates/ios` (the C header `ios/Sources/nori_ios.h`) a function returning the palette for the
      current settings: `nori_ios_palette(dark, amoled) -> struct {page, surface, ink, accent, on, ...}` computed with
      `nori_look::theme::palette(season setting, today's month/day, southern hemisphere from the time zone, ...)`.
      No hexes in Swift. Rust test for the mapping. Keep the header and the Swift bridge in step.
- [x] The device's date comes from the system (as the other clients); recompute on app launch, on becoming active,
      and on setting change; never a timer.

## 2. Theme.swift (colours only)
- [x] Replace the grey constants with the palette: `background` = page, `row` = surface, `label` = ink, `secondary` = ink 65 % (from the core)
      over page, `hairline`/`track` from ink alpha, `accent` = season accent (used where the current code uses `accent`:
      tab tint, nav tint, the playing row, lit shuffle/repeat, switches' on-colour = `on`), `dim` = quiet ink.
      Keep `Theme.light` semantic (light vs dark appearance); AMOLED = black page.
- [x] `Theme.Card` (player card/queue/lyrics): keep its structure; page-coloured per appearance (as today it is
      black/white regardless of cover), ink/secondary from the palette.
- [x] Views rebuild on change as they already do for the light/dark mode.

## 3. Settings: picker and follow (C2)
- [x] The Theme settings page gets: **Season** picker with Spring, Summer, Autumn, Winter (English names, 春 夏 秋 冬 beside
      them as in the other clients' pickers) and a **Follow the seasons** toggle (Follow = the core's `season` setting
      value `Follow`; off = a fixed season, defaulting to the one in effect). Use the core's settings model so the setting
      name/values are the same as Android's (`season`); only add the row if the core page doesn't already render it.
- [x] Words in `Say.swift` (English); appearance setting (light/dark; AMOLED if the setting is shown on the iPod).
- [x] Show the current season in the row's detail ("Autumn now · Winter from 7 November") if the core gives it
      (`season_now`), as Android's settings do.

## 4. Check and docs
- [x] Every screen reads its colours from `Theme` (grep for hard-coded `UIColor(` / `rgb:` in `ios/Sources` and move any
      that are theme colours).
- [x] `cargo test -j4 -p nori-ios -p nori-look`; build the iPod app with `tools/ipod.sh build` if Docker works.
- [x] `docs/site` iPod page: mention seasons and the setting.

## 5. Design notes (written last: what was done for C1-C5, what was left alone and why)

- **C1.** Every colour comes from `nori_look::theme::palette` through `nori_ios_palette` (`crates/ios/src/theme.rs`);
  no hex of a season is in Swift. The greys are the palette's ink over its page, composed in `Theme.swift`.
  `nori_ios_palette` takes the `season` setting's value, the mode, and the device's time and zone instead of the
  checklist's `(dark, amoled)`, so it works before the session is open (the last settings are kept in
  UserDefaults for the launch).
- **C2.** Settings, Theme lists Spring 春, Summer 夏, Autumn 秋, Winter 冬 and a "Follow the seasons" switch, and
  Light/Dark; they write the core's `season` and `theme` settings (values FOLLOW, SPRING..., LIGHT, DARK).
  Switching follow off keeps the season the calendar has now. The row shows "Autumn now · Winter from 7 November"
  from `nori_ios_season_now`. AMOLED is not offered, as before (an LCD), so the palette is asked with it off.
- **C3.** Follow and the 14-day blend are `nori_look::theme::palette(None, ...)`; the southern hemisphere comes
  from `southern(zone)`. The date is read at launch, when the app becomes active, and on a setting change; no timer.
- **C4.** secondary = ink 65 % over page, from the core, dim 36 %, track 20 %, hairline 14 %; `on` is the switch colour.
- **C5.** Accent is the tab and nav tint, the playing row (title and bar), the active server's check, a focused
  field, the lit shuffle on a page and the lit shuffle/repeat in the queue. Buttons that were accent fills or
  tints without being "you, now" (Play and Shuffle pills at rest, round buttons, Connect, Add server, the mini
  player's progress) are ink, so the iPod stays monochrome apart from the season's accent.
- **Left alone.** C6-C14 (not mine): no blur, seal, fonts or icons, layout unchanged. The error red on the login
  page, the black text of the system action sheets and the white name over a cover's dark band are not theme
  colours. Cover colours still tint nothing. Changing the theme still rebuilds the interface on the tab's root,
  as the light/dark switch did.
- **Verified.** `cargo test -j4 -p nori-ios` theme tests (fixed season, follow, southern, blend, offset,
  season_now); `tools/ipod.sh build` produced the .ipa (so the Swift compiles). Not run on a device or the
  simulator, so the look of the screens and the rebuild on a season change are unseen.
