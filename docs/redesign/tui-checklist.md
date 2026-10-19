# Terminal client redesign: checklist

Client: `crates/cli` (ratatui; `src/ui.rs` drawing, `src/app.rs` state and `Nav`, `src/art.rs` `Theme`, `src/text.rs`
words, `src/settings_view.rs`). Contract: `docs/redesign-contract.md`. Mockup (owner-approved, open it for exact
intent): `~/Desktop/nori-terminal-mockup.html`. Branch/worktree: `feat/tui-redesign` in `~/Documents/Projects/Apps/nori-tui`.
Goal: the terminal shows the same design language as the other clients within what a terminal can do:
season palettes in true colour, top outline with tabs, bottom player outline, seal, bare glyphs, fades instead of blur.

## 0. Setup
- [x] Read AGENTS.md, the contract, the mockup; run `cargo run -p nori-cli` against the test server
      (`tools/dev-server.sh`) and note the current layout (sidebar, page, right panel, player bar, `Theme::plain`).
- [x] Note the existing CLI tests (`cargo test -j4 -p nori-cli`, `NORI_TUI_DUMP=dir` dumps screens) and keep them green.

## 1. Theme from Rust (C1-C5)
- [x] Replace `Theme { accent, text, dim, page }` + `Theme::plain(prefs.accent)` with a theme built from
      `nori_look::theme::palette(...)` for the current `season` setting, date, hemisphere (`southern(zone)` /
      TZ env) and appearance: fields page, surface, ink, quiet, line (outline), hairline, accent, on.
- [x] True colour when `COLORTERM` says truecolor, else the nearest xterm-256 per role (write the mapper in
      `art.rs`, table-test it); NO_COLOR respected (no colour at all, bold/underline only).
- [x] Paint the page: fill the whole frame with the page colour (option `--no-paint` / setting to leave the terminal's own
      background, using only ink + accent). AMOLED = black page.
- [x] Drop the accent setting; accent only as ink for current things (C5): no `bg(accent)` for selected rows or the
      play button. Selected row = surface background (+ bold). Fix `selected()`, `one_card`, chips, lists.
- [x] Settings: season row (Follow + 4 seasons, English names with 春 夏 秋 冬) and appearance, using the core's settings
      model (`crates/settings`), shown in `settings_view.rs`.

## 2. Layout (C12, C6)
- [x] Remove the sidebar (`sidebar()`, `SIDE_W`, `Focus::Side` paths) and the always-on right panel logic; keep every
      destination reachable: Home, Albums, Artists, Songs, Downloads, Playlists (list popup), Favorites, Search,
      Equalizer, Settings, Servers/accounts (in the settings screen).
- [x] Top outline (3 rows, rounded box characters in the outline colour): tabs `⌂ Home | ◫ Albums ◉ Artists ♪ Songs ≡ Playlists
      好 Favorites`, current one accent + bold + underline; search box right (`⌕ Search`, `/` focuses), settings `✱`.
      Mouse: every tab/button registered in `App::hits`. Keys: 1-6 tabs, `/` search, `,` settings (keep existing keys).
- [x] Player outline (5 rows incl. border): cover thumbnail (image protocol or half-block) + title/artist left; transport
      `⤮ ⏮ ⏸/▶ ⏭ ↻/↻¹` middle with `1:12 ━━━──── 3:24` under it; right: seal, `⇄` devices, `≣` lyrics, `☰` queue, `◁)` volume bar.
      Jam guest rules as in the current `player_bar` (offered()). Narrow (< 100 cols): drop volume, then panel buttons;
      < 60: song + play only; tabs icons-only < 80.
- [x] Full player: cover (large, half-block/graphics) + title + seal + seek + transport + volume left; Lyrics / Queue /
      Devices panel right (reuse the existing `lyrics`, `queue`, `devices` drawers, restyled); `esc` closes.
- [x] Footer hint line (one row, quiet).

## 3. Fade, seal, glyphs (C6-C8)
- [x] Edge fade: the 2 rows nearest each outline blend fg/bg toward the page (33 % then 66 %); cards, art and text.
- [x] Seal: `▕好▏` quiet when not liked, `▐好▌` reversed accent when liked, one-frame invert as the stamp (skipped with
      reduced motion / `NORI_REDUCE_MOTION`), replacing `♥ ♡` everywhere (bar, full player, song rows, favorites).
