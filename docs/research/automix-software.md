# Transition techniques in automix software, products and DJ practice

Internal research note, not published. Written 2026-11-13 for nori's AutoMix planner (`crates/player/src/automix/`
in nori-dj: `plan.rs`, `mixer.rs`, `types.rs`). It builds on `automix.md` and `automix-next.md` and does not
repeat them. A second note covers academic papers; this one is the software side: source code that was read,
product documentation, and DJ practice.

Labels: **[code]** read in the source, with file and line (repositories cloned 2026-11-13, commit given once
per project); **[doc]** read on a product or project page; **[snippet]** only seen in a search result;
**[inferred]** my reasoning. GPL and AGPL code (Mixxx, BitChord, heydavidf/automix, Orchard) was read for facts and
numbers only; nothing is to be copied.

The problem this note answers: on the owner's rock/alt playlist 85 % of pairs cannot be beat-matched (tempo gaps of
21–41 %, live drummers drifting), and every one of them became a plain fade. Part A is what each project does;
Part B is the catalog of techniques with recipes; Part C is the ranked recommendation.

---

# Part A. What each project and product does

## A1. Mixxx Auto DJ (GPL-2.0+)

Commit 20f2c218 (2026-11-10), `src/library/autodj/autodjprocessor.cpp` **[code]**.

- **Five transition modes** (`autodjprocessor.h:168-174`; UI names `dlgautodj.cpp:177-185`):
  - *Full Intro + Outro* (default, `autodjprocessor.cpp:1474-1520`): transition length is the shorter of the
    outgoing outro and the incoming intro; the fade is placed so it **ends at the outro end**, so the end of the
    incoming intro lines up with the end of the outgoing outro and nothing of either is cut. If neither is marked
    it uses the fixed time from the spin box.
  - *Fade At Outro Start* (`:1521-1570`): the fade **starts at the outro start**; when the outro is longer than the
    intro, the rest of the outro is cut.
  - *Full Track* (`:1595-1609`): fixed-length fade ending at the file's end; the next track from 0:00.
  - *Skip Silence* (`:1576-1593`): fixed-length fade ending at the last sound; the next track from its first sound.
  - *Skip Silence Start Full Volume* (`:1572-1575`): as Skip Silence, but the crossfader jumps to the centre when
    the fade starts (`m_crossfaderStartCenter`, `:901-902`), so both decks are at full volume and then the
    outgoing one is pulled down.
- **Cue placement.** Intro start and outro end are placed automatically by the silence analyser at the first and
  last sample above **−60 dBFS** (`src/analyzer/analyzersilence.cpp:11`, `:141-190`); intro end and outro start
  are only ever set by the user. Without an intro end, Mixxx takes intro start = intro end − transition time
  (`autodjprocessor.cpp:1223-1239`); without an outro start, outro end = outro start + transition time
  (`:1268-1291`). Default transition time is **10 s** (`:17`). A **negative transition time inserts a gap** of
  that many seconds instead of an overlap (`:1673-1678`; the UI tooltip says so, `dlgautodj.cpp:142`).
- **Fade curve.** Auto DJ only moves the crossfader **linearly with the outgoing track's play position**
  (`:957-966`). The audible curve is the mixer's crossfader curve (`src/engine/enginexfader.cpp:16-80`): an
  additive or constant-power law with a user "transform" exponent from 0.6 to 1000 (default 1.0); the
  constant-power mode normalises `g1² + g2² = 1`, chosen because ReplayGain tests on 30 s snippets of many genres
  measured the crossover near 0.707, i.e. uncorrelated (`:59-70`).
- **Tempo mismatch.** Auto DJ does **nothing**: no sync, no EQ, no effect (the processor has no tempo or EQ code
  at all). Users turn on sync lock themselves. It is a radio-style automation, not a DJ.
- **Effects in Mixxx that matter as reference numbers** (`src/effects/backends/builtin/`): Echo delay 1/8 to 2
  beats, default **1/2 beat**, feedback default **−3 dB (0.71)**, send default −3 dB, quantised, optional
  triplet and ping-pong (`echoeffect.cpp:43-106`); Filter Q 0.4–4, default 0.707 (`filtereffect.cpp:52`);
  Reverb decay default 0.5 (`reverbeffect.cpp:33`).

**Lesson:** the default automation keeps every bit of both songs and lines the end of the incoming intro up with
the end of the outgoing outro. That is the right rule for Smooth when cues exist, and nori's `drop_aligned`
search already generalises it.

## A2. lenvdv/auto-dj (Vande Veire, drum and bass)

Commit a34ac0e0 (2017-09-08), `Application/` **[code]**. Master's thesis code (Ghent, 2017), all songs
time-stretched to a fixed **175 BPM** (`djcontroller.py:207`), so tempo mismatch never arises; ±1 semitone pitch
shift to fit keys (`tracklister.py:479-490`). Lengths are in **downbeats = bars**.

**Three transition types** (`tracklister.py:17-32`) and how they are placed:

| Type | Outgoing (master) cue | Incoming (slave) cue | Fade-in / fade-out length |
|---|---|---|---|
| **double drop** | the bar before an L→H boundary (its drop), so both drops land together (`:82-86`) | the bar before its own drop (`:119-122`) | in ≤ 16 bars, out **32 bars** after the drop (`:28-30`) |
| **rolling** | 16 + 16 bars before an H→L boundary, i.e. inside the outgoing song's high section (`:88-92`) | its drop (`:123-126`) | in ≤ 16, out ≤ 16 (`:27-28`) |
| **relaxed** ("chill") | an H→L boundary (its breakdown) (`:94-98`) | the song's start, 16 bars of intro (`:127-130`) | in 16, out 16 (`:31-32`) |

H and L segments come from a novelty segmentation where a segment is H when its RMS and onset strength after the
boundary are both above the song's mean (`structuralsegmentation.py:177-203`). Every cue is moved onto an 8-bar
phrase line (`tracklister.py:150`).

**Choosing the type** is a Markov chain (`tracklister.py:21-25`): after relaxed → 0 % relaxed, 70 % rolling,
30 % double drop; after rolling or double drop → 20 % relaxed, 80 % rolling, 0 % double drop. A type falls back to
the next when its cue points do not exist (`:157-194`). The next song is chosen among key-compatible songs by
onset-curve similarity over the overlap (DTW on 4-bar pieces, `:213-271`, `:431-444`) and rejected if singing
overlaps in two or more downbeats (`:44-47`, `:447`).

**Curves** (`songtransitions.py:173-208`). P is the switch point (end of the fade-in), L the length, both in bars.
Volume values are power fractions: the gain is `sqrt(linspace(v0, v1))` (`:44-50`). EQ values map to shelf gain
`−26·(1 − v)` dB: 1 → 0 dB, 0.8 → −5 dB, 0.3 → −18 dB, 0.2 → −20 dB, −1 → −52 dB (`:52-85`); the low shelf is at
**70 Hz**, the high shelf at **13 kHz**, Q 0.707, updated in 20 steps per segment (`:58-62`). There is no mid
band.

- *Rolling and double drop* (`:191-198`), time points `[0, 1, P/2, P, P+1, P+(L−P)/2, L]`:
  - outgoing volume 1, 1, 1, 1, **0.8**, 0.8, 0 (stays full through the fade-in, a 1 dB dip after the switch,
    then out over the second half of the fade-out);
  - outgoing lows 0 dB until P, then **−52 dB one bar later** (a one-bar bass swap);
  - outgoing highs 0 dB until P, −20 dB at P+1, −52 dB at the middle of the fade-out;
  - incoming volume 0, 0.2 after one bar, **0.8 by P/2**, held, 1 at P+1;
  - incoming lows −52 dB until P, 0 dB at P+1 (the other half of the swap);
  - incoming highs −52 dB until P/2, then rising to −20 dB at P and 0 dB at P+1.
- *Relaxed* (`:199-208`), time points `[0, P/2, P, P, P+(L−P)/2, L]`:
  - outgoing volume full until P, 0.7 at the middle of the fade-out, 0 at the end;
  - outgoing lows and highs step to −18 dB at P, then ramp to −52 dB by the middle of the fade-out;
  - incoming volume 0 → 0.7 over the first half of the fade-in, 1 by P;
  - incoming lows and highs −52 dB until P/2, ramp to −18 dB at P, then step to 0 dB (a gradual swap).
- *Very short fades* (P < 2 or L − P < 2, `:175-188`): a straight swap at P, both volumes 0.8 at the crossing.
- *Loudness balance* (`:264-280`): the sum is scaled by `1/sqrt(v1² + v2²)` so the total power stays 1.

**Lesson:** the three types are three **placements** of one EQ-swap blend, chosen by structure (drop with drop,
drop into a playing high section, intro under a breakdown), with a Markov rule that avoids repeating the dramatic
one. Variety comes from *where* the mix sits, not from different effects. All of it assumes a common tempo.

## A3. BitChord (the owner's "bitchord"): its own section

**What it is.** BitChord is an open-source (GPL-3.0) YouTube Music client for Android, with a TV, Wear OS, iOS
port and web player, by Kushagra Singh: <https://github.com/kushagrasinghx/BitChord> (commit acf219a7,
2026-11-06) **[code]**. It has a wide fork and add-on ecosystem (BitChord-iOS, BitChord-Desktop-Revived,
AuraMusic "based on Metrolist & BitChord", hi-res and self-hosted add-ons) per a GitHub search on 2026-11-13. Its
README lists "**Automix [Beta]** — DJ-style transitions with beat-matching and tempo-stretching" (`README.md:49`).
The earlier research (`automix.md` §7.1) read an older clone; this is the current code.

