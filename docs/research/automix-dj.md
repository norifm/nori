# AutoMix: a DJ planner and a better Smooth (November 2026)

The plan for rebuilding AutoMix's planner, its analysis inputs and the mixer's effects. Research behind it:
`automix-software.md` (open-source code, products, DJ practice; BitChord read in full) and `automix-papers.md`
(20 papers, 31 rules). "A" is the outgoing song, "B" the incoming one.

## 1. What is wrong today (measured)

On the owner's phone (0.9.0, rock and alternative, DJ style), 20 of 26 transitions in a morning were plain fades and
none had an effect; the echo of the old drop cut went with #148. The lab (§6) on 107 of the owner's songs and 40
popular ones, every ordered pair:

| | Smooth | DJ |
|---|---|---|
| plain fades | 92 % | 38 % (before the echo-out: 83 %) |
| beat-matched | 7.5 % | 17 % |
| why not matched | tempo gap 47 %, no reliable grid 42 % | same |

- **End grids fail on steady music.** Of the song ends whose constant grid failed, about half have beats Beat This!
  tracks at 95–100 % regularity with no drift when it reads the 30 s *before* the song's closing breakdown. The end
  grid was measured over the last 40 s of music, coda and feedback included.
- **One constant grid per end** cannot say where an end drifts, speeds up, turns beatless or holds a clean loop.
- **Unmatched pairs have one answer, a fade.** 21–41 % tempo gaps are common in rock; nothing but a fade (or now an
  echo-out, which flams against B's other tempo when the two overlap) handles them.
- **Smooth's fades are "underwater":** a low-pass from 18 kHz to 500 Hz over the whole fade, the sound Apple dropped
  in iOS 27.
- **Length is capped by fixed numbers**, not by what the two ends allow.

## 2. Principles from the research

- Placement matters most: never cut, echo or brake inside a sung phrase; never put an effect on a fade-out; B lands
  on a phrase start (DJustify: ratings 2.0 → 5.0 when fixed; Bittner).
- Tempo: fold octaves; split a change between both songs, slowing weighed 1.31× speeding (Ishizaki); glide, at most
  about 1 % a beat; listeners object from about ±10 % a song. Beyond what that covers, do not overlap two grooves:
  overlap only where one side is beatless, or hand over with a move whose shared audio has no beat of A's under B's.
- One bass at a time, swapped on a downbeat (instant or one bar); B's highs before the bass; B rises early, A holds
  until the swap; flat summed energy; mids only dip, on a timbre clash or two voices.
- Length comes from the material: locked instrumental blends run long (8–32 bars, more over loops), anything with a
  voice on both sides 4 bars or a handover.
- Variety comes from which sections meet, and from a Markov rule against repeating a marked move; never a random
  shuffle; the same pair always gets the same plan.

## 3. Analysis: each end as a beat map with bar profiles

Each song keeps, for its first and last minute (A's before its closing breakdown):
- **beats and downbeats** (Beat This! where it is sure, the classical tracker otherwise), not one constant grid;
- per bar: loudness, low band (the kick), onset strength (is there a beat), voice level;
- derived, in the planner: steady runs and their local tempo, clean loopable bars (instrumental, steady level, on
  the grid), beatless stretches, the kick at the join, energy at the join.

The constant end grid stays as the summary the planner already reads, measured before the closing breakdown.

## 4. The mixer and engine

New per transition (all allocation-free once configured, all off unless a plan asks):
- **filters on B**: low-pass and high-pass sweeps on the incoming deck ("filter in", the filter ride);
- **mid dip** on both decks (a peaking cut shaped over the overlap) for Smooth's three-band blend and voice clashes;
- **echo**: a send window (open on the beat chosen, closed after), repeats band-limited (about 250 Hz–3.5 kHz), and
  ducked as B lands;
- **reverb** on A's send (a small algorithmic reverb), for an energy drop;
- **brake** (tape stop): A read at a speed falling to zero over 2–8 beats, pitch falling with it;
- **tempo ramp over loops**: A's and B's loops played while one master tempo moves both, so gaps of 10–40 % can be
  crossed with both grooves locked (BitChord's loop mix; split per Ishizaki so neither song strays past half the gap).

## 5. The planner

A pure function, as now: candidates of every technique the pair allows, each scored, the best kept.

| family | technique | needs | style |
|---|---|---|---|
| blend | beat-matched (drop, intro end, phrase landings; double drop; loops; riser) | both grids, gap within the split limit | both (DJ longer) |
| blend | loop tempo ramp | steady grids, clean loops on both, keys not clashing | DJ |
| blend | beatless blend (intro under outro at any tempo) | a beatless end carrying the overlap | both |
| blend | three-band natural fade (bass handed over, mid dip, highs crossed) | nothing | Smooth's fallback |
| handover | filter ride; bass hand-off (two kicks) | A's beat | DJ |
| handover | echo out; filter out | A's beat, an unsung last beat (echo) | DJ |
| handover | brake; reverb wash (marked) | energy at A's exit, an energy drop | DJ, budgeted |
| segue | loudness segue (one-sided, ≤ 6 s), end to end after a cold ending | loudness ends | both |

Score: landing and exit placement, skipped music, voices together, beats together that are not locked, energy cliff
at B's entry, dead air, key roughness over the overlap, length worth (DJ values long locked blends), and a variety
penalty from the previous transition's technique (passed in the settings).

## 6. How it is checked

- Unit tests per technique and gate (`plan.rs`), mixer tests per effect (`mixer.rs`, `no_alloc`).
- The synthetic harness (`eval.rs`) gains DJustify's failure counts: mid-phrase exits, effect tails over B's voice,
  dead air, energy cliffs, unlocked beats overlapping.
- The lab: analyses real songs (classical and Beat This!), plans every pair in both styles, tallies techniques and
  failure counts, and renders transitions through the simulated player for listening.
- The lab's audit renders shuffled runs through the engine and checks the audio itself: holes, dropouts, bumps over
  both songs, clicks, overs and the limiter's extra work, squeezed plans, plain fade-ins, and beats against final0.
  It found, besides beat drift: rides over a dying fade-out (an effect needs its bars alive on average), handovers
  onto a fading-in head (the head now starts under the outgoing song), a near-silent opening entered at its start
  (opening bars under the audible level are skipped like silence), an echo-out whose next song rose over a bar after
  a one-beat fall (it now rises as the outgoing falls), and brakes and reverb washes running past a song's end (the
  engine squeezed them). Overs come only with volume leveling off: leveling turns loud masters down, and the
  limiter runs whenever it may turn a song up.
