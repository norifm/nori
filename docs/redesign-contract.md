# The redesign's design contract

Internal planning note, not published. Every client (Android, Mac, terminal, web, iPod) follows this. Android
is the reference implementation; where this note and Android differ, Android wins and this note gets fixed.
The long form is `docs/macos-redesign.md` section 2 ("The design language to mirror"); this is the checklist
form that all five client checklists point back to. Mockups the owner approved (on the owner's Desktop, not in
the repo): `nori-macos-mockup.html`, `nori-terminal-mockup.html`, `nori-seasons-final.html`.

## Rules (each client records, in its checklist's "Design notes" section, how it did each one)

C1. **Palettes come from Rust.** `nori_look::theme::{Season, palette, season_now, roles, accent_on}` is the
    only source of colours. No client re-derives or hard-codes a season's hexes (the web client and iPod,
    which cannot call Rust directly, get the values through their existing core bridge, never a copy).
C2. **Four seasons, one rule.** Spring (春), Summer (夏), Autumn (秋), Winter (冬); English names stay; the
    picker marks them with the season kanji. Setting `season`: Follow (the default) or a fixed season, plus
    appearance light / dark / AMOLED (black page, surface a third of the way down).
C3. **Follow = the calendar with a 14-day blend**: from 7 days before a season's start date to 7 days after,
    colour channels mixed linearly, nothing animated. Starts 4 Feb, 5 May, 7 Aug, 7 Nov; half a year round
    in the southern hemisphere. Implemented once in `crates/look/src/theme.rs` (`coming_in`, `BLEND_DAYS`).
C4. **Roles.** page, surface, ink, accent, on. Secondary text = ink 65 % over page (the least that reads at 4.5:1 in every palette); outline 36 %;
    hairline 14 %; chrome outline 20-22 %. No other greys.
C5. **Accent = "you, now" only**: the current tab, the song playing, a lit toggle (shuffle, repeat), a liked
    seal, a focused field. Never a decoration, never a fill behind a selected row. `on` is only for a switch
    that is on. On a page about a cover the accent stays the theme's, moved in its own hue until it reads
    (`accent_on`).
C6. **One outline for the chrome** (tabs + player): the page blurred inside it where the platform can
    (24 dp / px blur, page colour over it at 55 %), a hairline round it, no shadow, nothing past its edge.
    Where it cannot blur (terminal, old iPod) the page colour is thicker, never absent. Content scrolls under
    the chrome and fades into the page before it (24 dp ramp, starting late enough that the glass shows).
C7. **The seal**: a favorite is the seal 好. Not liked: outline, upright, quiet ink. Liked: filled with the
    accent, tilted about -5 degrees, stamped on (scale 1.5 to 1, spring damping .5, stiffness 700). Taking
    a like off is immediate; a seal that changes by itself is drawn, not stamped. Reduced motion: no stamp.
C8. **Play / pause is the bare glyph.** No filled plate. A round ripple (ink at ~16 %, grows from half size,
    ~400 ms) is the only circle, on press. The glyph changes by a 180 ms cross-fade with scale from .8; a
    spinner only after 300 ms.
C9. **Nothing animates unless touched.** No idle motion, no polling timers while the window is hidden.
    Honour reduced motion. The chrome changes colour with the page in one 340 ms cross-fade.
C10. **Type**: Inter for everything, Literata (semi-bold/bold, tracking -0.3) only for page titles;
     negative tracking on large titles. Terminal: weights only. iPod: unchanged.
C11. **Icons**: Material Rounded / Filled glyphs, same shapes per action across clients (play, pause, skip,
     shuffle, repeat / repeat-one, home, library, search, settings, album, person, music note, queue, speaker).
     Terminal: the Unicode set in its mockup. iPod: unchanged.
C12. **Layout (Mac, web, terminal)**: no sidebar. A rounded outline across the top: Home, Albums, Artists,
     Songs, Playlists, Favorites (icons + names, current one accent + bold); search on the right, then
     settings (the account and season live there). One outline along the bottom for the player: song on
     the left; shuffle, previous, play, next, repeat with the seek bar under them in the middle; on the
     right: seal, speaker (devices), lyrics, queue, volume. Full player: cover, title with seal, seek,
     transport, volume on the left; Lyrics / Queue / Devices panel beside it.
C13. **Rust first.** Any rule that more than one client needs lives in `crates/look` or the core with a test.
     A client holds only drawing.
C14. **Battery.** No new timers or per-frame work; blur recomputed only when the page behind it is redrawn.

## Process for every agent
- Work only in your own worktree/branch; never touch `~/Documents/Projects/Apps/nori` (the owner's checkout).
- `cargo -j4` at most; heavy commands via `python3 tools/with-resource.py build <cmd>`; shared
  `CARGO_TARGET_DIR=/Users/mateusz/Documents/Projects/Apps/nori-seasons/target` (disk is tight, do not make new target dirs).
- Tick checklist boxes in your checklist file as you finish them (in the same commit as the work).
- Commits: one line, `<type>: <what is different now>`, lowercase, no full stop, under 72 chars, no body, no
  trailers, no `Co-Authored-By`. One commit per piece of work. Commit locally only; never push.
- Docs change with the code (`docs/site/...` when user-visible).
- If a rule in this contract cannot be followed on your platform, do the closest thing and write the
  deviation and why in your checklist's "Design notes".
- Finish by running your client's tests and writing the "Design notes" section; report what you did NOT verify.
