# BitChord's AutoMix, read in depth, and how nori can do it better

Source: <https://github.com/kushagrasinghx/BitChord>, shallow clone of commit `acf219a7` (2026-11-06), GPL-3.0
(several files are AGPL-3.0 ports from Orchard and heydavidf/automix). This note describes algorithms, constants and
the reasons the code gives for them, in our own words. **No code is to be copied into nori** (MIT). All paths below are
relative to the BitChord repository; the long Android paths are shortened:

| short | full |
|---|---|
| `LoopMix.kt` | `shared/src/commonMain/kotlin/com/music/bitchord/playback/smart/LoopMix.kt` |
| `Planner.kt` | `shared/src/commonMain/kotlin/com/music/bitchord/playback/smart/TransitionPlanner.kt` |
| `Policy.kt` | `shared/src/commonMain/kotlin/com/music/bitchord/playback/smart/TransitionPolicy.kt` |
| `TrackAnalysis.kt` | `shared/src/commonMain/kotlin/com/music/bitchord/playback/smart/TrackAnalysis.kt` |
| `BeatTracker.kt`, `VocalTracker.kt`, `TrackFeatures.kt`, `AnalysisStore.kt` | `shared/src/jvmSharedMain/kotlin/com/music/bitchord/playback/smart/…` |
| `Controller.kt` | `app/src/main/java/com/music/bitchord/playback/CrossfadeController.kt` |
| `FilterProc.kt` | `app/src/main/java/com/music/bitchord/playback/TransitionFilterProcessor.kt` |
| `Session.kt`, `Port.kt` | `app/src/main/java/com/music/bitchord/playback/loopmix/LoopMixSession.kt`, `LoopMixPort.kt` |
| `TrackAnalyzer.kt` | `app/src/main/java/com/music/bitchord/playback/smart/TrackAnalyzer.kt` |
| `renderer.cpp/.h` | `native/loopmix/loop_mix_renderer.cpp/.h` |
| `audio_analysis.cpp`, `tempo_analysis.cpp` | `native/analyzer/…` |
| `web doc` | `web/docs/automix.md` |
| `NOTICE` | `web/app/src/playback/automix/NOTICE.md` |

The first pass (`software.md` §A3, §B6–B8) is accurate; this note goes deeper and corrects or sharpens it where the
code says more. The biggest new findings:

- the **loop mix is tried first for every pair** it can do, matched tempos included, and the beat-matched blend only
  runs when no loop plan exists (`Controller.kt:1131-1147`);
- the **blend is not phase-locked at all**: the incoming player is started on a 40 ms poll, and the phase-lock servo
  exists but every plan sets it off because speed changes cracked on some phones (`Planner.kt:644, 1130, 1862-1864`);
- inside a loop the decks assume **evenly spaced beats** between two detected downbeats; the model's individual beats
  are never used as warp points;
- the loop mix has **no key check, no skip budget and no headroom**: it can drop up to ~60 s of the outgoing song and
  ~45 s of the incoming intro, and it sums the decks with linear gains that reach 1.55.

---

## 1. Analysis

### 1.1 When it runs, what it reads, what it stores

- **On demand, per pair.** Every 250 ms planner tick asks for the playing track and the next one
  (`Controller.kt:1283-1291`, `IDLE_STEP_MS = 250` at `:4261`). Nothing else is analyzed ahead of time.
- **Two passes per track** (`TrackAnalyzer.kt:290-311`):
  - a *head* pass as soon as enough of the opening is cached: grid only, provisional, enough for the track's role as
    the **incoming** side;
  - a *whole-track* pass once the file is complete: everything, needed for the **outgoing** side.

  The reason given: a streamed track is only fully downloaded while it already plays, so a whole-track-only design
  never had the incoming side analyzed in time.
