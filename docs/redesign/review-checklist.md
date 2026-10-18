# Cross-client design review: checklist

Run by the lead after the clients land. For each contract rule C1-C14 (docs/redesign-contract.md) compare every client's
"Design notes" and the code, and note: same / differs-ok (platform) / differs-bad (fix).
- [ ] C1 palettes from Rust everywhere (grep for hard-coded season hexes in desktop, cli, web, ios)
- [ ] C2/C3 season setting + 14-day blend used by all (no second implementation of the calendar)
- [ ] C5 accent only for current things (grep accent fills)
- [ ] C6 chrome: top pill + player pill; same outline/blur/tint/fade constants (24 / 55 % / 24 / hairline 20-22 %)
- [ ] C7 seal states and tilt/stamp; C8 bare play glyph + ripple
- [ ] C10 fonts; C11 icons identical per action
- [ ] C12 tab order/names and player-pill order identical (Mac, web, terminal)
- [ ] Screenshots side by side for each season/appearance; list deviations