- [x] Play/pause: bare glyph; press feedback = 230 ms reverse video via a one-shot redraw timer ONLY after a click/key
      (no idle ticking; the app draws only on events today — keep that).
- [x] Shuffle/repeat lit = accent bold; off = quiet ink. Repeat-one `↻¹`.

## 4. Pages
- [x] Home "Listen now" with shelves (no continue card: see notes), Albums grid, Artists, Songs table (no accent row fills;
      playing song accent + bold), Playlists, Favorites, search results, album/artist/playlist pages, downloads,
      equalizer, settings, login: restyled to palette; popups use surface + outline.
- [x] Page titles bold (no fonts in a terminal); captions quiet.

## 5. Docs, tests, run
- [x] Update `docs/site` terminal page and `crates/cli/src/text.rs` words (English); no stale sidebar text.
- [x] Tests: palette→ratatui mapping, 256-colour mapper, fade blend, tab hit-testing, narrow-width layouts using
      `TestBackend` dumps; `cargo test -j4 -p nori-cli` green; clippy clean for the crate.
- [x] Run it in a real terminal-sized TestBackend for 120x36 and 80x24 and read the dumps for glitches.

## 6. Design notes (written last: for each contract rule C1-C14, how this client did it, or why not)

### Done as written, with these deviations
- The continue card is not drawn: the core exposes no "continue from your other device" data to Rust clients (it is Kotlin
  logic on Android), and a rule written in two languages is a bug.
- `,` is not settings: it stays a seek key (existing keys win). Settings is `9`, the `✱` box, or a click.
- Downloads and the equalizer are `7` / `8` and links in the hint line; the server's state is there too.
- Favorites lists the starred songs only (not starred albums and artists).
- The edge fade applies only while the page is scrolled under an outline ("items at rest are untouched").
  Pictures drawn with kitty or sixel cannot be faded; half-block covers are.
- Not verified: a real terminal (kitty/sixel pictures, 256-color look on a real xterm), `--no-paint` on a light terminal.

## 6. Design notes
- C1 Palettes come from Rust: `art::Theme::season` calls `nori_look::theme::palette` and `roles`; no hex lives in the CLI.
- C2 Four seasons: the `season` setting row (Follow, Spring 春 ... Winter 冬), `theme` (as the terminal / light / dark) and
  `amoled` rows are the core's, now offered to the terminal (`settings_pages::look`).
- C3 Follow is `palette(None, ...)` with today's date by the local clock and the hemisphere from `TZ` or `/etc/localtime`.
- C4 Roles: page, surface, ink, accent, on from the palette; quiet 62 %, hairline 14 % from `roles()`, chrome line 22 %
  through the same `over(with_alpha(..))` veil; a test pins them.
- C5 Accent only for the open tab, the song playing, lit shuffle/repeat, a liked seal, a focused field; selected rows are the
  surface (`Theme::select`), switches that are on use `on`. No `bg(accent)` remains except the liked seal's fill, which is the seal.
- C6 The outline is a rounded box in the 22 % line color with the page color thick inside it; the page's two nearest rows fade
  (66 %, 33 %) toward the page when scrolled under it (`fade_edges`). Unpainted pages dim instead.
- C7 Seal: `▕好▏` quiet, `▐好▌` reversed accent; the stamp is a 230 ms reversed frame, skipped with `NORI_REDUCE_MOTION`.
- C8 Play/pause is the bare glyph; press feedback is 230 ms reverse video after a click or key only (`App::flash`).
- C9 No new idle timers: the flash adds one wake only while it shows (`next_wake`); nothing animates untouched. The crossfade
  between palettes is not done (a repaint, not a 340 ms fade).
- C10 Weights only: bold titles, quiet captions, underline on the open tab.
- C11 The mockup's Unicode set (⌂ ◫ ◉ ♪ ≡ 好, ⤮ ⏮ ⏸ ⏭ ↻, ⇄ ≣ ☰ ◁)).
- C12 No sidebar; tabs, search and settings on top, the player below, full player with Lyrics / Queue / Devices panel.
- C13 Rules in Rust: season/appearance in `nori_look` and settings; the CLI holds drawing (`chrome.rs` layout is pure and tested).
- C14 No timers added; colors are mapped once per frame only for 256/no-color terminals (`art::finish`), no blur.