- **Two stages in the whole-track pass** (`TrackAnalyzer.kt:1045-1175`):
  1. *DSP pass* over the whole track, mono, at **11 025 Hz** (`TrackFeatures.kt` `sampleRate`): envelope, content
     end, tempo, key, energy curves, a heuristic vocal curve, phrases, cue candidates (`audio_analysis.cpp:584-689`).
  2. *Model pass* over the **ends only**: Beat This! and open-unmix on **2 windows at the head (~44 s)** and **3 at the
     tail (~65 s)**, each ~22.3 s (the vocal model's fixed input), overlapping by **1 s**
     (`TrackAnalyzer.kt:1090-1111, 1196-1238, 1586-1591`). The spans are chosen to match the loop searches (45 s in,
     60 s out). The middle of a song is never decoded for models.
- **Beat list stitching:** at each window overlap the earlier window's beats are kept up to the overlap's midpoint and
  the later window's after it; confidence is the beat-count-weighted mean (`TrackAnalyzer.kt:1240-1265`).
- **Stored** per track as one JSON file named by cache key, written atomically (tmp + move), with a schema version; a
  mismatched version is deleted and re-earned, never migrated ("a wrong beat grid is worse than none")
  (`AnalysisStore.kt:22-120`). Curves rounded to milliseconds. A failure (bpm 0) is not stored.
- **Refusals that protect quality:**
  - a decode that yields < **95 %** of the container's duration is refused, because a short decode looks exactly like
    trailing silence and would place the exit minutes early (`TrackAnalyzer.kt:967-1028`, `MIN_DECODED_FRACTION`);
  - analysis is always taken from the **canonical recording** (YouTube Opus), never from a substitute source; a track
    whose analysis came from another master gets no loop mix, because a deck cut from different audio would land off
    the song it splices into (`AutomixAnalysisSource.kt`, `Controller.kt:2432, 2438-2442`, `TrackAnalyzer.kt:1480-1497`).
- **Cost controls:** inference threads 1/2/4 by a performance setting; ONNX arena allocator off (it kept tens of MB
  for the session's life, `BeatTracker.kt:90-96`); work pauses when the phone is hot (`TrackAnalyzer.kt:321-324`);
  large buffers live in their own function frames so ART can free them before the models run
  (`TrackAnalyzer.kt:1064-1077, 1289-1335`).

### 1.2 What `TrackAnalysis` holds (`TrackAnalysis.kt:42-117`)

| field | meaning | source |
|---|---|---|
| `bpm`, `beatInterval`, `beatConfidence`, `firstBeat` | tempo from the **tail** model grid, else the head grid, else DSP | `TrackAnalyzer.kt:1119-1149` |
| `downbeats` | head + tail model downbeats merged (DSP's when the model found none) | `:1145-1147` |
| `headBeats`, `tailBeats` (`BeatSide`) | per-end **beat list**, downbeat list, bpm, confidence | `TrackAnalysis.kt:98-105, 191-200` |
| `phraseBoundaries` | DSP: a **fixed 32-beat lattice** from the first DSP downbeat, plus intro end, outro start, content end | `audio_analysis.cpp:440-515` |
| `key`, `keyConfidence` | DSP, whole track | `audio_analysis.cpp:319-427` |
| `audibleStartTime`, `pickupTime`, `introEndTime`, `outroStartTime`, `contentEndTime` | DSP envelope | `audio_analysis.cpp:150-202, 442-496` |
| `mixInCandidates`, `mixOutCandidates` | DSP cue points with type and score | `audio_analysis.cpp:555-577` |
| `energyCurve`, `lowEnergyCurve` | ≤ **240 points** per track (≈1 s apart on a 4-min song) | `audio_analysis.cpp:627-667` |
| `vocalActivityMask` | per energy-curve point; model values where it ran, else the rescaled DSP heuristic; neutral **0.2** elsewhere | `TrackAnalyzer.kt:1162-1167, 1416-1471, 1584` |
| `vocalProbability` | whole-track DSP vocal likelihood | `audio_analysis.cpp:429-437` |
| `vocalWindows` | **raw open-unmix output** per model window (~43 fps), keeping "not measured" distinct from "not sung" | `TrackAnalysis.kt:106-111` |
| `levelWindows` | RMS at **10 fps** over the same windows, stored as −60..0 dBFS → 0..1 | `TrackAnalysis.kt:112-116, 202-223`, `TrackAnalyzer.kt:1349-1374` |

### 1.3 Beats and downbeats (Beat This! `final0`)

- Model: the official **final0** checkpoint, exported to ONNX with **only MatMuls quantized to int8, per channel**;
  convolutions stay float. An earlier export that also quantized convolutions **found no beats at all**
  (`BeatTracker.kt:219-230`, web doc lines 123-130). The web build checked it matches the float model on 99–100 % of
  beats and every downbeat (web doc 125-126).
- Front end: 128-band mel at **50 fps**; inference in chunks of **1500 frames** with **6 frames** dropped at each
  inner edge (`BeatTracker.kt:159-207, 231-239`).
- Peak picking (`BeatTracker.kt:241-305`): a frame is a beat if its logit is positive and the maximum over a
  **7-frame** window; adjacent tied frames collapse to their mean; then a **parabolic fit** through the peak and its
  neighbors gives sub-frame time. The reason: 50 fps quantizes beats to 20 ms, a large share of a flam budget.
- No DBN or HMM post-processing; meter is not inferred (4/4 is assumed everywhere downstream).
- Downbeats are **snapped onto the nearest beat** so the bar grid is a strict subset of the beat grid
  (`BeatTracker.kt:136-144`).
- Tempo (`BeatTracker.kt:307-320`): median inter-beat gap, then the median again over gaps within **±20 %** of it (so a
  few missed beats don't drag it); valid only in **40–220 BPM**; needs ≥ **8** beats.
- Confidence (`BeatTracker.kt:322-338`): a blend of how regular the spacing is (share of gaps within **±10 %** of the
  median) and how decisive the peaks are (a logistic of the median peak logit, centered at 0.5), mapped to
  `0.35 + 0.4·regular + 0.25·strength`, capped at **0.95** ("a model is evidence, not proof").
- **Representation**: a *beat list* per end (`headBeats`/`tailBeats`) and a constant `beatInterval` for the rest. Every
  consumer except the loop picker uses the constant interval.

### 1.4 Tempo drift and live drummers

What it does:

- **Two grids per track**, head and tail, never one: a breakdown that comes back half a beat off breaks any single
  fit (`TrackAnalysis.kt:98-103`; the web NOTICE measured it on *One More Time*, `NOTICE` "Head and tail grids").
- **Loops are cut between detected downbeats**, so each loop carries its own tempo (its length over its beat count).
  A test feeds a beat that stretches 0.5 % over a minute and checks both loop ends land on detected downbeats
  (`LoopMixTest.kt:120-138`). A loop whose far downbeat is more than **¼ beat** from where it should be is skipped,
  which rejects downbeats the model put on the wrong beat (`LoopMix.kt:254-285`, test `:141-150`).
- **Phase readout per bar**: for the (disabled) servo and the scrubber pulse, a position's beat is counted inside the
  bar its two downbeats bound, so a drifting track reads its local beat (`Controller.kt:3641-3681`). A bar is trusted
  only if it holds 1–8 beats of the nominal interval.
- **Live recordings are refused outright** by title/album text: bracketed "(Live …)", "- Live" suffixes, albums named
  "Live at …"; the bare word was matching "Live It Up" and similar (`Planner.kt:104-127`).

What it does not do:

- inside a loop, beats are spaced evenly (`LoopMix.kt:203-206, 511-513`); the model's actual beat times are not used;
- the blend runs one constant rate from median BPMs, so drift during a 12–16 s blend is not followed;
- no stability measure (beyond the regularity term in confidence) gates the loop mix.

### 1.5 Sections, phrases, energy

- **Envelope** (`audio_analysis.cpp:134-202`): **250 ms** RMS windows; noise floor = 5th percentile, reference =
  85th percentile; activity threshold = max(0.0025, min(2.6 × floor, 0.28 × reference), 0.1 × reference). Audible
  start: first **1.5 s** with ≥ 2/3 of windows active and a peak ≥ 1.45 × threshold. Content end: trailing silence
  (≥ 0.35 s under a silence gate) or the last sustained active stretch.
- **Interior mix-out ("energy cliff")** (`audio_analysis.cpp:205-281`): the longest silence ≥ **0.3 s** after 55 % of
  the track and ≥ 4 s before the end, bordered by sound; early gaps (before 80 % of content) count only if the main
  arrangement does not come back; then backtrack ≤ 4 s to where the level fell below 65 % of reference.
- **Phrases** are not detected: the code lays a **32-beat lattice** from the first DSP downbeat
  (`audio_analysis.cpp:440-443, 498-501`), adds intro end and outro start, and labels each span intro / outro /
  body / breakdown (< 58 % of reference) by average energy (`:516-527`).
- **Intro end**: max(first lattice line after the start, the first 4 s averaging ≥ 62 % of reference), snapped to a
  downbeat, capped at **48 s** (`:451-466`).
- **Outro start**: from 60 % of content, the first 4 s window under 68 % of reference whose tail also stays under
  72 % and which is not a dip that recovers (`:468-496`).
- **Mix-in candidates** (`:529-565`): `pickup` (audible start, 0.8), `intro_drop` (the first 8-bar line, capped at
  min(36 s, 28 % of content), 0.9), `main_drop` (the same 8-bar line when the cap moved `intro_drop` earlier, 0.95).
  In practice these are three points on one lattice, not detected drops.
- **Mix-out candidates** (`:567-577`): `energy_cliff` 0.95, `outro_start` 0.9, `content_end` 0.75.
- **Energy for the planner**: ≤ 240-point RMS curve normalized to the reference, capped at 1.5. "Calm" = a window
  averaging < **0.7** of the track's mean (`Planner.kt:1172-1175`).

### 1.6 Vocals

Three sources, in order of trust:

1. **Synced lyrics** (loop picker only; `LoopMix.kt:72-87, 445-468`, `Controller.kt:2406-2423`). Needs ≥ 4 timed
   lines. Each line becomes a sung span padded **0.15 s** before (breath) and **0.35 s** after (voice tail, reverb).
   A line without an end is taken as sung until the next line, at most **10 s**, but only its first **2 s** are
   "certain"; the guessed rest is left to the vocal model. A loop needs **0.3 s** of clear time before it, or a line
   still ending would be cut and repeated. Music videos are never looped: their audio rarely matches the album cut the
   lyrics are timed to (`LoopMix.kt:159-161, 176`).
2. **open-unmix "vocals" (umxhq, int8)** (`VocalTracker.kt:69-271`). MIT weights (htdemucs's are non-commercial and
   its ONNX export is blocked by complex STFT ops, `:75-79`). Linear STFT 4096/1024 at 44.1 kHz, fixed input of
   **960 frames (~22 s)**. Per frame it averages the ratio *target magnitude / mix magnitude*, clipped to 0..1, over
   **200–4000 Hz** and both channels; padded frames are excluded. This is an *unweighted* mean of per-bin ratios.
3. **DSP heuristic** (`audio_analysis.cpp:297-313, 683-685`): a logistic over the log-compressed band shares
   (< 250 Hz, 250–4000 Hz, > 4 kHz) and spectral flatness, with frames under 0.25 of reference energy scaled by 0.3.
   Rescaled so its old decision point (0.6) maps onto the model's threshold (`Policy.kt:99-115`).

**Threshold, with its measurement** (`Policy.kt:87-97`; also the web `loopMix.ts:37-42`): on the open-unmix mask a
clear chorus reads **0.57–0.71**, a soft verse over the band **0.2–0.4**, an instrumental break **0.02–0.15**. The old
threshold of 0.6 called soft verses instrumental, "which is how a sung line got looped". It is now **0.25**.
Unmeasured instants sit at 0.2, just under it, so absence of evidence never trips vocal logic either way.

### 1.7 Kick presence, key, loudness

- **Low band** (`audio_analysis.cpp:365, 386-389, 648-667`): per ~0.65 s frame, sum of log1p FFT power under
  **250 Hz**; normalized to its 85th percentile; resampled onto the 240-point grid; zero where the full-band level is
  ≤ 0.1. This is *low energy*, not kick onsets: a sustained bass reads as a kick.
- **"Carries a kick"** (`Planner.kt:289-317, 721-727`): mean low energy over **8 beats** before the exit and 8 beats
  after the incoming start, each ≥ **0.5** of that track's reference (max of the 90th percentile and ¼ of the
  maximum).
- **Key** (`audio_analysis.cpp:319-427`): whole-track chroma from 4096-point frames hopped ≥ 0.65 s, 45 Hz–5 kHz, each
  bin weighted by log1p power × frame RMS; Krumhansl–Kessler major/minor profiles; confidence =
  4 × (best − second) / best. Trusted at ≥ **0.25** (`Planner.kt:363-365`).
- **Compatibility** (`Planner.kt:345-361`): same mode: ≤ 2 semitones or a fourth/fifth ("the move every DJ makes");
  different modes: ≤ 1 semitone. Note: **relative major/minor (3 semitones, different mode) counts as a clash**.
- **Loudness**: RMS dBFS − 0.691, explicitly *not* gated K-weighted LUFS (`audio_analysis.cpp:607-619`). The loop mix
  scales the incoming deck by the ratio of the app's normalization gains (`Controller.kt:2554-2555`).

---

## 2. The planner (blends, filter rides, crossfades)

The Android controller always runs with "Advanced" on (`Controller.kt:1096-1097`), so the classic branches are dead
on the phone. The order of decisions per tick:

### 2.1 Whole decision tree

0. **Loop mix first** (`Controller.kt:1131-1147`): if `LoopMix.plan` returns a plan (§3) and the next stream is
   ready, it is armed and nothing below runs. Refused when: no native renderer, playback speed ≠ 1×, skip-silence on,
   sleep timer, either analysis not final, a source-backed track, an output that is not stereo float PCM, no standby
   sink, lyrics not yet looked up (`Controller.kt:2425-2470`).
1. `planTransition` (`Planner.kt:1408-1675`):
   1. no duration → blocked; mode not "smart" → plain crossfade at the end (setting, 1–12 s).
   2. track < **45 s** → no transition (`:86-87, 1494-1496`).
   3. rank mix-out candidates (§2.3); the best is the anchor.
   4. **album played in order** and same album and no interior exit → `GAPLESS` (starts 0.45 s before the end, 0.12 s
      fade). Only when the caller says it is an album playthrough: shuffled album siblings are mixed
      (`:319-329, 1512-1524`).
   5. spoken word (podcast, episode, audiobook, concert) or a labeled live recording on either side → blocked.
   6. analysis not ready → plain fade of the setting length (6 s default, `Controller.kt:3864`).
   7. tier (`Policy.kt:477-514`): a tempo outside **40–220 BPM**, or **both** confidences < **0.2** → `EQUAL_POWER`
      ending at the anchor, incoming cued at its audible start.
   8. **unmixable pair** (`Planner.kt:271-287, 1569-1598`): tempo not reachable by stretch **and** (trusted keys
      clash **or** both whole-track vocal probabilities ≥ **0.62**) → `EQUAL_POWER` ending at the **content end**,
      into the incoming intro. "Every DJ gesture put two loud unsynced songs on top of each other."
   9. fixed 8/16-bar phrase mode (`:490-649`): implemented, not reached on Android (no caller passes `phraseBars`).
   10. **phrase switch** (`:1059-1133`): both keys trusted and compatible, tier `BEATMATCHED` (both confidences
       ≥ **0.55**, rate within the window) → beat-matched `DJ_BLEND` run over the incoming intro and closing on its
       drop.
   11. otherwise the **adaptive plan** `mixPlan` (`:1677-1868`), then a **vocal reroute**: when its vocal overlap is
       > **0.3**, the next 3 ranked exits are planned too, and one is taken if it at least halves the overlap
       (`:1177-1188, 1646-1664`).
   12. **incoming skip cap**: the cue never goes past **25 %** of the incoming song; a plan past it is pulled back by
       whole incoming bars (`:413-479`). "A main drop two minutes in … the listener arrived at a song with twenty
       seconds left."

### 2.2 Style for an adaptive pair (`mixPlan`)

- **Stretch window** (`Policy.kt:60-85`): the incoming song may be sped up by up to **25 %** and slowed by up to
  **10 %**, asymmetric because speeding up drops slivers and stays clean far past 10 %, while slowing down repeats
  them and smears metallically. The audible outgoing deck is never retimed: changing an AudioTrack's speed mid-play
  cracked on some routes (`Planner.kt:1141-1154`).
- **Same-beat blend** (`Planner.kt:1707-1714`): tempos within **5 %** (octave-folded at 1.5/0.67) with at least one
  confidence ≥ 0.2, **or** both confidences ≥ 0.55 and the tempo reachable. In Advanced, only if the incoming rate is
  ≥ 1, i.e. an adaptive blend only ever *speeds up* the incoming song (`:189-195, 1709`). A slower-needed pair falls
  to a filter ride that still carries the slowed rate (`:1847`). Only the phrase switch (§2.1 step 10) blends with the
  incoming song slowed, by up to 10 %.
- Same-beat → `DJ_BLEND` with a bass swap; otherwise `DJ_FILTER` with a gesture chosen deterministically from the
  material (`:148-164, 299-317`):
  - **BASS_HANDOFF** when both sides carry a kick at the join (§1.7);
  - **SWEEP** otherwise, and always when the vocal overlap is ≥ **0.18** (`:1180-1181, 1832-1837`);
  - **ECHO_RIDE** never: "a beat echo at the outgoing tempo flams over an incoming track at another one".

### 2.3 Scoring entry and exit points

- **Exits** (`Policy.kt:383-469`): score = analyzer score + type weight (`energy_cliff` and `interior_mix_out` 0.95,
  `outro_start` 0.9, `content_end` 0.75). Any candidate that would skip more than **12 s of audible music** is
  dropped (audible = energy ≥ 10 % of the 85th-percentile reference; silence is free). Ties go to the later time.
  Vocal boundaries are deliberately *not* exits: "vocals describe how the overlap should be shaped, not where the
  outgoing song stops" (`:399-403`).
- **Entries** (`Policy.kt:344-381`): analyzer score + type weight (`main_drop` 0.5, `intro_drop` 0.4, `pickup` 0.15,
  `phrase` 0.1); +0.1 if within half a beat of a downbeat; −0.2 for a **cold open** (less than 4 beats of intro before
  it); + (0.5 − mean vocal over the 16 beats before it) × 0.4, so a sung run-up loses.
- **Start snapping** (`Planner.kt:385-411`): to a phrase line within max(1 s, 4 beats), else a downbeat within
  max(0.75 s, 2 beats), else raw. Blends prefer the line at or before the target.
- **Arrangement overlap** (`:665-672, 1715-1721, 1761-1766`): when the exit is just the content end, both decks move
  back by **8 beats**, so the incoming drop lands *inside* the blend and the outgoing arrangement is still playing,
  instead of the drop being the finish line.
- **Exit on the one** (`:1722-1728`): the end is pulled back onto a downbeat within 4 beats.

### 2.4 Length

| case | beats | seconds |
|---|---|---|
| tempo far, SWEEP | 12 | cap **8 s** |
| tempo far, BASS_HANDOFF | 8 | cap **7 s** |
| key clash or both sung (whole-track) | 8 | min 4 s (6 s at ≥ 140 BPM), cap 12 s |
| any other pair | 16 | same |
| both edges calm (< 0.7 of mean energy over 8 beats) | × **2**, at most 32 beats | cap **16 s** |

Sources: `Planner.kt:67-78, 1167-1175, 1232-1253, 1280-1333, 1729-1745`. Further caps: 40 % of the time before the
exit and 40 % of the incoming song. Everything rounds to whole bars. A same-beat blend aims to cover the incoming song
from its cue to its handoff (drop + 8 beats), clamped to [min(12 s, max), max], so **most blends are 12 s or 16 s**
(`:1779-1793`). Filter rides use 4 beats (2–5 s) as their floor (`:1746-1752, 1794-1810`) and cue the incoming song at
its **audible start**: "cueing an unmatched song at its drop put two full arrangements on top of each other".

Listening reasons recorded for these lengths:

- "the old 10–12 second sweep made every fallback sound like the same ordinary crossfade", so unmatched fallbacks are
  compact and differ by gesture (`:1300-1303`);
- the energy factor only lengthens: halving it when two peaks met made blends shorter than before, "which read as a
  regression" (`:1232-1237`);
- the beat-length bound exists because a seconds bound makes a faster track get a longer mix, "which is backwards"
  (`:655-658`).

### 2.5 Phrase switch length (`planWsolaTransition`, `Planner.kt:910-1057`)

- Fade beats = min(16, the beats fitting 16 s rounded down to a bar, the incoming intro's whole bars), floor 4.
- **Vocal shrink loop**: while the overlap clashes, shorten by 4 beats (down to 4). A clash is any of:
  - over **5 %** of instants sung on both sides at once (`VOCAL_CLASH_TOLERANCE`, `Policy.kt:285-295`), walked
    instant by instant with the incoming time scaled by the stretch (`Policy.kt:231-283`);
  - both window means ≥ 0.25;
  - over 8 beats, the outgoing side still sung in its earlier part.

  Why instant by instant: a mean hides clashes, e.g. 3 s of two voices inside 12 s averages "clear"
  (`Policy.kt:235-241`).
- Incoming handoff = drop + 8 beats; cue = handoff − overlap; start = the outgoing downbeat at or before
  (exit − 8 beats if the exit is the content end) − overlap.

### 2.6 Bass swap placement (`Planner.kt:684-817`)

- Prior **0.7** of the overlap. Allowed range [min(0.5, latest), latest], where latest = min(**0.85**, 6 s / overlap).
- Each candidate beat scores the **rise** in incoming low energy across that beat minus the rise in outgoing low
  energy (windows of one beat, or 1.1 × the curve resolution).
- The best wins if it scores ≥ **0.25**; otherwise the beat nearest the prior, preferring bar lines.

### 2.7 Variety and repetition

- Only between gestures, and deterministic per pair (`Planner.kt:148-155`: a random choice would make the marker and
  the renderer change their minds between ticks).
- No memory across transitions, no repeat penalty, no energy arc over a session.
- Loop mixes of matched tempos all come out about **8 bars** long (§3.4).

---

## 3. The loop mix

Plan: `LoopMix.kt` (pure). Clock: `LoopMix.Clock`. Decks and render: `renderer.cpp`. Envelopes and setup:
`Session.kt:113-227`. Splice and alignment: `Port.kt`, `Controller.kt:2525-2845`.

### 3.1 Choosing loops (`LoopMix.kt:150-443`)

- **Search spans:**
  - outgoing: [max(playhead + **15 s**, end − **60 s**), end − 0.5 s], **latest first**;
  - incoming: [0, min(45 s + 4 bars, length − 20 s)], **earliest first**.
- **Lengths** tried **4, then 2, then 1 bar**; "a 1-bar instrumental loop beats a 4-bar one with a word in it"
  (`:45-49`).
- **Loop boundaries:**
  - with model downbeats (≥ bars + 1 of them in that end's `BeatSide`): from a detected downbeat to the detected
    downbeat nearest *bars × 4* beats later, rejected if that is more than ¼ beat off or past the span;
  - otherwise a constant grid from the first downbeat (`:254-285`).
- **Grooves** (loudness): the loop's **quietest bar** RMS (from the 10 fps level windows) must be ≥ **0.6** × the
  60th percentile of the quietest-bar values among all candidate loops of that length, and above **0.01** RMS
  (−40 dBFS) (`:89-93, 334-343, 379-405`). This skips breakdowns and fade-outs.
- **Instrumental**, the first source that applies:
  - *lyrics*: no certain sung span may touch [loop start − 0.3 s, loop end]; if only guessed tails touch, the mask
    decides;
  - *mask*: open-unmix windows must cover ≥ **80 %** of [loop start − 1 beat, loop end]; ≤ **3 %** of frames
    ≥ 0.25 and no sung run longer than **0.2 s** ("a breath").

  No lyrics and no mask means no loop.
- **Rejection counts** (quiet, sung by lyrics, sung by mask, unmeasured) are kept for the log and the stats line
  (`:309-322`).
- **Not checked**: key, seam continuity, fills, phrase position, how much music is skipped.

### 3.2 Master tempo curve (`LoopMix.kt:203-232, 470-531`)

- Each loop's tempo is its own: length / (bars × 4) per beat.
- **Fold**: the incoming tempo is taken at ×1, ×2 or ×0.5, whichever is nearest in log ratio (so 111 → 169 BPM ramps
  to 84.5, half time). This differs from the policy's fold (thresholds 1.5 / 0.67, `Policy.kt:161-171`).
- **Entry**: the master tempo holds the outgoing BPM for **4 bars** (`ENTER_BARS`) while the incoming loop comes in at
  that tempo.
- **Ramp**: ceil(|target − out BPM| / **2.5**) bars, clamped **2–24**; "past this a ramp is heard as a lurch, not a
  mix" (`:60-63`). The tempo during outgoing beat *k* is linear in *k*, sampled at the beat's middle, so it moves in
  steps of 0.625 BPM per beat (0.52 % per beat at 120 BPM).
- **Exit**: **2 bars** (`EXIT_BARS`) at the target tempo, then the total is **rounded up to whole incoming loops**
  (the incoming loop can only let go at its own end), so the real exit is 2 bars or more (`:211-213, 230`).
- **Settle**: **2 bars** of the incoming song at its own rate after release, then its own player takes over
  (`SETTLE_BARS`, `:57-58, 479`).
- **Beat times**: a cumulative sum of 60 / tempo(k). Derived times: entry end, ramp end, handoff (the ramp's middle
  beat; the UI and queue move there), swap bar length (one bar at the mid-ramp tempo), release, player takeover
  (`:476-531`).

Worked example: **120 → 160 BPM** gives fold 1 and a 16-bar ramp. 4 + 16 + 2 = 22 bars rounds up to 24 (the incoming
loop is 4 bars), so the exit is 4 bars. That is ~8 s + ~27 s + ~6 s ≈ **41 s**, plus 3 s of settle. **120 → 120**:
ramp 2 bars (the minimum), total 8 bars ≈ 16 s.

### 3.3 Per-beat position knots (exactly how each deck follows the tempo)

- **Knots** (`Session.kt:150-189`, `LoopMix.kt:511-513`): one per mix beat *k* = 0..beats. Time = beat time *k*.
  Position, *unwrapped*:
  - outgoing: loop start + *k* × out beat length;
  - incoming: loop start + *k* × fold × in beat length.

  So during beat *k* the outgoing deck plays at rate tempo(k) / out BPM, and the incoming one at
  fold × in-beat × tempo(k) / 60, which is exactly **1** at the ramp's end (asserted in `LoopMixTest.kt:168-170`).
- **Wrapping** (`renderer.cpp:199-224`): unwrapped positions past the loop end wrap modulo the loop length until the
  deck's release. After release they run on from the loop end (the incoming deck), so the song continues seamlessly
  into what follows its loop. Release is placed at a whole number of incoming loops (`LoopMixTest.kt:171-174`).
- **Rendering** (`renderer.cpp:289-371`): each deck runs Signalsmith Stretch with the "cheaper" preset and split
  computation (latency is free because the mix is written far ahead of the speaker, `:250-263`). Per output block:
  - heard position at the block end = linear interpolation of the knots (or rate 1 after the last knot for the
    incoming deck);
  - samples to read = (heard end + input latency + rate × output latency) − read position, clamped to 0..8× the block;
  - the stretcher time-scales that many input samples into the block.

  So the **heard position lands on the knot at every block boundary**: no drift, no servo. Position is piecewise
  linear per beat, so the rate is constant within a beat and steps at beat lines.
- **Start**: the stretcher is seeded with `seek()` **0.4 s** of pre-roll plus its output latency before the first
  knot, so its first audible sample is already settled (`renderer.cpp:298-327`, `Session.kt:166, 186`).
- **Loop seam**: a **hard wrap** in the input stream; no crossfade, no zero-crossing search. The stretcher's overlapping
  STFT windows smear the seam. The web build measured "beats at loop seams within 2 ms of the rest" and decks "locked
  within 5 ms of each other" (web doc 131-133).

### 3.4 Envelopes, bass swap, exit (`Session.kt:191-225`, `renderer.cpp:373-420`)

- **Gains** (linear amplitude, linear in time):
  - outgoing deck: 0 at −25 ms, 1 at 0, 1 at entry end, **0.6** at ramp end, 0 at release;
  - incoming deck: 0 at 0, **0.55** at entry end, 1 at ramp end.

  The sum peaks near **1.55** at the entry end. The loop renderer has **no headroom trim or limiter**; the blend path
  has one (`Controller.kt:3818-3821`).
- **Bass swap**: a **250 Hz low shelf** (RBJ, slope 1) on both decks. The incoming deck starts at **−18 dB**; over
  **one bar** centered on the ramp's middle beat the two decks trade 0 ↔ −18 dB, linearly in dB. Both decks sit at
  −9 dB at the midpoint, so two kicks are half present for a bar (unlike the blends' half-beat high-pass, §4).
- **Drain**: outgoing high-pass (RBJ biquad, Q 0.7, 12 dB/oct) from **20 Hz to 1.5 kHz**, exponential, from ramp end
  to release, while its gain falls 0.6 → 0.
- **Control rate**: filter coefficients are recomputed per **64-frame** block when the shelf moves > 0.01 dB or the
  corner > 0.5 Hz, without smoothing; gain is interpolated per sample (`renderer.cpp:150-151, 373-419`).
- **Incoming handover**: the incoming deck fades out over **25 ms**, measured on its *heard* track position, at the
  takeover point, while the real player fades in over 25 ms at the same track position (`renderer.cpp:152-153, 405-411`,
  `Port.kt:18-23`). "Identical audio crossed this quickly is heard as one" (`LoopMix.kt:99-100`).

### 3.5 Splicing into the real players (Android plumbing)

These parts exist only because BitChord plays through two ExoPlayers; nori's single engine does not need them.

- The renderer runs **inside the outgoing player's audio sink**. At the decoded frame where the song reaches loop
  start, the sink crosses from the song to the renderer over 25 ms, sample-exact (`Port.kt:6-27`).
- The incoming player starts silently **0.5 s** into the mix. After **1.2 s** of settling, its offset to the deck is
  measured on both sinks' clocks (median of **15** readings, 30 ms apart, up to **3** rounds) and cancelled by
  dropping frames or inserting silence before it becomes audible; aligned means ≤ **1.5 ms** (`Session.kt:55-95,
  229-232`, `Controller.kt:2688-2729, 4287-4291`).
- **Self-check**: 8192 frames of the song and of the deck's pre-roll are cross-correlated over ±960 frames, and the
  lag is logged (`Port.kt:120-169`, `Controller.kt:2644-2652`).
- **Bail**: pause, seek or flush settles onto whichever song is ahead, at the mix clock's position for it
  (`Controller.kt:2792-2829`).

---

## 4. Blend, filter ride and bass hand-off

### 4.1 Filters (`FilterProc.kt:18-26, 416-540`)

- One low-pass and one high-pass per player, each **two cascaded TPT (trapezoidal) state-variable sections** with
  Butterworth section Q **0.5412** and **1.3066**: a 4th-order, **24 dB/oct** Butterworth (not Linkwitz–Riley).
- The corner glides in the log domain by **5 %** of the remaining distance every **64 frames** (~30 ms time
  constant); corners are clamped to 10 Hz … 0.45 × fs; high-pass ≤ 2 kHz. Open = 20 kHz low-pass and 20 Hz high-pass.
- **Lead compensation** (`FilterProc.kt:57-69`, `Controller.kt:1821-1826, 1629-1640`): the filter runs where audio is
  *written*, up to ~4 s ahead of the speaker, while player volume acts at the speaker. So each filter is aimed at the
  blend position its audio will be heard at, and the incoming filter is set to its opening position before its buffer
  fills.
- All cutoff motion is **geometric** ("pitch is logarithmic", `Controller.kt:3382-3391`). "Bass cut amount" *a*
  means a high-pass at 20 Hz × 10^a (20 → 200 Hz).

### 4.2 Faders (`Controller.kt:3484-3545`, constants `:3985-3998`)

- Base pair: sin/cos equal power.
- **DJ_BLEND with bass swap**: the incoming fader is fully up **by the swap** (sin over [0, swap]). The outgoing fader
  is held at 1 until `fallFrom` = min(swap, 0.5, 1 − 4 beats / overlap), then falls on cos. Holding to the swap and
  then dropping in the last beats "was heard as the song cutting out", hence the 0.5 and one-bar limits
  (`:477-484`). Both are pulled back toward plain equal power in proportion to the vocal clash.
- **DJ_FILTER**: incoming sin(π/2 · p^**0.8**) (early, behind its high-pass); outgoing cos(π/2 · p^**1.6**) (held,
  the low-pass already takes it away).
- **Vocal-safe shape**, blended in by the clash amount: incoming p^**1.65**, outgoing p^**0.65**.
- **Headroom**: if the faders sum above 1, both are scaled by 1/√sum (`:3818-3821`).
- **Progress clock**: the incoming player's position since its cue, extrapolated between Media3's ~280 ms position
  steps (`:577-593`), and scaled by the stretch (a 4 % stretch otherwise ended blends 4 % early, `:1798-1802`).

### 4.3 Bass-swap blend (`rideBassSwap`, `Controller.kt:3393-3462`)

- **Swap**: the outgoing high-pass rises 20 → **200 Hz** over **half a beat ending on the swap beat**, while the
  incoming one falls 200 → 20 Hz. Both are 24 dB/oct. Half a beat is "a hand on the EQ, not a kill switch": long
  enough that the corner moving does not click, short enough that two kicks never play together (`:3968-3974`). A
  24 dB/oct filter appearing in one buffer is a transient of its own (`:3399-3401`).
- **Incoming entry high-pass**: from **520 Hz** (950 Hz at full clash), descending with shape exponent **0.35**, fully
  open by **0.45** of the blend (0.7 at full clash); the higher of this and the swap corner wins.
- **Outgoing exit low-pass**: open until **0.3** (0.12 at full clash), then to **2.2 kHz** (1.1 kHz at full clash) by
  the end.

### 4.4 Filter ride for unmatched tempos (`rideFilterSweep`, `Controller.kt:3268-3324`)

- **Outgoing low-pass** starts at **7 kHz** at once and falls to a **300 Hz** floor along progress^**0.75**
  (front-loaded): it crosses 5 kHz a twentieth of the way in, 3.8 kHz at a tenth, 2.6 kHz at a fifth, 1.0 kHz at the
  midpoint (`:4072-4095`).
- **Incoming high-pass** from **1.2 kHz** (**2.4 kHz** at full clash), shape 0.35, open by **0.6** (**0.78** at full
  clash).
- **Bass**: one exchange at **0.5** of the blend, snapped to a bar line, over half a beat, as in §4.3, so only one
  kick ever plays.

### 4.5 Bass hand-off for two kicks (`rideFallbackBassHandoff`, `Controller.kt:3326-3357`)

- Swap at **0.43** (bar-snapped). Incoming high-pass from **900 Hz** (2.4 kHz at clash), open by **0.38**.
- The outgoing low-pass stays open until the swap, then closes to **1.6 kHz** along a 0.8 exponent.
- With a vocal clash, the outgoing low-pass starts closing toward 1.6 kHz from the beginning (clash × p^0.55).

### 4.6 Plain fades with two voices (`rideVocalSeparation`, `Controller.kt:3230-3266`)

- Even `EQUAL_POWER` separates voices when both masks say they collide. The reason: the tier is decided by tempo
  evidence, which may be weak while the vocal evidence is strong, and "the weak half of the evidence was silencing the
  strong half" (`:3174-3186`).
- Outgoing low-pass down to **1.6 kHz** × clash; incoming high-pass from **700 Hz** × clash, open by 0.6.

### 4.7 Echo out (implemented, never planned)

- Settings: 1-beat delay (≤ 1 s), feedback **0.4**, wet **0.5**, one-pole **250 Hz–3.5 kHz** band in the loop, about
  5 repeats; the dry signal fades over **4 beats** after the echo beat (`FilterProc.kt:517-534`,
  `Controller.kt:4000-4013`).
- Every plan sets the echo time to 0 (`Planner.kt:1833-1836`).

### 4.8 Tempo handling in blends

- The incoming song plays at one fixed rate, rounded to 1e-4, for the whole blend.
- Afterwards it **eases back** in steps of at most **0.75 % per beat**, one per beat of its own track: a 10 % stretch
  takes 14 beats. "Gradual drift below about 1 % a beat reads as the groove, not as the track changing speed"
  (`Controller.kt:3684-3747, 4050-4055`).
- These are plain speed changes. A short gain notch around each change was tried and was "heard as a pump"
  (`:3694-3696`).

### 4.9 Phase-lock servo (designed, switched off)

`holdPhase`, `Controller.kt:3547-3681`, constants `:4015-4048`:

- Phase error = incoming minus outgoing beat phase, in outgoing seconds, wrapped to the shorter of the two beat
  periods (so half or double time still locks).
- Readings start **300 ms** after the incoming start. The median of **5** readings is acted on unless under **8 ms**;
  an error over **0.3 beat** is treated as grids disagreeing and ignored.
- A correction is a pulse of ±**3 %** speed lasting |error| / (rate × 0.03), at most **1.5 s**. Then it waits
  **400 ms** for the buffered audio to play out, or **500 ms** after an in-phase reading. It stops at **90 %** of the
  blend, and the total speed-up stays ≤ **1.12**.
- Pulsed, not continuous, because a speed change is only heard after the buffered audio plays out, so a continuous
  loop oscillates.
- **Off in every plan**: "Live phase-lock pulses changed AudioTrack playback parameters mid-buffer and cracked on some
  phones" (`Planner.kt:1862-1864`; also `:644`, `:1130`).

So a phone blend starts on a **40 ms** poll plus the player's start latency, and nothing corrects it; the comment on
the servo itself says this lands "anything up to tens of milliseconds off the beat — the galloping of a bad mix"
(`Controller.kt:3555-3558`). The web build keeps a servo (every 40 ms, up to 3 % audible, 10 % while the deck is below
gain 0.3, a seek past 250 ms) and measured **−82 ms** at the first sample, −8 ms a second later, then −9 to −14 ms
(web doc 72-74, 76-81).

---

## 5. Quality checks and listening findings

### 5.1 Tests

- `LoopMixTest.kt` (25 cases):
  - whole bars, latest out and earliest in;
  - prefers mask-clean bars, skips a fade-out;
  - fold to double or half time;
  - drift (0.02 % per beat) still cut on detected downbeats;
  - never cuts on a misplaced downbeat;
  - clock invariants: wraps only at loop ends, incoming rate exactly 1 at ramp end, release on a whole loop;
  - position before, during and after the mix; beat phase 0 on every beat;
  - lyric spans with padding and open lines; a soft verse (mask 0.3) counts as sung;
  - falls back to a 1-bar loop; no loop when nothing is instrumental, for music videos, or without a mask and lyrics.
- `AdvancedAutomixPlannerTest.kt`: unmixable pair → end crossfade; tempo-only mismatch → filter; gesture follows the
  low end; unmatched pairs never echo and enter at the intro; two singers → vocal-safe sweep; outgoing never retimed;
  outside the stretch window → not matched; never skip > 25 % of the incoming song; whole bars.
- `PhraseBarAutomixTest.kt` (8 and 16 bars exactly on the grid), `SpokenOrLiveTest.kt`.
- Web: `beatgrid.test.ts` (grid across a beatless breakdown, jitter), `loopMix.test.ts` (mirror of the Kotlin cases),
  `stretch.test.ts` (window limits), and a synthetic **124 BPM A-minor** self-test expecting 123.93 BPM and 8A
  (web doc 92-98).

### 5.2 Runtime measurements

- Splice cross-correlation and alignment error are logged per mix (§3.5).
- Web doc: per-pair phase errors, grid confidences per song (rubato *Hello* 0.00, *DNA.* with a tempo change 0.06,
  steady pop and dance 0.25–0.75), click-track handovers within 5–8 ms (web doc 53-54, 76-91).
- Not done: a headed listening pass, Firefox (web doc 100-104).
- **No audio-level quality metric** at render time: nothing measures flams, two kicks at once, loudness bumps, seam
  clicks or vocal overlap in the rendered output.

### 5.3 Listening findings recorded in comments (in our words)

| # | finding | where |
|---|---|---|
| 1 | A filter sweep squared on progress left the outgoing cutoff at 6.9 kHz at the midpoint: two full vocals through the first half, i.e. a plain crossfade. Exponent 1.3 was still back-loaded; **0.75** (front-loaded) works, with the 300 Hz floor guarding against gutting. | `Controller.kt:3275-3297, 4072-4095` |
| 2 | Starting the sweep at 20 kHz wasted the first tenth of the fade (17.5 kHz, inaudible). **9 kHz** still read as late; **7 kHz** sits in the presence range, so the voice changes at once. | `:3926-3946` |
| 3 | Below a few hundred hertz a low-passed track stops sounding distant and starts sounding **broken**: floor **300 Hz**. | `:3948-3953` |
| 4 | Filtering only the outgoing track left both vocals competing; a DJ hands the midrange over: the incoming track enters high-passed. | `:3273-3291` |
| 5 | An incoming high-pass at 700 Hz left almost three octaves shared with the closing low-pass, exactly where vocal fundamentals sit; **1.2 kHz** removes the octave where the collision happens. | `:4097-4115` |
| 6 | A plain geometric high-pass descent reached 123 Hz a third of the way in, doing nothing for two thirds; shape **0.45 → 0.35** keeps the corner in the voice range through the middle of the blend. | `:3368-3375, 4127-4145` |
| 7 | Vocal separation at 450 Hz was "mostly inaudible in practice" (268 Hz a fifth of the way in): **700 Hz**. | `:4159-4175` |
| 8 | A 320 Hz blend entry high-pass was swallowed by the 200 Hz bass kill; **520 Hz** gives an entry gesture that outlives it. | `:4177-4189` |
| 9 | Starting the blend's exit low-pass at the midpoint was "remembered late"; **0.3** of the way in. | `:4215-4227` |
| 10 | The bass changes hands over **half a beat**: no click, and two kicks never sound together. | `:3966-3974` |
| 11 | Holding the outgoing fader to the swap and dropping it in the last beats sounded like the song cutting out; the fall starts by the midpoint and at least a bar before the end. | `:477-484` |
| 12 | An echo whose dry signal was killed in a quarter beat sounded like the song cutting out; **4 beats**. A long run of repeats over the next song sounded like a glitch: feedback **0.4**. An echo at the outgoing tempo flams over an incoming song at another tempo, so it is never planned. | `:4000-4005`, `FilterProc.kt:517-524`, `Planner.kt:296-297, 1833-1836` |
| 13 | Fallback sweeps of 10–12 s all sounded like the same ordinary crossfade: compact, varied gestures instead. | `Planner.kt:1300-1303` |
| 14 | Making two peaks meet in a shorter blend read as a regression; energy only lengthens. | `:1232-1237` |
| 15 | Cueing an unmatched song at its drop put two full arrangements on top of each other: cue at the intro. | `:1806-1809` |
| 16 | Pairs with unreachable tempos plus a key clash or two singers: every gesture sounded like two songs fighting; a plain end crossfade sounds right. | `:1569-1573`, test `AdvancedAutomixPlannerTest.kt:74-75` |
| 17 | A drop two minutes in left the listener with 20 s of the next song: cue ≤ 25 %. | `:413-419` |
| 18 | Tempo moves under ~1 % per beat read as groove; the ease is **0.75 %/beat**. A master ramp past **2.5 BPM/bar** sounds like a lurch. | `Controller.kt:4050-4055`, `LoopMix.kt:60` |
| 19 | Time-stretch: speeding up stays clean past 10 %, slowing down smears metallically: window **0.9–1.25**. | `Policy.kt:60-70` |
| 20 | Speed changes on an audible Android output crack; a gain notch around each was heard as a pump. | `Planner.kt:1141-1146, 1862-1864`, `Controller.kt:3694-3696` |
| 21 | Two players rendering the same audio are 9–41 ms apart, heard as the outgoing song's last instant twice; so the second player always carries the *incoming* song. | `Controller.kt:82-96` |
| 22 | Vocal threshold 0.6 called soft verses instrumental and a sung line got looped; **0.25** from measured mask values. | `Policy.kt:87-97` |
| 23 | Mean vocal activity over a window hides real clashes; measure simultaneous instants. | `Policy.kt:231-241` |
| 24 | With weak tempo evidence, plain fades left two vocals on top of each other; vocal separation applies to every style. | `Controller.kt:3174-3186` |
| 25 | A 1-bar instrumental loop beats a 4-bar one with a word in it. | `LoopMix.kt:48` |
| 26 | Moving the session to the incoming song at its first, barely audible note was wrong; at the midpoint (or the swap). | `Controller.kt:1651-1659` |
| 27 | Beat This! with quantized convolutions found no beats; MatMul-only int8 matches the float model. | `BeatTracker.kt:219-230` |
| 28 | Integer 50 fps peaks cost 20 ms of flam budget: parabolic refinement. | `BeatTracker.kt:259-263` |
| 29 | One beat grid cannot fit both ends of a song. | `TrackAnalysis.kt:98-103`, `NOTICE` |
| 30 | A beat interval of 24.7 s was once stored for a 127 BPM track: the BPM is the authority, and an interval off by > 25 % is ignored. | `Controller.kt:3651-3654` |
| 31 | Positions read between Media3's ~280 ms steps made faders and filters lurch: extrapolate. | `Controller.kt:577-593` |

---

## 6. Weaknesses and opportunities

### 6.1 Analysis

1. **No beat-level warp.** Model beat lists exist per end, but decks assume even beats inside each loop, and blends
   use one constant rate. A live drummer's push and pull inside four bars, or a swung or humanized track, is not
   followed.
2. **Phrases are a lattice, not detected.** 32 beats from the first DSP downbeat (§1.5), so a pickup bar, a 2-bar
   insert or a 6-bar intro shifts every line. The mix-in candidates are points on that lattice, not detected drops.
3. **Meter is assumed 4/4.** A 3/4 or 6/8 song gets "bars" of four beats, so its loops are cut mid-bar.
4. **The vocal measure is crude.** An unweighted mean of per-bin mask ratios: quiet bins count as much as loud ones;
   reverb tails and backing vocals are not distinguished; the threshold of 0.25 sits next to soft-verse values.
5. **Kick presence is band energy**, not onsets: a bass pad or a sub-heavy synth reads as a kick.
6. **Key is whole-track, Krumhansl–Kessler, 11 kHz**, with no per-end key and no tuning estimate. Compatibility
   rejects **relative major/minor**, the most common harmonic DJ move.
7. **Energy resolution** is ≤ 240 points per track (~1 s), too coarse for bar-level decisions; the loop picker works
   around it with its own 10 fps level windows.
8. **No loop-specific features**: seam similarity, fills, chord changes at the seam, onset density per bar.

### 6.2 Planner

1. **The loop mix pre-empts everything** (`Controller.kt:1131-1147`). A matched-tempo pair with a perfect outro and
   intro still gets two short loops repeated for ~8 bars instead of a blend of the real music. There is no scoring
   between families.
2. **No skip budget in the loop mix.** The outgoing loop can sit up to 60 s before the end, and everything after it
   is never played; the incoming song's first ≤ 45 s before its loop are never played. The blend planner caps skipped
   music at 12 s and the incoming cue at 25 %; the loop mix bypasses both.
3. **No key check in the loop mix**, although two loops play together for 4 + ramp bars (up to ~30 bars).
4. **Fixed shape.** Entry 4 bars and exit 2 bars always; the ramp length depends only on the tempo gap; same-tempo
   loop mixes are always about 8 bars. No energy, structure or session-level variety.
5. **The tempo ramp is linear in absolute BPM per bar**, with a corner at each end. 2.5 BPM per bar is 3.1 % at 80 BPM
   and 1.5 % at 170 BPM: perceptually uneven. The rate steps per beat (fine), but its slope jumps from 0 to maximum at
   the ramp's start and back at its end.
6. **Blends are not beat-locked** on the phone (§4.9); the most "DJ" style is the least precise one.
7. **Adaptive blends only speed up the incoming song** in Advanced (`Planner.kt:1709`), and the outgoing deck never
   moves. A 5 % gap is all on one deck, and a song that needs slowing gets a filter ride unless the stricter phrase
   switch takes it.
8. **Two folding rules** (policy thresholds 1.5 / 0.67 vs the loop mix's nearest log ratio).
9. **Dead or unreachable paths**: phrase-bar mode (no caller passes it), echo ride, phase lock, and every classic
   branch.

### 6.3 Loop mix rendering

1. **Linear amplitude gains** summing to 1.55 with no headroom or limiter in the loop renderer.
2. **Bass "swap" by a −18 dB shelf over a whole bar**: both kicks are present at −9 dB in the middle of the bar, and
   18 dB of shelf leaves the sub of the incoming loop audible during the whole entry. This is weaker than the blend's
   half-beat 24 dB/oct exchange.
3. **Seams are hard wraps**. They are inaudible only as long as the stretcher's window smears them and the loop is
   rhythmically and harmonically closed; a sustained pad or a reverb tail jumps at every seam.
4. **Coefficient switching without smoothing** at 64-frame blocks (TDF-II biquads), fine for slow sweeps but a risk
   for fast ones.
5. **The incoming song enters at its loop**, not at its first bar, and leaves its loop at the loop end: the intro the
   listener would normally hear first is replaced by a repeated slice.

### 6.4 Their own open items

- Web doc 100-104: no headed listen yet, Firefox untested, SABR-only tracks fall back.
- The comments mark the heuristic vocal mask and DSP grid as interim (`audio_analysis.h:111-114`,
  `TrackAnalysis.kt:37-41`).
- Echo and phase lock are kept in code for later.

---

## 7. How nori can do it better

nori already has most of the foundations BitChord lacks:

- one engine rendering both decks sample-exact;
- a beat-matched mixer with an LR4 bass swap (`crates/player/src/automix/mixer.rs`);
- a pure-Rust Signalsmith stretch with a ratio that ramps (`stretch.rs`);
- per-end constant grids with stability, metre and Beat This! per end (`crates/player/src/types.rs:60-140`);
- novelty-based sections (`structure.rs`), a scored window search with a 15 s skip cap and a looped outgoing slice
  (`plan.rs`: `drop_aligned`, `MAX_SKIP_MS`, `out_loop_ms`);
- a synthetic evaluation suite (`eval.rs`).

The proposals below take BitChord's ideas where they are good, fix its weaknesses, and fit nori's rules (decisions in
Rust, zero cost when off, no allocation on the audio path).

### 7.1 One mix clock for every beat-matched transition

- Port the idea, not the code, of a **mix clock**: an array of beat times for the mix (in outgoing beats) and, per
  deck, one **position knot per beat**. Make it the single description the mixer reads for blends, ramps and loops
  alike. A plain blend is then the special case "both decks' knots advance at constant rates"; a loop mix is "knots
  wrap modulo a loop".
- nori's engine already renders both decks in one place, so it needs none of BitChord's splice, alignment or servo
  machinery (§3.5, §4.9). Every beat-matched mix in nori can be **sample-locked**, which BitChord's phone blends are
  not.
- Drive `stretch.rs` per block from the knots, as BitChord does: input samples per block = knot-interpolated heard
  position + latency terms − the read position. This replaces "hold ratio, ramp back to 1" with any tempo curve, and
  keeps the "a grid on the input holds on the output" invariant nori already documents.
- Test on the virtual clock: knot positions are heard to the sample at beat lines, seams only at loop ends, rate
  exactly 1 at release (BitChord's `LoopMixTest` invariants are the right list).

### 7.2 Warp to real beats, not to an even grid

- **Store the beats per end.** For the outgoing ~60 s and the incoming ~45 s, store each model beat as a signed
  offset from nori's constant per-end grid, in milliseconds as `i8` (±127 ms). That is about 1 byte per beat, under
  250 bytes per track, and it keeps the constant grid as the summary every planner rule already uses.
- **Use them as knots.** Each beat of a loop or blend maps to the *detected* beat in the source, so a live drummer's
  micro-timing inside four bars is followed and both decks hit the shared pulse together. BitChord only aligns at loop
  boundaries.
- **Gate on the warp.** If the offsets' spread over the chosen bars exceeds ~25 ms, or any beat is missing, the
  window is not loopable (the seam would expose the irregularity). This turns BitChord's one-off "¼ beat off"
  downbeat check into a per-beat stability measure on exactly the bars used.

### 7.3 Loop selection that listens to the seam

Keep BitChord's good filters:

- quietest bar ≥ 0.6 of the typical loop, not silent;
- ≤ 3 % sung frames and no sung run over 0.2 s, mask coverage ≥ 80 % including the beat before;
- synced-lyric spans padded **0.15 s / 0.35 s**, 0.3 s clear before, open lines certain for their first 2 s only;
- lengths 4 / 2 / 1 bars, cut between detected downbeats.

Add what it lacks:

1. **Seam continuity.** Compare the bar *after* the loop end (what the song would have played) with the loop's first
   bar: chroma correlation, onset-pattern correlation, low-band envelope. A loop whose next bar starts a new chord or a
   new section is a broken record waiting to happen. Score it; require a minimum.
2. **No fills.** Reject a loop whose last bar has far higher onset density or spectral novelty than its other bars (a
   drum fill or riser repeated is the most obvious loop artefact).
3. **Harmonic period.** Prefer a loop length equal to the chord-change period found by chroma self-similarity (a
   2-bar loop of a 4-bar progression flips chords at every seam).
4. **Section position.** Prefer loops that start on a section line from `structure.rs` (or on a 4-bar line from it),
   not on BitChord's 32-beat lattice.
5. **Metre.** Loops of `beats_per_bar` beats, so a waltz loops whole bars (nori already detects 3/4; BitChord
   cannot).
6. **Skip budget.** Charge the outgoing music after the loop end and the incoming intro before the loop start against
   `MAX_SKIP_MS`, with BitChord's "silence is free" rule. Prefer the outgoing loop **nearest the end** and the
   incoming loop **at its first downbeat**; a loop 40 s inside the song is then scored down instead of taken.
7. **Seam smoothing.** Crossfade the loop's last ~10 ms with the ~10 ms *before* its start (the sampler's
   loop-crossfade), so sustained notes and reverb tails do not jump. It is cheap, done on the stretch input, and
   costs nothing when the seam is already clean.

### 7.4 Key and pitch in loops

- Two loops overlap for up to 30 bars, so a loop mix needs a harmonic check that the blend planner already has:
  Camelot distance ≤ 1 (nori's `camelot_dist`), or at least one loop **chordless**. For the second, extend nori's
  `drop_runup_tonal_db` idea (tonal energy relative to the body, dB) to each candidate loop; a drum-only loop
  clashes with nothing.
- Count **relative major/minor as compatible** (BitChord's rule rejects it).
- Experiment (only if listening confirms): the stretcher keeps pitch and can also shift it. A ≤ 1 semitone shift of
  the incoming loop while both loops play, gliding back to 0 before release, could turn a Camelot distance of 2 into
  1. Gate it behind its own setting; measure that the glide is inaudible.

### 7.5 A better tempo curve

- Express the cap in **percent per bar** (≈ 2 %, DJ practice 1.3–2.9 %, `software.md` §B8), not BPM per bar, so slow
  and fast songs ramp equally gently.
- Shape the ramp as a **raised cosine**: zero slope at both ends, peak slope at the middle. A ramp of length *L* bars
  covering a log-tempo change Δ has peak slope πΔ / (2L). Pick *L* so the peak stays under the cap: about 1.57× the
  bars of a linear ramp of the same peak, but with no corner.
- **Split the travel between decks**: before the incoming loop is in, the outgoing loop may start bending toward the
  target (BitChord holds it 4 bars flat). Halving each deck's maximum stretch improves sound.
- Keep **one folding rule** for the whole planner (nearest in log ratio among ×½, ×1, ×2, with a tie-break toward
  the outgoing song's octave).
- Keep BitChord's **asymmetric stretch window** (speeding up is cleaner than slowing down) as a starting point and
  check it against nori's stretch on the eval set.

### 7.6 Integrate loops into the scored search, not as a pre-emptive rung

- nori's `drop_aligned` already scores windows (landing, run-up, phrase, skip, dead air, sung run-up, loop cost).
  Add the loop mix as a **third family** of candidates there: loop out, loop in, ramp length.
- Charge it what it really costs:
  - repetition (`LOOP_COST` per bar repeated, plus a stronger cost per bar of the *incoming* loop, which replaces the
    intro);
  - skip;
  - seam score;
  - length.

  Credit it for what it buys: a tempo gap beyond the blend's stretch limit.
- Expected outcome:
  - matched or near tempos (≤ ~6 %): real-music blends win;
  - 6–12 %: blends with a split stretch;
  - 12–40 %: loop ramps win when clean loops exist;
  - beyond that, or with no clean loops: filter, echo or a clean cut on the one.

  This is BitChord's capability without its "two short loops for every pair" habit.
- Keep BitChord's **unmixable** rule as a gate in nori's ladder: unreachable tempo plus a key clash or two singers
  gets a plain end fade into the intro. It is cheap and backed by listening.

### 7.7 The loop mix's sound

- **Bass**: use nori's LR4 high-pass exchange on a **downbeat** inside the ramp (a sixteenth before the line, as nori's
  `swap_len` already does), not a 250 Hz −18 dB shelf over a bar. Only one kick ever plays, the same rule BitChord
  applies to its own blends.
- **Incoming entry**: keep the low end cut until the swap, and add a gentle high-pass on the incoming loop's mids if
  either loop carries tonal content (BitChord's blend entry, 520 Hz shaped 0.35, is a good reference).
- **Gains**: equal-power or constant-loudness pairs, with each loop's short-term loudness (nori has BS.1770) matched;
  a headroom trim like BitChord's 1/√sum for blends. The loop renderer's 1 + 0.55 sum must not pass to the output
  unchecked.
- **Exit**: BitChord's rising high-pass drain (20 Hz → 1.5 kHz, exponential, 2 bars) is a good gesture. Use the
  4th-order filter nori has, and place the drain on a section or 4-bar line of the incoming song so the outgoing loop
  leaves where the music turns.

### 7.8 Length and variety

- Choose lengths from structure and energy, not from tempo gap alone:
  - calm into calm: long (BitChord doubles, up to 32 beats or 16 s, and found halving peaks "a regression");
  - peak into peak: a short swap or a clean cut on the one;
  - an outgoing breakdown: leave on it (nori's `Ending` already does).
- Add a **session memory**: the last few transitions' kind and length, and a small cost for repeating the same family
  or length (musicmix-dj's 0.15 per recent repeat, `software.md` §A6). BitChord deliberately keeps choices
  deterministic per pair; keep that per pair, but vary by history.
- Make the loop's entry and exit lengths variable: 2, 4 or 8 bars, chosen by the incoming section length and the
  outgoing loop's seam score. BitChord's fixed 4 + 2 makes every loop mix feel the same.

### 7.9 Vocals

- Calibrate nori's `VOCAL_MIN` (0.45 voice-band share) against BitChord's measured mask ranges (chorus 0.57–0.71, soft
  verse 0.2–0.4, break 0.02–0.15) on the same songs, using nori's lyrics sync as ground truth. BitChord's lesson is
  that the threshold must sit below *soft verses*.
- Use **instant-by-instant overlap** (BitChord's simultaneous fraction with a 5 % tolerance) rather than means
  wherever nori compares two vocal shares; nori's plan uses window means today (`intro_vocal`, `outro_vocal`,
  `drop_runup_vocal`).
- Loops must be judged on the **frame-level** vocal curve nori already computes in `vocal.rs` for lyrics sync. Store
  its values over the outgoing last 60 s and incoming first 45 s with an explicit "not measured" value, as BitChord's
  `vocalWindows` keep it apart from "not sung".
- Synced lyrics, when the provider has them, are the strongest evidence for "no word in this loop". nori's
  `nori-lyrics` already has line and word timing; reuse BitChord's padding rule.

### 7.10 Evaluation BitChord does not have

Extend `eval.rs` with **rendered-output metrics** per transition, measured on the mixed stream of synthetic and real
pairs:

1. **Beat agreement**: onset times of both decks in the overlap, from the per-deck streams before summing; median
   |Δ| < 5 ms and 95th percentile < 12 ms (BitChord's web build measured 2–5 ms on its loop decks; its phone blends
   are uncorrected).
2. **Double kick**: frames where both decks' sub-150 Hz energy is within 6 dB of each other. Should be under one beat
   per transition.
3. **Seam artefact**: spectral flux at each loop seam against the median flux of other beat lines; seam / median
   < 1.5.
4. **Loudness path**: short-term loudness through the transition; no bump over +1.5 dB or dip under −3 dB against the
   two songs' levels.
5. **Vocal overlap**: frames where both decks' vocal curves are over threshold, after gains and filters.
6. **Tempo smoothness**: the maximum change per bar of the master tempo (%), and that the rate is exactly 1 at
   release.
7. **Skip**: seconds of music never played, per side, under the cap.

Synthetic cases to add:

- a drummer with ±15 ms humanization and 0.5 % drift;
- a 4-bar progression with a fill in bar 4;
- a pad sustained over the bar line;
- a waltz;
- a song whose only instrumental bars are 45 s from the end.

Each one checks a rule above. Following nori's test rules, each metric must be able to fail on a real bug, e.g. a
loop cut on the fill, or a linear 1 + 0.55 gain.

### 7.11 Cost

- A loop mix runs two stretchers for 20–45 s per transition. On Android it must stay in burst rendering with no
  allocation on the audio path; the knot arrays are sized at plan time.
- Measure with `tools/perf-host.sh` against a plain blend.
- Analysis additions (beat offsets, per-bar seam features over the ends) reuse the end windows nori already decodes
  for Beat This!. The stored row grows by under 1 KB per track.
- Everything stays behind the existing AutoMix switch, with loop ramps as a quality tier of their own, so it costs
  nothing when off.