**Settings** (`app/src/main/res/values/strings.xml:269-280, 904-907`): Crossfade 0–12 s; **Automix [BETA]** ("Times
and blends every transition from the tracks themselves"); **Advanced Automix** ("Mixes like a DJ: keeps the beats
locked, makes each move on the beat, and cuts between songs that won't blend. No extra processing."); Automix
performance Efficient / Balanced / Performance (1, 2 or 4 analysis threads; `AutomixPerformanceMode.kt`). No
style picker: the style is chosen per pair.

**Styles** (`shared/.../smart/TransitionPlanner.kt:134-164`):

| Style | When | What it does |
|---|---|---|
| `GAPLESS` | an album played through | near-instant hand-off, filters open |
| `EQUAL_POWER` | weakest evidence | cos/sin fade; still separates two voices if both sing (`CrossfadeController.kt:3172-3186`) |
| `DJ_BLEND` | tempos within 5 % on two grids, or phase-lockable (`TransitionPlanner.kt:1715-1720`) | beat-aligned blend with a bass swap; incoming fader fully up by the swap, outgoing held up until it |
| `DJ_FILTER` + `SWEEP` | tempos too far apart, no two kicks at the join | complementary filter ride (below) |
| `DJ_FILTER` + `BASS_HANDOFF` | tempos too far apart and **both sides carry a kick** at the join (low-band energy ≥ 0.5 of each song's loud end over 8 beats, `TransitionPlanner.kt:289-316`) | a quick one-kick bass exchange, then a shallow exit filter |
| `DJ_FILTER` + `ECHO_RIDE` | **never chosen**: "a beat echo at the outgoing tempo flams over an incoming track at another one" (`:296-297`); the planner sets `echoSeconds = 0.0` (`:1836`) | — |
| **loop mix** | a pair with usable grids and instrumental bars on both sides (below) | two loops, one locked tempo ramp |

Unmixable pairs: tempos no speed-up can meet **plus** a key clash or a vocal on both sides get a plain crossfade at
the content end (`TransitionPlanner.kt:269-286`). Spoken word and anything *labelled* live is never mixed
(`:94-130`). The incoming song may be sped up by at most 25 % and slowed by 10 % (`TransitionPolicy.kt:69-70`).

**The filter ride for unmatched tempos** (`app/.../CrossfadeController.kt:3299-3322`, constants `:3946-4235`):
- outgoing low-pass from **7 kHz** down to a **300 Hz** floor, shaped `progress^0.75` (front-loaded: the top is gone
  in the first tenth; earlier versions with exponent 2.0 and 1.3 left two voices clashing through the first half,
  `:4095` and its comment);
- incoming high-pass from **1.2 kHz** (2.4 kHz when both sing) opening by a set share of the blend;
- the low end changes hands **once, on a beat**, at the middle (`FILTER_BASS_SWAP_AT = 0.5`), over **half a beat**
  (`SWAP_BEATS`), by a 24 dB/octave high-pass at **200 Hz** (`BASS_SWAP_HZ`) moved onto the other deck, "so only one
  kick is ever playing";
- faders: equal-power pair with shaped progress, outgoing `cos(π/2·p^1.6)` (held up, the low-pass already takes it
  away), incoming `sin(π/2·p^0.8)` (comes up early behind its high-pass) (`:3502-3540`; constants `:3985-3986`);
- filters are two-stage (24 dB/oct) TPT filters with a ~30 ms glide (`TransitionFilterProcessor.kt:488-515`).

`BASS_HANDOFF` (`CrossfadeController.kt:3332-3356`): the bass swaps at **0.43** of the blend, the incoming
high-pass from **900 Hz** opens by 0.38, and only after the swap the outgoing deck darkens to a **1.6 kHz**
low-pass. Lengths: the filter ride is **12 beats** for SWEEP and **8 beats** for BASS_HANDOFF (`TransitionPlanner.kt:1302-1309`),
capped at **8 s / 7 s** (`:1730-1734`), "deliberately compact and vary by gesture; the old 10-12 second sweep made
every fallback sound like the same ordinary crossfade" (`:1298-1301`).

**The loop mix** (`shared/.../smart/LoopMix.kt`, renderer `native/loopmix/loop_mix_renderer.cpp`, envelopes
`app/.../loopmix/LoopMixSession.kt:191-221`). This is the "DJ loop mixing approach" from issue #144: it is how
BitChord mixes two songs at **any** tempo gap.
1. **Pick the loops.** Outgoing: the **latest** clean loop in the last **60 s**; incoming: the **earliest** clean
   loop in its first **45 s** (`LoopMix.kt:96-97, 182-201`). Lengths tried 4, then 2, then 1 bar (`:49`): "a 1-bar
   instrumental loop beats a 4-bar one with a word in it". A loop runs from a detected downbeat to the detected
   downbeat *n* bars later (Beat This! `final0`, int8), and is skipped if that lands more than a quarter beat off
   (`:262-285`). Clean means: every bar at least **0.6** of the 60th-percentile loop's quietest-bar RMS (not a
   breakdown or fade), and **no singing** — no synced lyric line (padded 0.15 s before, 0.35 s after, and 0.3 s of
   clear time before the loop), else the vocal model finds at most **3 %** sung frames and no sung run over
   **0.2 s** (`:65-93`, `:324-377`).
2. **Tempo.** One master tempo, in outgoing beats per minute, **holds for 4 bars** while the incoming loop comes
   in (`ENTER_BARS`), then **ramps linearly to the incoming tempo at most 2.5 BPM per bar**, for 2 to **24 bars**
   (`:60-63`, `:208-210`, `:484-488`); it folds to double or half time when that is closer (111 → 169 BPM ramps to
   84.5; web doc `web/docs/automix.md`). Both decks follow the master tempo **beat by beat** through Signalsmith
   Stretch (pitch kept), each deck driven by one position knot per beat, so the loops stay locked "however far apart
   they started" (`loop_mix_renderer.h:1-12`). 24 bars × 2.5 BPM = 60 BPM of travel: this is how it covers
   30–40 % gaps that no single stretch ratio can.
3. **Exit.** After the ramp, **2 bars** (`EXIT_BARS`) for the outgoing loop to drain, rounded up so the incoming
   loop releases at its own boundary; then the incoming song plays on from its loop end at native tempo, and its own
   player takes over **2 bars** later (`SETTLE_BARS`) with a 25 ms crossfade on identical audio (`:51-58`, `:99-100`).
4. **Envelopes** (`LoopMixSession.kt:191-221`): outgoing gain 1 through the entry, **0.6** at ramp end, 0 at release;
   incoming gain 0 → **0.55** over the 4 entry bars, 1 at ramp end (linear values). Bass: a **250 Hz low shelf at
   −18 dB** (`loop_mix_renderer.cpp:250`, `LoopMix.kt:101`) on the incoming deck, swapped over **one bar** centred
   on the middle of the ramp. Outgoing high-pass from **20 Hz to 1.5 kHz**, exponential, from ramp end to release
   (it "drains away through a rising high-pass").
5. **Cost and length:** the join gets longer by the ramp ("that's intended", web doc). A 120 → 160 BPM pair (+33 %):
   16 bars of ramp + 4 + 2 ≈ 22 bars, roughly 40 s [inferred from the constants].

**Why it suits nori's problem.** It is the only open-source code found that mixes 21–41 % gaps *with the grooves
locked*: neither song has to hold a fixed stretch, because each plays a short loop and only the master tempo
moves. It needs a grid at the outgoing end and the incoming start, and instrumental bars at both (loops of one bar
are allowed). Its risk is the same as Apple's iOS 27 loops: a loop repeated for 20+ bars is "broken record" if the
bar is not a groove. BitChord keeps the ramp at ≤ 2.5 BPM/bar ("past this a ramp is heard as a lurch, not a mix",
`LoopMix.kt:60`).

**Also in BitChord:** a **phase-lock servo** that nudges the incoming speed by **3 %** in pulses of at most 1.5 s
when the beats drift more than **8 ms** apart, never past 0.3 beat (then the grids disagree), and stops at 90 %
of the blend (`CrossfadeController.kt:4022-4048`); a tempo change limit of **0.75 % per beat** "below about 1 % a
beat reads as the groove, not as the track changing speed" (`:4055`); echo when used: 1-beat delay (≤ 1 s),
feedback **0.4**, wet **0.5**, band-limited **250 Hz–3.5 kHz** in the loop, **5 repeats**, the dry signal fading
over **4 beats** after the echo beat because a quarter-beat kill "was heard as the song cutting out"
(`TransitionFilterProcessor.kt:518-531`, `CrossfadeController.kt:4005-4013`).

Changelog or Play Store page: not found in the repository (no CHANGELOG file); the web player's doc
(`web/docs/automix.md`) is the only narrative, dated by its measurements (2026-10-23).

## A4. heydavidf/automix (the engine BitChord's web player ports, AGPL-3.0)

<https://github.com/heydavidf/automix>, commit f4eb8cb8 (2026-10-27), `src/lib/engine/transition-scheduler.ts`
**[code]**. A browser player that DJs a local folder.
- Beat-matched blend only when both grids have confidence ≥ **0.2** (`:36`) and the folded rate is within
  **±8 %** (`:86-91`); else a plain **12 s** equal-power crossfade from the incoming intro end (`:157-178`).
- **Blend length scales with grid confidence**: the user's bars are a maximum; min confidence ≥ 0.6 → as asked,
  ≥ 0.45 → ≤ 8 bars, else ≤ 4 bars (`:48-57`) — "a wrong lock costs ~11s, not a 46s blend".
- Mix-out on the phrase before the outgoing outro; mix-in so the incoming **full-energy phrase lands as the blend
  ends** (`:96-125`).
- Equal-power cos/sin gains; **bass swap over one bar at the midpoint**, **−12 dB** on blends of 8 bars or more,
  **−6 dB** on shorter ones "so piano-led lows aren't gutted" (`:37-41`, `:134-143`); highs cross −8 dB over the
  whole window; 3-band EQ is a 250 Hz low shelf, 1 kHz peak (Q 0.6), 4 kHz high shelf (`eq.ts:3-13`).
- After the blend the incoming rate steps back to 1 in **8 steps of 3 s** (~0.8 % each) (`:197-209`).

## A5. pyCrossfade (MIT)

<https://github.com/oguzhan-yilmaz/pyCrossfade>, commit 5f85f4a6 (2026-08-09) **[code]**.
- The **outgoing** song is stretched **bar by bar** to the incoming tempo over the 8 bars *before* the crossfade
  (`len_time_stretch = 8`, factors `linspace(1, final)`), so by the fade the tempos already match
  (`pycrossfade/transition.py:81-104, 185-241`; `config.py` `CrossfadeSettings`).
- 8-bar crossfade with a 3-band EQ crossfade (`utils.py:248-278`): low shelf **70 Hz** and high shelf **13 kHz**
  at `−26·(1 − level)` dB on each deck, plus a **mid dip of −6 dB·sin(πt)** at 1 kHz on **both** decks at the
  middle "to avoid a muddy blend"; volume equal-power from 0.9 → 0 and 0.1 → 1 (`config.py` `FadeSettings`).

## A6. musicmix-dj (Apache-2.0; hip-hop and rap focus)

<https://github.com/Jacopo888/musicmix-dj>, commit 41bf0fdd (2026-10-22) **[code]**. A research toolkit with the
most complete *non-electronic* technique table found, every rule tagged with its source (DJ books: Brewster &
Broughton, Webber, Butler, Katz; music psychology: Huron, Margulis) and its owner's listening scores [U].
- **Listening scores** (one listener, 6 clips, `conoscenza_mix/IMPLEMENTAZIONE_V11.md:279`): filter out and echo
  out on a phrase start **9/10**; an 8-bar blend **8/10**; another echo out **7/10**; a dry drop straight onto the
  incoming chorus **3/10**. Their conclusion (`:286`): filter-out and echo-out "empty the texture before B enters",
  remove vocal overlap and need no long beat-match, "so they are the defaults when tempo, key or voices prevent a
  blend".
- **Policy table** (`src/musicmix/dj_policy.py:120-165`):
  - *far tempo* (beyond ±8 %, `max_stretch` 0.08 citing a ±10 % discomfort study): **echo_freeze**; **note_bridge**
    (one bar of a pad in the shared key) when keys fit; **backspin** when the outgoing song sings or pushes at the
    exit; **tape_stop** when the incoming one is calmer. Length: **1 bar of effect + 1–2 bars of tail**.
  - *energy drop* (incoming > 6 dB quieter): reverb_wash or tape_stop.
  - *default*: echo_freeze, reverb_wash.
  - *same tempo*: loop_extend (1–2 instrumental bars of A looped under B's entry, 8 bars if stretch ≤ 5 % else 4),
    acapella_over and vocal_swap (stems, 4–8 bars), build_into_chorus (HPF + snare roll + riser in B's key,
    2–4 bars), lyric_link (echo of A's last word landing on a matching word of B).
  - a **repeat penalty of 0.15** per technique used recently and an optional budget penalty for "marked" effects:
    "Effects should be your secret weapon, not your constant companion" (Brewster & Broughton, ch. 29).
- In every no-overlap technique, **B enters clean on its own downbeat at its own tempo**; the two grooves never
  play together.
- **Recipes:**
  - *echo_out* (`dj_effects.py:84-92, 356-517`): delay **½ or 1 beat**, feedback **0.5**, echo high-passed rising
    **150 Hz → 2.5 kHz**, repeats until −48 dB (max 16), B on a downbeat 0, 1 or 2 bars after the exit, tail
    **−12 dB** under B; micro-fades of **10 ms** ending on the downbeat.
  - *echo_freeze* (`dj_creative.py:1738-1817`): one bar before the exit the dry signal is cut and replaced by a
    **spectral freeze** of the last beat (mean STFT magnitude, random phase) at 0.7 plus a **1-beat echo at feedback
    0.6**, high-passed **150 Hz → 1.5 kHz**, at −3 dB against A; B enters clean on the downbeat; the freeze drops
    **12 dB** and fades out over **1–2 bars of B**.
  - *filter_out* (`dj_effects.py:93-108, 518+`): high-pass **30 Hz → 1.2 kHz**, geometric, 4th order, over
    **1–4 bars**, ending at the exit downbeat, then B; optional *filter_in* on B, low-pass **250 Hz → 18 kHz**
    ("filter swap"). Risk noted: "filter too slow = mud".
  - *reverb_wash* (`dj_creative.py:1818-1882`): send rising `u^1.5` over **1–2 bars**, RT60 **3.5 s**, 20 ms
    pre-delay, wet −3 dB, wet high-passed at **250 Hz**; A's dry cut on the exit downbeat; tail −12 dB under B,
    fading over **1–4 bars**.
  - *tape_stop* (`:1963-1985`): speed `(1 − u)^1.5` over **2 beats** (1, 2 or 4), pitch falling with it, gain
    `sqrt(min(1, 4v))`, the fader pulled over the last beat; B clean on the downbeat.
  - *backspin* (`:1934-1962`): the platter stopped in 40 ms, thrown backwards to **3×** speed in 80 ms, slowing
    exponentially to zero over **2 beats**; amplitude ∝ √|speed|.

## A7. DARAVE (MIT; Windows DJ-set tool)

<https://github.com/kam1k88/DARAVE>, commit 3e8dc7b4 (2026-11-12) **[code]**. A large technique registry
(`techniques.py`, ~60 techniques) and an explicit argument *against* variety for its own sake
(`mix_strategist.py:420-437`, translated): "A DJ runs a set with ONE main move: the new track's intro under the
old, a bass swap, a filter out. A fader cut, a double drop, a drop tease are performance, done deliberately, once a
set, on a pair that asks for it. An algorithm that sprinkles them 'for variety' sounds exactly as it should:
ignorant." Variety, it argues, comes from **which sections meet** (drop to drop, breakdown to drop, body to body),
measured as 8.1 different section pairings per random pair on a 66-track library (`:505-520`).

Only two exceptions leave the main move:
- **tempos that do not meet** (`:439-452`): ranked *Echo Tail + Spinback* (TT-07; echo 4 beats, spin 1 beat at 9×,
  the next track on the next "one"; "the echo holds the harmony when the track is already gone, so it works on
  clashing keys and unmatched tempos"), *Tape Stop* (TT-02; **2 beats**, curve 1.6, at a phrase line, "the only
  honest way to change tempo"), *Quick Cut* (DNB-02; a crossfade of **½ beat** at a phrase boundary), *Beatmatch
  Cut*. Its comment: "a cut with nothing is heard as a mistake, echo and turntable as a decision".
- **hard key clash**: *Echo Cut* (DNB-07: the incoming mids killed, it enters with bass and highs only for N bars;
  echo on the outgoing last note; the outgoing fader cut; the incoming mids opened on its drop), *Reverse Roll*
  (TT-04: the last 4 beats of the outgoing song reversed with slip).

Main move *Classic Blend* (DNB-25, `techniques.py:958-975`): incoming intro under the outgoing song with its lows
cut, 8 bars (2–32), one bass swap, the old one led out with a filter and echo, "4–12 bars, like a live DJ". Out-FX
presets (`fx_presets.py:54-107`), as a share *b* of the transition's beats: *Delay Tail* (echo send 0 → 0.75 over the
last 40 %, delay **¾ beat**), *Filter Up* (HPF over the last 75 %, ease-in), *Reverb Out* (house/techno/trance/ambient
only), plus flanger, phaser, overdrive, bitcrusher limited to genres.

## A8. MORA and BeatBridge

- **MORA** (<https://github.com/Rizen1322/MORA>, commit 181a60c6, `packages/core/src/plan/unlocked.ts:1-80`)
  **[code]**: echo out only for octave-folded gaps of **25 % or more** ("research: echo out for >~30 %"); grids
  must be stable (confidence 0.6 / 0.45). Recipe: the outgoing song finishes a sung phrase; its **next bar goes
  into a one-beat echo, bass removed first**; it stops on the downbeat; the incoming song starts on its own start
  or a downbeat that skips only instrumental intro, **late enough that at least a bar of instrumental separates the
  last outgoing line from the first incoming one**; "the two grooves never play together". It refuses when the
  combined quiet "valley" would exceed **5 s**.
- **BeatBridge / AuraDJ** (<https://github.com/WalterByte-afk/BeatBridge>, MIT, `offscreen.js:914-1060`)
  **[code]**: "vibe" transitions chosen by energy: *Tape Brake* (high-energy into soft: playback rate 1 → 0.05
  exponentially in **650 ms**, the tail caught in a 420 ms delay at feedback 0.85 through a 750 Hz band, decaying
  over 3.5 s; the soft song swells in over 450 ms), *Half-Time* (2:1 tempos: outgoing LPF 20 kHz → 2.4 kHz over a
  bar, dotted-eighth delay on the half-time grid, incoming LPF 300 Hz → 20 kHz over 3 bars, bass swap at bar 2),
  *Riser Drop* (soft into high-energy: outgoing bass cut in 0.8 s, a **band-pass Q 7 swept 400 Hz → 4 kHz** over 2
  bars as a riser, bass swap at the end).

## A9. DJtransGAN (MIT code, ICASSP 2022)

<https://github.com/ChenPaulYu/DJtransGAN>, commit 64228931 **[code]**. The generator outputs, per deck, fader and
3-band EQ curves; bands are split at **20 / 300 / 5000 / 20000 Hz** (`djtransgan/config/settings.py:14`), the cue
region is **8 bars** (`:22`). Each curve is a sum of clipped ramps (`prelu6(x − start)·slope`, `mixer/fader.py:23-27,
95-98`): a learned **start time and slope per band**, i.e. "when each band moves and how fast". This is the shape
space a learned controller would fill; the DJ data behind it is EDM.

## A10. Liquidsoap: loudness-driven crossfades

- **`cross.smart` / `crossfade(smart=true)`** (Liquidsoap 2.2, `src/libs/fades.liq` at tag v2.2.5, lines 682-805)
  **[code]**: looks at the levels at the end of A and the start of B (`width` window) and picks one of five
  transitions; defaults `high = −15 dB`, `medium = −32 dB`, `margin = 4 dB`, fades 3 s, sine shape:
  1. both ≤ medium and within the margin → full crossfade (both fade);
  2. B ≥ A + margin, A ≥ medium, B ≤ high → **only A fades out**, B starts at full;
  3. A ≥ B + margin, B ≥ medium, A ≤ high → **only B fades in**, A plays on at full;
  4. B ≥ A + margin and A ≤ medium → no fade at all, just overlap ("do not fade if it's already very low");
  5. otherwise (both loud) → **no crossfade, sequence** ("too loud to overlap nicely").
  In 2.4 the operator is the simple `crossfade` and cue points come from **autocue** instead.
- **autocue** (`src/libs/autocue.liq`, current master) **[code]**: from EBU R128 loudness per frame, relative to the
  track's integrated loudness: cue in where it first exceeds **−34 dB** (with 0.1 s margin and a **0.2 s** fade-in
  "to avoid hard cuts"), cue out where it last exceeds **−42 dB**, **crossfade start where it last exceeds −7 dB**,
  overlap capped at **6 s** (`:57-130`, `:486-524`), plus "sustained ending" detection that lowers the thresholds
  when the ending stays loud (`:87-118`). Loudness target −14 LUFS.

These are the radio rules nori's MixRamp fade already follows; the useful additions are rule 2–3 (fade only the
quieter side, the loud side plays at full) and autocue's −7 dB "start the next song here" point.

Also read: **Orchard** (now PolyForm Shield, `crates/earmark/src/types/transition.rs:15-29`, commit 150a08f) has five
strategies: equal-power crossfade, beat-matched crossfade, bass swap, filtered blend ("the outgoing track is
progressively low-passed away while the incoming track opens up", when the spectra fight) and a short fade for
material that cannot be matched. Nothing beyond what nori has.

## A11. Products

From a web pass on 2026-11-13 (product pages, help centres, release notes, reviews). Where a fact comes from a
third party, that is said.

- **Spotify Mix** (Premium playlists, beta since Aug 2025). Spotify's help page names three lanes per transition:
  *Volume*, *EQ* (lows, mids, highs) and *Effects* ("currently low-pass and high-pass filters")
  ([support.spotify.com](https://support.spotify.com/article/mixed-playlists/)) **[doc]**. Preset recipes, from a
  third-party guide ([lettmusic](https://www.lettmusic.com/blog/how-to-arrange-spotify-playlists)) **[doc]**:
  - **Auto**: Spotify's choice from key and tempo;
  - **Fade**: a standard crossfade with the bass swapped around the midpoint;
  - **Rise**: an overlap with the bass swap at the end, a low-pass on the incoming song and a high-pass on the
    outgoing one;
  - **Blend**: an overlap with a smooth 3-band EQ fade;
  - **Wave**: an overlap with the bass swap at the centre, low-pass in and out;
  - **Melt**: the incoming fades in as the outgoing fades out, bass swap at the centre, high-pass in and out;
  - **Slam**: a hard, centred volume swap.

  The building blocks:
  - volume styles "Smooth crossfade, Overlap, Fade in fade out, Cut in fade out, Fade in cut out, Center cut, and
    Crossfade";
  - EQ styles "Center bass" (default), "End bass swap", "3-band fade", "None"
    ([maketecheasier](https://maketecheasier.com/spotify-mix-dj-for-playlists/));
  - curves editable on mobile ([Engadget](https://www.engadget.com/2216279/how-to-use-spotify-mix-tool/)).

  A clone reverse-engineered from the desktop client 1.3.4 ([vimj PR #2](https://github.com/lorenzolfm/vimj/pull/2))
  **[doc, third party]** lists:
  - three more presets, **Dissolve, Surge, Afterglow**, whose recipes come from Spotify's servers;
  - 8 volume, 9 EQ and 11 filter styles, including "High-pass filter half out" and "**Noise out end**";
  - 12 effects, all on the outgoing song: reverb out at the centre, cut at the end, out at the end; delay ½ and ¾;
    echo ½, ¾ and 1 beat (a delay repeats once, an echo keeps repeating);
  - outgoing **loops of 1–16 beats and "loop halving"**.

  Length is 2, 4 or 8 bars, offered only when a clear beat grid is found
  ([Tom's Guide](https://www.tomsguide.com/entertainment/music-streaming/forget-spotify-lossless-this-feature-is-the-only-reason-i-havent-cancelled-my-subscription-yet)
  **[snippet]**; lettmusic). **Unmatchable pairs:**
  - Spotify warns that mixing isn't possible (Tom's Guide **[snippet]**);
  - when forced, it ramps tempo "from 100 BPM to 120 BPM over a few seconds" with glitches, and "for slow songs
    without a set BPM or beat, it sounds horrific"
    ([Spotify Community](https://community.spotify.com/t5/Content-Questions/Feature-request-Tempo-matching-switch-in-Mix-mode/m-p/7517345/highlight/true)).
- **djay / djay Pro** (Algoriddim). Automix transitions: "Automatic, Fade, Filter, EQ, Echo, Dissolve, Neural Mix,
  and more" ([help, Mac](https://help.algoriddim.com/user-manual/djay-pro-mac/settings/automix)); the Windows page
  adds **Tremolo** and **Riser**
  ([help, Windows](https://help.algoriddim.com/user-manual/djay-pro-windows/mixing-basics/using-automix)).
  - Duration: automatic, or bars or seconds.
  - Tempo adjust: *Off*, *Sync* (one BPM throughout), *Sync + Tempo blend* ("gradually adjusts the outgoing track's
    tempo to match the incoming one"), *Automatic*.
  - Since 5.x Automix uses the Crossfader FX set, adding **Drum Cut**, **Vocal Cut**, **Neural Mix Echo Out** and
    **Neural Mix Fade** (stems), with a "Shuffle" that picks the next FX, locked to a 4-beat sync interval
    ([release notes](https://www.algoriddim.com/djay-pro-windows/releasenotes)).
  - Descriptions ([2021 press release](https://algoriddim.com/press_releases/435-algoriddim-announces-major-update-to-djay-pro-ai)):
    "Dissolve delicately evaporates the outgoing track"; "Riser … steadily creating energy before swapping the
    tracks"; "Echo gives you a fast mix into the new track while the outgoing one bounces off into the distance".
  - "The Automatic transition acts like a shuffle, randomly selecting different transition types" (Windows page).
  - For a tempo gap "djay adjusts the BPM to match the currently playing song"; no half-time rule and no
    no-sync fallback are documented beyond setting tempo adjust to Off.
- **Apple Music AutoMix**. Nothing new since `automix.md` §1a: iOS 27 has "new transition types", none named
  ([MacRumors](https://www.macrumors.com/2026/06/09/apple-music-gains-automix-upgrades-and-more-in-ios-27/)).
  Incompatible tempos or genres get no dynamic transition ([support](https://support.apple.com/en-gb/105067)).
- **VirtualDJ** ([manual](https://www.virtualdj.com/manuals/virtualdj/interface/browser/sideview/automix.html))
  **[doc]**. Automix types:
  - *Smart*: finds the best mix point, "mix time varies from ~4 to ~8 secs";
  - *Fade (remove Intro/Outro)*, *Fade (remove silence)*, *Fade (remove nothing)*;
  - *Fade out, Cut in (remove silence)*, radio style;
  - *None (back-to-back)*.

  Settings:
  - `fadeLength`, where a negative value inserts a gap;
  - `autoMixBeatMatchOnFade`, which beat-matches only when tempos are close;
  - `automixTempoMode` keeps the BPM "when possible within a pitch of +-10%".

  Forum reports say distant BPMs snap to half or double
  ([forum](https://virtualdj.com/forums/233607/General_Discussion/BPM_issues.html) **[snippet]**).
- **rekordbox 7.2** Automix has only playlist options; MIX POINT LINK starts the incoming track at a chosen cue
  ([manual pp. 166, 216-217](https://cdn.rekordbox.com/files/20250925103803/rekordbox7.2.3_manual_EN.pdf)). The
  Pioneer XDJ-AERO automix starts 30 s before the end, "crossfaded (when using the SYNC function)", otherwise
  "faded in and out" ([manual](https://www.manualsdir.com/manuals/174377/pioneer-xdj-aero.html?page=25)).
- **Traktor** Cruise Mode: a plain fade of "Auto Crossfade Time"
  ([NI support](https://support.native-instruments.com/support/solutions/articles/69000879534-traktor-using-the-traktor-cruise-mode-for-automatic-playback-of-your-tracks)).
- **Serato** Autoplay: no crossfade. **Engine DJ, YouTube Music, Amazon Music, Tidal, Deezer, SoundCloud**: no
  automix transitions found (Amazon's DJ Mode is spoken commentary). **Pacemaker** named no transition styles; its
  effects were Echo, Roll, Reverb and a "Hi-Lo" filter ([Digital DJ Tips](https://www.digitaldjtips.com/reviews/pacemaker/)).

**What products do with an unmatchable pair:**
- Apple skips the dynamic transition;
- Spotify warns, then ramps tempo badly if forced;
- VirtualDJ, Pioneer hardware and Traktor do a plain fade;
- djay and VirtualDJ Smart retime the incoming song anyway.

No product documents a dedicated "no beat or live drummer" style. Among the open-source projects, BitChord's filter
ride and loop mix, MORA's and musicmix-dj's echo-outs, and DARAVE's echo, tape stop and quick cut are the only
deliberate answers found.

## A12. What working DJs do with pairs that cannot be beat-matched

Summarised from DJ tutorials and forums (per-technique numbers are in Part B):
- **Indie and rock:** crowds "don't expect smooth mixes"; "lots of indie fades or ends abruptly"; still start the
  next track on the downbeat, "your BPM happily going all over the place"; pace by energy, not BPM
  ([Digital DJ Tips, Phil Morse](https://www.digitaldjtips.com/how-to-dj-indie/)).
- **A DJ TechTools rock thread** ([forum](https://forum.djtechtools.com/t/how-to-mix-transition-rock-songs/3819))
  suggests:
  - mix on 4- or 2-bar loops of a riff;
  - "delay the first track (always sounds better with the bass off) and freez it", then start the next;
  - let the "resonance of the last note of the song in the room" carry a short silence;
  - "listen to commercial radio DJs".

  Older rock drifts, while newer rock is "pretty much locked to the grid".
- **End to end:** pick a next song that "starts with a bang" and start it on a downbeat, "leaving a small amount of
  silence if you have to" ([Digital DJ Tips](https://www.digitaldjtips.com/rock-the-dancefloor/five-basic-dj-transitions/)).
  Mobile DJs: "Use slam-cues, or turntable shut-down effects… Use those cold endings"
  ([Pro Mobile DJ](https://promobiledj.com/2013/02/mixing-how-much-is-too-much)).
- **Radio:** "Never cut off a cold ending of a song. Ever." On a fade, once the important part is over, "get out"
  at the end of a bar ([MusicMaster](https://www.musicmaster.com/newsletters/1218.php)). Overlap is the smaller of
  the outgoing segue-out and incoming segue-in; "five or six seconds is generally about the max … before it starts
  to sound distracting" ([TuneTracker](https://www.tunetrackersystems.com/tips/UnderstandingSegueMarkers.html)).
  Radio doesn't beat-match, it knows the intro and whether the song ends cold or fades
  ([Cridland](https://james.cridland.net/blog/2022/music-streaming-services--and-cross-fading-songs--and-decent-segues/)).

---

# Part B. Catalog of transition techniques

Each entry: **who** uses it; **recipe** (timeline in beats or bars, fader, EQ and filter moves, effect
parameters); **needs**; **when** a DJ picks it; **nori** (can the mixer in nori-dj do it today, or what it would
need). "A" is the outgoing song, "B" the incoming one. Bar counts assume 4/4. Ratings of how good a move sounds
come from the sources named, not from listening here.

nori today (`crates/player/src/types.rs:229-322`, `automix/plan.rs`):
- per-deck fade windows with equal-power, linear or sin² curves, and a loudness trim;
- bass swap (Linkwitz-Riley high-pass at `cut_hz` handed between decks);
- `highs_in` (incoming high shelf);
- `vocal_duck` (incoming 1 kHz band cut, Q 0.35);
- low-pass and high-pass sweeps on **A only**;
- echo on **A only**;
- `out_loop_ms` (A's tail read round) and `in_loop` (B's opening repeated);
- `bend` (both decks glide tempo together, at most 2 %/bar and 20 %, `plan.rs:1154-1160`);
- kinds Gapless, EqualPowerFade, MixRampFade, BeatMatched, EchoOut, Cut (A stops on a downbeat, B at full after
  a riser, A's last beats echo).

The echo-out (`plan.rs:1224-1300`) exits on a downbeat in the last 8 bars after A's last line. A falls over a
beat, echo feedback 0.55, wet −4 dB, delay one beat folded into 250–1000 ms, high-pass 60 Hz → 1.5 kHz, a 2-bar
tail. B starts on its first downbeat and rises over a bar. Each song keeps its tempo.

## B1. Equal-power crossfade (the radio baseline)
- **Who:** Mixxx's constant-power crossfader (`enginexfader.cpp:59-70`); heydavidf/automix's fallback, a **12 s**
  equal-power fade (`transition-scheduler.ts:157-178`); BitChord `EQUAL_POWER`; Traktor and VirtualDJ fades; Liquidsoap
  `cross.simple` (3 s sine fades, `fades.liq` master `:800-867`).
- **Recipe:** `g_out = cos(πx/2)`, `g_in = sin(πx/2)`, 3 dB down each at the midpoint, constant power for
  uncorrelated material. Linear (6 dB down at the midpoint) only for correlated material, such as two beat-locked
  copies ([Fink, Holters, Zölzer 2016](https://www.hsu-hh.de/ant/wp-content/uploads/sites/699/2017/10/Fink-Holters-Zölzer-2016-Signal-matched-power-complementary-cross-fading-and-dry-wet-mixing.pdf)
  **[snippet]**). Log-shaped fade-outs sound most natural, S-curves hold the level up
  ([Sage Audio](https://www.sageaudio.com/articles/how-to-create-fades-during-mastering) **[snippet]**).
- **Needs:** nothing. **When:** nothing is known. Radio caps a blind overlap at about 5–6 s (TuneTracker, above);
  BitChord found its 10–12 s sweeps all sounded "the same ordinary crossfade" (`TransitionPlanner.kt:1298-1301`).
- **nori:** yes (`EqualPowerFade`, ≤ 12 s, `MAX_BLIND_FADE_MS`). Consider ≤ 6 s for blind fades, per radio practice.

## B2. Loudness-driven segue (MixRamp, autocue, one-sided fades)
- **Who:** Liquidsoap `cross.smart` and autocue (A10); MPD MixRamp and Plexamp Sweet Fades (`automix.md` §2);
  RadioBOSS (mix point 2.5 s, level trigger −14 dB, linear fade-out; [manual](https://manual.djsoft.net/radioboss/en/crossfades.htm)
  **[snippet]**); mAirList (next item at the fade-out marker, auto-cue −20 dB, **[snippet]**); radio "extro" at
  about two-thirds of the way into a fade ([Insanity Radio](https://training.insanityradio.com/?p=101)).
- **Recipe:**
  - B starts where A last falls below **−7 dB** of its integrated loudness (autocue; Moonbase59's autocue uses
    −8 LU and recomputes overlaps over 15 s, [GitHub](https://github.com/Moonbase59/autocue));
  - overlap ≤ 6 s; B's cue in at −34 dB with a 0.2 s fade-in; A's cue out at −42 dB.
  - Only the quieter side fades (cross.smart rules 2–3). B louder by ≥ 4 dB: A fades out over 3 s, B starts at
    full. A louder: B fades in, A plays on. Both loud: no overlap, sequence.
  - Never fade a cold ending; stop it on its last beat.
- **Needs:** a loudness envelope at both ends. **When:** any pair without a grid; the right default for Smooth on
  rock.
- **nori:** mostly yes (`MixRampFade`). The one-sided rules can be expressed with the existing per-deck fade
  windows (`out_fade_*`, `in_fade_*`); what is missing is the rule table and a "cold ending: play to the last beat,
  no fade" branch.

## B3. Intro under outro (Mixxx's *Full Intro + Outro*)
- **Who:** Mixxx default mode (A1); heydavidf/automix and BitChord's blend place B so its full-energy phrase lands
  as the blend ends (`transition-scheduler.ts:121-125`); Vande Veire's *relaxed* (A's breakdown under B's 16-bar
  intro); radio "mix in" (Zetta, **[snippet]**).
- **Recipe:**
  - length = min(A's outro, B's intro); the end of B's intro meets the end of A's outro;
  - gains equal-power, or Vande Veire's *relaxed* curves (A full until the switch, then out; B to 0.7 by the
    middle of its fade-in);
  - EQ as in B4 or B5.
- **Needs:** intro and outro bounds. Beat-matching is needed only if both have drums: a beatless intro or outro
  needs no grid ("It doesn't matter if the track is of a different BPM",
  [Digital DJ Tips](https://www.digitaldjtips.com/rock-the-dancefloor/five-basic-dj-transitions/);
  [DJ TechTools](https://djtechtools.com/2017/09/11/mixing-songs-dont-dj-intro-4-common-techniques/) blends 99
  into 60/120 BPM over drumless sections).
- **When:** whenever cues allow; it skips nothing.
- **nori:** yes; the `drop_aligned` search generalises it. **Gap:** a *beatless* overlap of an unmatched pair is
  not used as a long blend. When either side is drumless over the window (low onset and bass energy), a long
  overlap is safe at any tempo.

## B4. Three-band EQ crossfade ("Blend")
- **Who:**
  - Spotify **Blend** ("a smooth 3-band EQ fade");
  - pyCrossfade (`utils.py:248-278`);
  - Vande Veire (shelves only, A2);
  - heydavidf/automix (low swap plus highs −8 dB across, `:134-143`);
  - a DJ-school recipe ([cursa](https://cursa.app/en/page/dj-mixing-foundations-eq-use-for-smooth-blends)): a 16-bar
    blend with the bass swap at bar 9, the low EQ nudged every 4 bars, mids pulled when two vocals clash. Another
    layout: bars 1–4 B's lows out, 5–8 B's mids in, 9–12 swap the lows, 13–16 A out.
- **Recipe (pyCrossfade):**
  - low shelf 70 Hz and high shelf 13 kHz on each deck at `−26·(1 − level)` dB;
  - a **−6 dB·sin(πt) mid dip at 1 kHz on both decks** over the overlap;
  - equal-power volume, 8 bars.
- **Recipe (heydavidf):** low shelf 250 Hz, peak 1 kHz Q 0.6, high shelf 4 kHz; highs −8 dB crossed over the window.
- **Needs:** for 8–16 bars, a locked grid; with no lock, keep it to the fade length of B2.
- **When:** Smooth's natural blend; the iOS 27 sound after "underwater" was dropped (`automix-next.md` §1).
- **nori:** partly. Bass swap and highs-in exist; the mid dip does not. The vocal duck's band-cut
  (`VOX_Q`, peaking subtraction) could run at −6 dB on *both* decks as the mid dip with no new filter type.

## B5. Bass-swap blend, with its placements (Fade, Wave, Melt, Rise, rolling, double drop)
- **Who:**
  - Vande Veire (rolling and double drop: the bass swaps in **one bar** at the switch, the other bands earlier, A2);
  - Spotify **Fade** (swap around the midpoint), **Wave** (centre swap with a low-pass), **Melt** (centre swap with a
    high-pass), **Rise** (swap at the end, LPF on B, HPF on A);
  - heydavidf (one bar at the midpoint, −12 dB, −6 dB under 8 bars);
  - BitChord `DJ_BLEND` (half a beat ending on the swap beat, 24 dB/oct at 200 Hz; B's fader fully up by the swap,
    "both faders up");
  - DARAVE *Classic Blend* (4–12 bars, one swap).
- **Recipe:**
  - B in with its lows cut (shelf −12 to −52 dB, or a 180–250 Hz high-pass) and highs held down;
  - B's highs open over the first half;
  - swap the lows on a downbeat (half a beat to one bar);
  - A's highs and volume leave over the rest.
  - Vande Veire's timings: B's volume at 0.8 power by half the fade-in; A's volume full until the switch, 0.8 after
    it, out over the second half of the fade-out.
- **Placement is the variety** (Vande Veire): drop on drop (double drop, 32 bars out), B's drop into A's high
  section 16 bars before A's breakdown (rolling), B's intro under A's breakdown (relaxed). A Markov rule avoids two
  double drops in a row.
- **Needs:** both grids, tempo within the stretch limit (heydavidf ±8 %, BitChord 5 % for a blend, musicmix ±8 %
  citing discomfort beyond ±10 %).
- **nori:** yes (`BeatMatched`, `drop_aligned`, double drop, `highs_in`, swap of 0.25 bar in DJ).

## B6. Complementary filter ride for unmatched tempos (BitChord `DJ_FILTER` + `SWEEP`)
- **Who:**
  - BitChord (A3);
  - Spotify **Rise**'s filter pair (LPF in, HPF out);
  - Orchard's filtered blend;
  - We Are Crossfader's 128 → 150 BPM genre change: B behind a low-pass with its bass fully cut, opened over a
    16-bar blend ([link](https://wearecrossfader.co.uk/blog/creative-trick-for-genre-transitions-128bpm-150bpm/)).
- **Recipe (BitChord):** 12 beats, ≤ 8 s.
  - A: low-pass 7 kHz → 300 Hz floor, shaped `p^0.75` (front-loaded).
  - B: high-pass from 1.2 kHz (2.4 kHz when both sing), opening by a set point.
  - Lows change hands **once, on a beat, at 0.5**, over half a beat, at 200 Hz, 24 dB/oct: "only one kick is ever
    playing".
  - Faders `cos(π/2·p^1.6)` out and `sin(π/2·p^0.8)` in.
  - Resonance kept low ("high resonance can sound harsh or spike in level",
    [cursa](https://cursa.app/en/page/dj-mixing-foundations-filter-transitions-and-tension-control)).
- **Needs:** A's beat (to place the one swap) or nothing at all. **No tempo match.** Two different grooves overlap
  for 12 beats, but in complementary bands with one kick.
- **When:** unmatched tempos where neither song ends cold, and keys or voices don't clash hard (with a vocal clash,
  BitChord pushes B's high-pass to 2.4 kHz and holds it longer).
- **nori:** **no**: the mixer filters only the outgoing deck. Needs a high-pass (and low-pass) sweep on the
  **incoming** deck. The bass swap's 4th-order Linkwitz-Riley high-pass already exists per deck, so an incoming
  high-pass sweep is the same filter with a moving corner. The planner would choose it for unmatched pairs.

## B7. Bass hand-off for two kicks (BitChord `BASS_HANDOFF`)
- **Who:** BitChord, chosen when **both sides carry a kick** at the join (low band ≥ 0.5 of each song's loud end
  over 8 beats).
- **Recipe:**
  - 8 beats, ≤ 7 s;
  - B in behind a 900 Hz high-pass, open by 0.38;
  - lows swap on a beat at **0.43**;
  - only then A darkens to a 1.6 kHz low-pass.

  "Keep the mids recognisable, remove the two kicks from each other's way, then darken the outgoing deck."
- **Needs/when:** as B6, for two driving rhythm sections (rock into rock). A **different gesture** from B6, so two
  unmatched transitions in a row need not sound alike.
- **nori:** needs the incoming high-pass of B6 and a low-pass on A that starts after the swap (already possible).

## B8. Dual-loop tempo ramp (BitChord's loop mix)
- **Who:** BitChord (A3); Apple iOS 27's repeated intro and outro bars are the same family (`automix.md` §1a); DJ
  practice "mix on 4- or 2-bar loops of a riff" (DJTT rock thread) and "loop out" with a tempo push
  ([DJ TechTools](https://djtechtools.com/2015/04/05/bpm-transition-trick-using-loop-out-move/)).
- **Recipe:**
  - A loops its latest clean 4/2/1 bars in its last 60 s; B loops its earliest clean bars;
  - B enters over **4 bars** at A's tempo, gain 0 → 0.55, lows −18 dB (250 Hz shelf);
  - the master tempo ramps **≤ 2.5 BPM per bar** (2–24 bars) to B's tempo, octave-folded;
  - the lows swap over one bar mid-ramp;
  - A then drains over **2 bars** through a high-pass 20 Hz → 1.5 kHz while its gain goes 0.6 → 0;
  - B releases at its loop boundary into its song at native tempo.

  DJ riding rates for comparison:
  - 128 → 150 in "eight equal nudges, one per bar" (≈ 2 %/bar);
  - 104 → 126 and 125 → 183 over 16 bars (≈ 1.3 and 2.9 %/bar,
    [We Are Crossfader](https://wearecrossfader.co.uk/blog/12-high-energy-dj-transitions/));
  - a slow drift of "1 BPM every 30 seconds"
    ([Digital DJ Tips](https://www.digitaldjtips.com/how-to-change-tempo-dj-sets/)).
- **Needs:** a grid on A's last minute and B's opening; **instrumental, steady bars on both** (no word looped, every
  bar ≥ 0.6 of the typical loudness); keys not clashing (two loops play together for the whole ramp); stretch
  quality across the ramp.
- **When:** the main *matched-groove* answer for tempo gaps of 10–40 %; it makes a rock pair sound DJ-mixed instead
  of faded. The join gets longer by the ramp (≈ 22 bars, ~40 s, for +33 % at 120 BPM).
- **nori:** **mostly there.** `out_loop_ms` (A's tail looped), `in_loop` (B's opening repeated) and `bend` (both
  decks glide together) exist. Missing:
  1. a bend that runs over **looped** material, so the gap can exceed 20 % without running out of song;
  2. a cap in **BPM per bar** (2–2.5) with up to 24 bars instead of 2 %/bar and 20 %;
  3. B's loop taken from its opening only (nori's constraint), which BitChord's "earliest clean loop in 45 s"
     mostly satisfies on rock intros.

  Gate it hard on instrumental bars (BitChord's ≤ 3 % sung frames, no run over 0.2 s) and on a steady grid, since
  drifting live drummers make a loop seam audible.

## B9. Tempo ride without loops (bend, "Sync + Tempo blend")
- **Who:**
  - pyCrossfade stretches **A** bar by bar to B's tempo over the 8 bars before the fade (A5);
  - djay *Sync + Tempo blend* ("gradually adjusts the outgoing track's tempo");
  - Apple iOS 27 is reported to glide late and subtly (`automix.md` §1a);
  - heydavidf eases B back in **8 steps of 3 s**;
  - BitChord's ease limit is **0.75 % per beat**.
- **Recipe:** A's tempo glides toward B's over 4–8 bars before and during the overlap, B eases back after. Limit
  about 1 %/beat or 2 %/bar; DJs "stay within 5 %" for a ride
  ([Digital DJ Tips](https://www.digitaldjtips.com/7-insider-tips-making-exciting-tempo-changes-djing/)).
- **Needs:** both grids, gap within what the songs' own bars can absorb.
- **nori:** yes (`bend`, 2 %/bar, ≤ 20 %, 2–8 bars). Fine as is for gaps up to ~15 %; beyond that use B8.

## B10. Echo out into a clean entry
- **Who:**
  - nori (`plan.rs:1238`);
  - musicmix-dj (9/10 and 7/10 in its listening);
  - MORA (gaps ≥ 25 %);
  - DARAVE (with spinback);
  - djay **Echo** and **Neural Mix Echo Out**;
  - Spotify Mix echo ½, ¾, 1 beat;
  - Pacemaker Echo;
  - Roger Sanchez: "a nice dramatic exit" with echo ([Pioneer DJ blog](https://blog.pioneerdj.com/?p=5227)).
- **Recipe, consolidated:**
  - Engage on a downbeat after A's last sung line, never mid-phrase
    ([mixgraph](https://www.mixgraph.io/glossary/echo-out)).
  - **Bass off before the echo** (DJTT rock thread: "always sounds better with the bass off"; MORA removes the bass
    first).
  - Delay ½ or 1 beat (Digital DJ Tips, musicmix), ¾ in DARAVE's preset; feedback 0.4–0.6 (BitChord 0.4,
    musicmix 0.5–0.6, nori 0.55, Mixxx default 0.71); wet "about 30–40 %"
    ([Digital DJ Tips](https://www.digitaldjtips.com/do-you-know-how-to-use-these-3-essential-dj-effects/)) to
    −3 dB.
  - Echoes high-passed upward (musicmix 150 Hz → 2.5 kHz; BitChord band-limits 250 Hz–3.5 kHz in the loop).
  - A's dry signal pulled at once ("pull the channel fader down immediately", mixgraph); BitChord found a
    quarter-beat kill "heard as the song cutting out" and fades the dry over 4 beats.
  - Tail 2–4 bars. B starts **on its downbeat at its own tempo, before the tail dies**, the tail ducked about
    −12 dB under B (musicmix); MORA leaves at least a bar of instrumental between A's last line and B's first.
- **Needs:** A's beat (for the delay and the exit downbeat); B's first downbeat. Any tempo gap, and keys hardly
  matter (the echo "holds the harmony", DARAVE TT-07).
- **When:** the default for unmatched pairs in every open-source project that handles them. Opinion: it works but
  "is a dead end"; "overusing it can drain the dancefloor"
  ([Digital DJ Tips](https://www.digitaldjtips.com/stop-using-echo-out/)).
- **nori:** **yes.** Small refinements from the sources: cut A's lows over the beat before the echo; let the echo
  high-pass start near 150 Hz rather than 60 Hz; duck the tail about 12 dB as B lands; vary the delay (½, ¾, 1 beat)
  per pair rather than always one beat.

## B11. Filter out into a clean entry (high-pass wash)
- **Who:**
  - musicmix-dj *filter_out* (**9/10**, its joint top score);
  - DARAVE *Filter Up* (HPF over the last 75 % of the transition, ease-in);
  - Spotify "High-pass filter half out";
  - djay **Filter**;
  - DJ practice: move the filter "over the phrase", 8 bars (4–16), remove the filtered track at the phrase boundary
    ([cursa](https://cursa.app/en/page/dj-mixing-foundations-reliable-transition-types-eq-blend-filter-blend-drop-swap)).
- **Recipe:**
  - high-pass on A, **30 Hz → 1.2 kHz**, geometric, 4th order, over **1–4 bars**, ending on A's exit downbeat;
  - B starts clean on its downbeat (10 ms micro-fades);
  - optional "filter swap": B in behind a low-pass 250 Hz → 18 kHz over 2 bars (musicmix);
  - a slow sweep is "mud".
- **Needs:** A's beat and B's first downbeat; any tempo.
- **When:** like B10 but drier and shorter; good where A's last bars are sung (no stutter from an echo) and when
  B "starts with a bang".
- **nori:** **yes with what exists** (`high_pass` sweep on A, fade windows). It needs a planner kind or an EchoOut
  variant with no echo. The optional B-side low-pass needs an incoming filter (B6).

## B12. Echo freeze (spectral freeze)
- **Who:** musicmix-dj `echo_freeze` (its far-tempo first choice); the DJTT rock thread ("freez it"); a
  "loop-out … echo freeze out of the old track" trick
  ([DJ TechTools](https://djtechtools.com/2015/04/05/bpm-transition-trick-using-loop-out-move/)).
- **Recipe (musicmix):**
  - one bar before the exit A's dry is cut;
  - in its place a spectral freeze of A's last beat (mean 4096-point STFT magnitude, random phase) at 0.7, plus a
    1-beat echo of that beat at feedback 0.6, at −3 dB against A, high-passed 150 Hz → 1.5 kHz;
  - B clean on the downbeat;
  - the freeze drops 12 dB and fades over 1–2 bars of B.
- **Needs:** A's beat. Any tempo.
- **nori:** **no** (needs an STFT freeze voice). It is a pad-like bridge: more "ambient" than an echo, and it does
  not stutter a vocal.

## B13. Reverb wash
- **Who:**
  - musicmix-dj `reverb_wash` (for an energy drop, or the default with echo);
  - Spotify reverb out at the centre, cut at the end, out at the end;
  - djay **Dissolve** ("evaporates the outgoing track") reads like this **[inferred]**;
  - DARAVE *Reverb Out*;
  - Pacemaker Reverb.
- **Recipe:**
  - musicmix: send rising `u^1.5` over **1–2 bars**, RT60 **3.5 s**, 20 ms pre-delay, wet −3 dB, wet high-passed at
    **250 Hz**; A's dry cut on the exit downbeat; tail −12 dB under B over 1–4 bars.
  - DJ settings: wet 50 %, decay ~10 s, post-fader, cut the lows under heavy reverb
    ([DJ Courses Online](https://djcoursesonline.com/dj-blog/2018/7/9/getting-dj-effects-to-work)).
  - Let the tail run out, then drop in a track at a different tempo
    ([Digital DJ Tips](https://www.digitaldjtips.com/do-you-know-how-to-use-these-3-essential-dj-effects/)).
- **Needs:** nothing rhythmic; any tempo; best into a calmer B. "Killer in the more chilled-out genres"
  ([DJ TechTools](https://djtechtools.com/2017/09/11/mixing-songs-dont-dj-intro-4-common-techniques/));
  "overuse it and it just sounds cheesy" ([Digital DJ Tips](https://www.digitaldjtips.com/beginners-guide-to-dj-effects/)).
- **When:** energy going down (ballad next, B quieter by 6 dB or more), or a big sustained final chord.
- **nori:** **no reverb.** A small algorithmic reverb (Freeverb or Dattorro, about 100 lines, allocation-free once
  sized) on A's send only while a transition runs.

## B14. Tape stop / brake
- **Who:**
  - musicmix-dj `tape_stop` (far tempo with a calmer B);
  - DARAVE *Tape Stop* ("the only honest way to change tempo") and *Motor Off* (8–32 beats, once a set);
  - BeatBridge *Tape Brake* (high-energy A into a soft B);
  - Serato and rekordbox brake FX;
  - mobile DJs' "turntable shut-down".
- **Recipe:**
  - speed `(1 − u)^1.5` (musicmix) or curve 1.6 (DARAVE) over **2 beats** (1–4), pitch falling with it, the fader
    pulled over the last beat;
  - B clean on the next downbeat;
  - Serato and rekordbox at about 10 o'clock "brake to stop within a beat usually"
    ([We Are Crossfader](https://wearecrossfader.co.uk/blog/vinyl-brake-tricks-did-you-know/));
  - BeatBridge: rate 1 → 0.05 in 650 ms, the tail caught in a 420 ms delay at feedback 0.85.
- **Needs:** A's beat (to stop on a phrase end). Any tempo.
- **When:** a hard energy break, a big tempo change, or A ending loud with no tail. "Horrible if a DJ did it more
  than once in a set" (forum **[snippet]**); musicmix and DARAVE both class it as a marked effect, used sparingly.
- **nori:** **no.** It needs varispeed read on A with a speed going to zero (the engine has varispeed; the plan
  needs a speed envelope on A, not only a tempo ratio).

## B15. Backspin / spinback / rewind
- **Who:** musicmix-dj `backspin` (A sings or pushes at the exit); DARAVE *Echo Tail + Spinback* (its first
  choice for unmatched tempos: echo 4 beats, spin 1 beat at 9×) and *Rewind* (1 bar at 6×); Serato Spin Back and
  Spinback Echo, rekordbox BACK SPIN (**[snippet]**).
- **Recipe (musicmix):** platter stopped in 40 ms, thrown backwards to **3×** speed in 80 ms, slowing exponentially
  to zero over **2 beats**, amplitude ∝ √|speed|; B on the downbeat.
- **When:** hip-hop, reggae and drum and bass culture; "an escape hatch if you can't get two records to mix
  properly" (Brewster & Broughton via musicmix `dj_policy.py:104`). Rare.
- **nori:** **no** (reverse varispeed read). Low priority for a rock or alt playlist.

## B16. Loop roll / halving build
- **Who:**
  - Spotify "loop halving";
  - djay Crossfader FX roll;
  - Pacemaker Roll;
  - DARAVE *Loop Roll Build* (4 → 2 → 1 → ½ → ¼ beats);
  - musicmix *halving_build* (4 or 8 bars, rising high-pass, optional 1-beat pause);
  - Apple iOS 27's repeated beat (`automix.md` §1a).
- **Recipe:** in the bar before B's landing, loop 1 beat, then ½, then ¼, released on the downbeat (slip keeps
  position) ([mixgraph](https://www.mixgraph.io/glossary/loop-roll)); B on the downbeat. "Anything longer than a
  bar … sounds like you got stuck"; "two per set reads as taste and ten reads as a nervous tic" (same).
- **Needs:** A's grid; instrumental beats (a rolled word is the iOS 27 "broken record" complaint).
- **nori:** **partly** (`out_loop_ms` holds one fixed slice). A roll needs a loop length that halves on schedule.

## B17. Cut on the one (drop swap, Slam, quick cut)
- **Who:**
  - Spotify **Slam** ("a hard, centred volume swap") and "Center cut";
  - DARAVE *Quick Cut* (½ beat at a phrase boundary);
  - nori `Cut`;
  - indie and rock DJs: "the second the one hits, your old tune has gone"
    ([Digital DJ Tips](https://www.digitaldjtips.com/rock-the-dancefloor/five-basic-dj-transitions/));
  - dropmix: slam on the one, or start a beat early to absorb timing error
    ([Digital DJ Tips](https://www.digitaldjtips.com/dropmix-like-a-pro-hip-hop-dj/)).
- **Recipe:**
  - B at full on its first downbeat, A gone within 10 ms to ½ beat;
  - 0–4 bars of overlap at most
    ([cursa](https://cursa.app/en/page/dj-mixing-foundations-reliable-transition-types-eq-blend-filter-blend-drop-swap));
  - pick a point where A is winding down;
  - half- and double-time changes work "better with a faster cut, rather than a longer crossfade"
    ([DJ.Studio](https://dj.studio/blog/dj-tempo-change-techniques));
  - across a tempo gap, cut between songs that share an element (distorted guitar, a lyric), "one or two times a
    set" ([Digital DJ Tips](https://www.digitaldjtips.com/how-to-change-tempo-dj-sets/)).
- **Scores:** musicmix's dry drop straight onto B's chorus scored **3/10**; a prepared one (riser, build) is what
  its policy prefers. DARAVE: "a cut with nothing is heard as a mistake".
- **nori:** **yes** (`Cut` with riser and A's last beats echoing; within nori's no-hard-cut rule). Use it when A
  ends cold or B "starts with a bang".

## B18. Outgoing loop under the entry (loop extend, loop tail)
- **Who:**
  - Apple iOS 27;
  - DARAVE *Loop Tail* (A loops its last phrase, 4 bars held 8, while B comes in, "how you fix 'the new one's intro
    is too long'");
  - musicmix `loop_extend` (1–2 instrumental bars of A, 8 bars if stretch ≤ 5 %, else 4);
  - DJs "loop 4–16 beats of drums on the outgoing track to make an artificial outro"
    ([DJ TechTools](https://djtechtools.com/2017/09/11/mixing-songs-dont-dj-intro-4-common-techniques/)).
- **nori:** **yes** (`out_loop_ms`, instrumental only). B8 is its extension to unmatched tempos.

## B19. Half-time / double-time
- **Who:** BeatBridge *Half-Time* (A LPF 20 kHz → 2.4 kHz over a bar, dotted-eighth delay on the half-time grid,
  B LPF 300 Hz → 20 kHz over 3 bars, bass swap at bar 2); BitChord and heydavidf fold ×2 and ×½; DJ practice
  160↔80, 140↔70; "156 → 78 works for lounges and radio but can clear the floor"
  ([Digital DJ Tips](https://www.digitaldjtips.com/how-to-change-tempo-dj-sets/)).
- **nori:** yes (octave folding in the tempo check). Not a separate effect.

## B20. Noise sweep and synthetic riser
- **Who:** nori's riser (rising high-pass before the drop, `RISER_BARS = 2`, 40 → 900 Hz); Spotify "Noise out end";
  djay **Riser**; BeatBridge *Riser Drop* (band-pass Q 7 swept 400 Hz → 4 kHz over 2 bars); musicmix
  *build_into_chorus* (HPF + snare roll + a riser in B's key, 2–4 bars); Pioneer NOISE (filtered white noise).
- **Use:** swept up over the last bars, dropped on the downbeat, kept under the music. It masks "a tempo that is
  drifting, or a phrase that did not line up", and "stops working if you use it every eight bars"
  ([mixgraph](https://www.mixgraph.io/glossary/noise-fx)). Mobile DJs: a sweeper is "a cheap 'n easy way to change
  tempo" ([Pro Mobile DJ](https://promobiledj.com/2013/02/mixing-how-much-is-too-much)).
- **nori:** the filter riser exists; a noise voice does not (a seeded white-noise source through the existing
  sweep filter would do it).

## B21. Bridges: a cappella, drum loop, pad
- **Who:**
  - DJ practice: kill the music, drop an a cappella at the new BPM, then a beat at that BPM
    ([Digital DJ Tips](https://www.digitaldjtips.com/7-insider-tips-making-exciting-tempo-changes-djing/));
  - a keylocked percussion loop moved to the new tempo, then mix in
    ([Digital DJ Tips](https://www.digitaldjtips.com/how-to-change-tempo-dj-sets/));
  - musicmix *note_bridge*: one bar of a synthesised pad on a chord common to both keys.
- **nori:** no; the a cappella route needs stems (`automix-next.md` §7.4). A drum loop is B8 with A's drums. The pad
  bridge is a small synth voice; low priority.

## B22. Stem moves (Neural Mix, vocal swap, drum cut)
- **Who:** djay *Neural Mix Fade*, *Neural Mix Echo Out*, *Drum Cut*, *Vocal Cut*; musicmix *acapella_over* and
  *vocal_swap* (Demucs stems, 4–8 bars, tempo within ±8 %, keys compatible or ±1 semitone); DARAVE's stem
  techniques.
- **nori:** no (the "Studio" stem plan, `automix-next.md` §7.4). An echo-out of the *vocal stem only* would be the
  best-sounding unmatched move of all, since the drums stop dry while the voice rings, but it needs stems.

## B23. Colour effects (tremolo, flanger, phaser, bitcrusher, overdrive)
- **Who:** djay Tremolo; DARAVE limits flanger, phaser, overdrive and bitcrusher outs to genres
  (trance/house/DnB, techno/dubstep/phonk) (`fx_presets.py:76-107`).
- **nori:** no, and not recommended for a library player; DARAVE itself keeps them genre-gated and manual.

## B24. End to end on the downbeat, with a breath of silence
- **Who:** indie and rock DJs, mobile DJs ("I play usually the whole song",
  [Pro Mobile DJ](https://promobiledj.com/2013/02/mixing-how-much-is-too-much)); radio for cold endings;
  Liquidsoap's rule 5 (both loud: sequence); VirtualDJ *None*; Mixxx's negative transition time (a gap).
- **Recipe:**
  - A ends on its cold last beat or fades;
  - B starts on a downbeat-like onset, "leaving a small amount of silence if you have to";
  - the room's ring of the last note carries it (DJTT rock thread).
- **nori:** yes (gapless, silence trim). It is a legitimate *DJ* choice for a cold ending into a song that "starts
  with a bang", not only a fallback.

---

# Part C. Ranked recommendation

## C1. DJ style

Order of choice per pair. Structure first, as Vande Veire and DARAVE both argue: the variety a listener hears
mostly comes from *which sections meet*, not from a random effect.

1. **Matched tempo (≤ ~8 %, two grids):** keep today's bass-swap blends with drop landings, double drops and
   rolling placements (B5); add the B4 mid dip.
2. **Unmatched but loopable (gap ≤ ~40 % octave-folded; steady grids at A's end and B's opening; instrumental
   bars on both; keys not clashing):** the **dual-loop tempo ramp** (B8), ≤ 2–2.5 BPM per bar, ≤ 24 bars. This is
   the one move that makes a 21–41 % rock pair sound *mixed* with both grooves locked. It extends nori's
   `out_loop` + `in_loop` + `bend`. Highest audible gain for the owner's log.
3. **Unmatched, overlapping without a lock:** the **complementary filter ride** (B6) or, when both sides have a
   kick at the join, the **bass hand-off** (B7), 8–12 beats. It needs one new mixer piece, the incoming-deck
   high-pass sweep, and gives two distinct gestures.
4. **Unmatched, no overlap of grooves** (no clean loop, or A's last bars sung):
   - **echo out** (B10, exists; add the refinements);
   - **filter out** (B11, buildable from existing parts; musicmix's joint top score);
   - choose between them by whether A's last beat is sung: a sung beat goes to filter out, since an echo stutters a
     word.
5. **Cold ending, or B starts with a bang:** **cut on the one** (B17, exists) or **end to end** (B24); this is
   what rock DJs actually do.
6. **Energy drop (B 6 dB or more quieter, or a ballad next):** **reverb wash** (B13, needs a reverb) or **tape
   stop** (B14, needs a speed envelope). Both are "marked" effects: budget them, at most one in any 5–8
   transitions (musicmix's repeat penalty 0.15 and FX budget; mixgraph's "two per set").
7. **Rare spice, off unless budget allows:** loop roll into B's downbeat (B16), noise sweep (B20), backspin (B15).
   Never colour effects (B23).

**Variety control:** a per-technique repeat penalty (musicmix 0.15), a Markov rule against the same marked move
twice in a row (Vande Veire), and deterministic choice per pair (BitChord: "a genuinely random answer would make the
marker and the renderer change their minds"). Do **not** shuffle styles the way djay's "Automatic" does.

## C2. Smooth style

1. **Loudness-driven segue** (B2) as the base for anything without a lock: autocue-style start-next point, overlap
   ≤ 5–6 s, one-sided fades (cross.smart rules 2–3), cold endings played to the last beat.
2. **Intro under outro** (B3), including long overlaps of a *beatless* intro or outro at any tempo.
3. **Three-band EQ crossfade** (B4): bass hand-off plus a −6 dB mid dip on both decks plus highs crossed; this is
   the "natural blend" iOS 27 moved to.
4. **Beat-matched blend** (B5) with a gentle swap when grids allow, tempo glide subtle (B9).
5. **Vocal separation only when two voices collide**, scaled by the overlap (BitChord's `rideVocalSeparation`:
   outgoing low-pass toward 1.6 kHz, incoming high-pass 700 Hz, "subtle enough that a listener notices the absence
   of the clash rather than the presence of a filter"). nori's vocal duck already does this.
6. Outgoing instrumental loop (B18) only to reach a phrase; no echo, brake, reverb or roll in Smooth.

## C3. What nori's mixer would need, by value

1. **Incoming-deck high-pass (and low-pass) sweep:** unlocks B6, B7 and the B11 filter swap. Same LR4 filter as the
   bass swap, moving corner.
2. **Bend over loops, capped in BPM per bar** (2–2.5), up to 24 bars and a ~40 % gap: unlocks B8.
3. **Planner kinds** for filter out (B11), the B6 and B7 rides, the cross.smart one-sided fades and the cold-ending
   end to end; plus a variety budget.
4. **Mid dip on both decks** (B4), reusing the vocal duck's band-cut.
5. **Reverb send on A** (B13) and a **speed envelope to zero on A** (B14): the two marked effects for energy drops.
6. Later: loop-length halving (B16), a noise voice (B20), spectral freeze (B12); stems (B21, B22) per the Studio
   plan.
