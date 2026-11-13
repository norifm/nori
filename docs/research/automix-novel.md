# AutoMix: state of the art and what nori should build

Scope: nori's automatic DJ on phones, Rust, plans computed offline before each mix. The starting
point is the measurement on the owner's rock library (147 songs, reference beats from Beat This!
final0): a constant grid over a 30 s end window is right 19 of 73 times, the classical tracker makes
confident 2:1 and 3:2 errors, and grids fitted to the longest steady run (within 40 ms) are right 85 %
of the time but cover only 8 to 16 bars.

The short version: drop the constant grid as the model of a song end. Keep the beats themselves,
refine them below the network's 20 ms frame, smooth them into a per-beat tempo curve, and warp the
incoming song beat by beat onto the outgoing one under explicit limits on ratio and ratio change. That
is what DJ software converged on (rekordbox dynamic analysis, Traktor 4 elastic grids, djay 5 flexible
grids, Ableton warp markers), and nori can do it better than they do because it plans offline with
the whole overlap known in advance.

Status words used below: **build** (do it now), **later** (worth it after the build items), **no**
(not worth it on a phone, or not better than what nori has).

---

## 1. Beat-synchronous warping for live music

### What exists

- **DJ software.** rekordbox has "dynamic" analysis, a variable-tempo grid with markers at tempo
  changes; users report it still needs hand fixes. Traktor Pro 4 (2024) added "elastic" (flexible)
  beatgrids that follow each tempo change and keep sync working across them. Algoriddim djay Pro 5
  (Dec 2023) added flexible beat grids. Serato remains essentially a single fixed grid per track
  (variable grids appear only in some imported cases). Mixxx stores a beat map (a list of beat
  positions) but defaults to "assume constant tempo", and sync with non-constant grids has long-open
  bugs. Ableton Live's warp markers are the general model: any beat may be pinned to any time and the
  segments between markers are stretched linearly.
  - https://www.digitaldjtips.com/advanced-beatgridding-flexible-dynamic-elastic-how-to-do-it/
  - https://dj-kb.readthedocs.io/traktor_elastic_beatgrids/
  - https://www.native-instruments.com/products/traktor-pro
  - https://djtechtools.com/2023/12/06/algoriddims-djay-pro-5-0-is-here-hyper-fast-stem-separation-flexible-beat-grids
  - https://bugs.launchpad.net/mixxx/+bug/1114643, https://bugs.launchpad.net/mixxx/+bug/1851985
  - https://www.ableton.com/en/manual/audio-clips-tempo-and-warping/
- **Research.** Automatic DJ systems still fit fixed 4/4 grids: Vande Veire and De Bie (EURASIP 2018,
  drum and bass) and Mosaikbox (Sowula and Knees, ISMIR 2024), whose "precise beat-grid estimation"
  is explicitly a fixed grid "similar to popular DJ software". Nobody in the automatic-DJ literature
  publishes a per-beat warp for live music; the pieces exist separately (variable-tempo beat tracking,
  warp markers, robust smoothing).
  - https://asmp-eurasipjournals.springeropen.com/articles/10.1186/s13636-018-0134-8
  - https://repositum.tuwien.at/handle/20.500.12708/212628
- **How much DJs stretch.** Kim et al. (ISMIR 2020) aligned 1,557 real mixes (20,765 transitions):
  86.1 % of tracks were played within 5 % of their original tempo, 94.5 % within 10 %, 98.6 % within
  20 %. Transitions cluster at multiples of 32 beats. https://arxiv.org/abs/2008.10267
- **Perception.** Listeners detect tempo changes of roughly 1 to 3 % in isochronous sequences (Drake
  and Botte 1993), more for gradual drift, which sets the per-bar limit on ratio change below.

### The key fact about Beat This! timing

Beat This! (Foscarin, Schlüter, Widmer, ISMIR 2024) runs at 50 fps (hop 441 at 22.05 kHz), 30 s
chunks, and picks peaks as frame maxima within ±3 frames above 0.5. Its loss is **shift tolerant**:
predictions are max-pooled over 7 frames before the loss, so a peak anywhere within ±3 frames (±60 ms)
of the label is not penalized, and negatives within ±6 frames are ignored. The paper calls this its
most impactful design choice (F1 gains of about 1.4 points beat, 3.2 downbeat). Consequence for nori:
**the network is accurate about which beat, not exactly when.** Its peak position carries the 20 ms
frame quantization (±10 ms, about 6 ms RMS) plus an untrained offset. A 10 ms lock therefore needs a
refinement step against the audio's transients, and the final0 references used for the library
measurement have the same limit (fine for counting 82 ms errors, not for judging 10 ms ones).
https://arxiv.org/abs/2407.21658, https://github.com/CPJKU/beat_this

### Algorithm (build)

Inputs per transition: A's last 60 s and B's first 60 s (two small0 chunks per end; overlap the
chunks by about 6 s and keep the center of each, rather than concatenating non-overlapping chunks,
which loses beats at the seam).

**Step 1. Sub-frame refinement of every beat.**

```
onset(t): log-magnitude spectral flux, STFT 1024 window, hop 128 at 44.1 kHz (2.9 ms),
          two bands summed after per-band normalization: 40-150 Hz (kick, bass attack)
          and 1.5-8 kHz (snare, hats, pick attack)

for each network beat b (frame time):
    b1 = parabolic peak of the logit at frames (f-1, f, f+1)       # removes most quantization
    cands = local maxima of onset() in [b1 - 40 ms, b1 + 40 ms]
    pick c maximizing onset(c) * exp(-((c - b1) / 20 ms)^2 / 2)
    if onset(c) > median + 3 * MAD of onset() over the surrounding 2 s:
        beat = c, confidence = high
    else:
        beat = b1, confidence = low          # soft passage, no transient to lock to
offset = median(c - b1) over high-confidence beats   # per-end systematic lag
apply offset to the low-confidence beats
```

Cost: one extra STFT over 120 s per transition, a few ms of CPU. The classical onset tracker nori
already runs may provide `onset()` if its hop is 5 ms or less.

**Step 2. Clean the beat sequence (missed and extra beats).**

Use a variable-tempo dynamic program over candidate peaks (Ellis 2007, made local), not a constant
grid. Candidates are all activation peaks above 0.2 (lower than Beat This!'s 0.5, so weak true beats
are present to choose from).

```
P(t)  = local beat period: median IBI of the raw beats within ±8 beats (or PLP, below)
score(j) = s(c_j) + max over i < j of [ score(i) - alpha * log2((t_j - t_i) / P(t_j))^2 ]
           also allow t_j - t_i ≈ 2 P: insert a synthetic beat at the midpoint, extra cost beta
alpha = 400 (Ellis's "tightness" range; one 10 % IBI jump costs about 4 points)
beta  = 1.5 (a synthetic beat costs as much as a mid-confidence peak)
s(c)  = network logit, clipped to [-2, 4]
backtrack -> clean beats, each flagged detected / inserted
```

Extras (a peak between two beats) lose because both halves pay the log-ratio penalty; misses are
filled by the insertion move. Runs over 120 beats in microseconds.
Ellis: https://www.ee.columbia.edu/~dpwe/pubs/Ellis07-beattrack.pdf

Alternative with the same purpose: Predominant Local Pulse (Grosche and Müller 2011) on the network's
beat activation, 3 to 5 s Hann kernel, tempo set 60 to 240 BPM. PLP enforces local periodicity while
allowing tempo to drift, which is exactly the live-rock case; dPLP (Chiu, Strahl, Müller, ISMIR 2025)
is its differentiable form and confirms its role as the drift-tolerant replacement for the DBN. Pick
the DP; it gives explicit insert/delete decisions, which the warp needs.
https://www.audiolabs-erlangen.de/content/05_fau/professor/00_mueller/03_publications/2025_ChiuSM_dPLP_ISMIR_ePrint.pdf

**Step 3. A per-beat tempo curve instead of a constant grid.**

Fit the clean beat times t_k (index k) with a Whittaker smoother (Eilers 2003): minimize

```
sum_k w_k (t_k - y_k)^2  +  lambda * sum_k (y_{k+1} - 2 y_k + y_{k-1})^2
```

The second difference of beat times is the change of IBI, so lambda moves continuously from "every
beat as detected" (0) to "constant tempo" (infinity, nori's current grid). It is a pentadiagonal
system, O(N).

```
w_k = 1 for high-confidence beats, 0.3 low-confidence, 0.05 inserted
lambda: largest value on a log grid 1e-2 .. 1e6 (bisect on log lambda) such that, after 3 IRLS
        passes with Tukey bisquare weights (c = 25 ms):
          weighted 90th percentile |t_k - y_k| <= 6 ms, and
          max |t_k - y_k| over non-outliers <= 15 ms
outlier: a beat with |t_k - y_k| > 25 ms after the last pass (a push, a fill, a misplaced peak)
```

Outputs per end: smoothed beats y_k, local IBI_k = y_{k+1} - y_k, the residual per beat, outlier
flags, and steadiness as the largest IBI change over any 4 bars. The library finding (constant grid
right 19/73, steady runs of 8 to 16 bars) suggests that most of the "unsteadiness" is slow drift, which
this model absorbs, so lockable runs should grow to most of the 60 s. That is a hypothesis to measure
first (see the harness in Section 2).

Ending detection from the same curve: a **ritardando** is a smoothed tempo falling more than 3 % over
the last 4 bars; a **fade** is a steady negative loudness slope; a **ring-out** is a final hit then
decaying energy without beats. The overlap must end before any of them, or A is cut, echoed out or
braked instead of beatmatched.

**Step 4. The warp: B's beats onto A's, per beat.**

A plays at its natural rate (A is the master deck, like DJ sync). The planner fixes the overlap: A's
beat s aligned with B's beat 0, L beats long (16, 32 or 64, on downbeats and phrase starts), and a
level mapping m ∈ {1, 2, 1/2} (half/double-time mixing, Section 2).

Variables: u_j, the output time of B's beat j. Known: B's source beat times β_j (refined, Step 1) and
A's beat output times α_j = a_{s + m j}. Let q_j = (u_{j+1} - u_j) / (β_{j+1} - β_j), the inverse of the
stretch rate on beat segment j; q is linear in u, so the problem is a small convex QP.

```
minimize   c1 * sum_j (q_j - q_{j-1})^2                       # few ratio changes
         + c2 * sum_j (q_{j+1} - 2 q_j + q_{j-1})^2           # changes as glides, not steps
         + mu * sum_j conf_j * (u_j - alpha_j)^2              # pull toward exact lock
subject to |u_j - alpha_j| <= E_j                             # the lock
           |q_j - q_{j-1}| <= D                               # ratio change per beat
           |q_j - 1| <= R                                     # total stretch
           u_0 = alpha_0

E_j = 10 ms when both beats are high-confidence, 20 ms otherwise
D   = 0.004 (0.4 % per beat, 1.6 % per 4/4 bar; at the edge of the tempo JND)
R   = 0.06 preferred, 0.08 hard (Kim 2020: 86 % of DJ plays are within 5 %)
c1 = 1, c2 = 4, mu = 1e-3 per ms^2 (units: q dimensionless, u in ms)
```

Size: L ≤ 128 variables, about 3L constraints. Solve with a few hundred ADMM iterations on a banded
Cholesky of (P + rho AᵀA), which is banded because every term couples neighbors only (well under 1 ms),
or with Clarabel.rs, a pure-Rust interior-point QP solver (Apache-2.0) if a dependency is acceptable.
https://github.com/oxfordcontrol/Clarabel.rs, https://osqp.org (ADMM reference)

Before and after the overlap:

```
lead-in (B audible before beat 0, at low gain): q = q_0
after the overlap: q glides back to 1 at most G = 0.25 % per beat (1 % per bar), or holds if the
                   next transition starts first
```

Feasibility ladder when the QP has no solution: widen E to 25 ms for low-confidence beats only; else
shorten L to the longest feasible phrase-aligned sub-window; else plan a non-beatmatched transition
(echo-out, filter sweep, cut on A's last downbeat). Never ship a beatmatched overlap that misses the
tube.

**Step 5. Shared clock when A misbehaves (optional mode).**

If A's smoothed tempo changes more than 1.5 % per bar inside the overlap, locking B to A makes B wobble
too. Then warp both onto a master beat clock whose log IBI moves from A's to B's with the crossfade:

```
log m_j = (1 - g_j) * log IBI_A(s + m j) + g_j * log IBI_B(j),   g = smoothstep over the overlap
solve the Step 4 QP twice, A onto the master and B onto the master, each with E_j / 2
```

This reuses nori's outgoing tempo glide; it trades a little stretch on A for a steady combined pulse.

**Step 6. Use the warp cost to choose the overlap.** For each candidate pair (A's phrase start s, B's
entry phrase, level m), solve the QP and score: objective value, mean |log r|, the share of beats at
the 20 ms tolerance, vocal overlap (Section 4) and phrase fit (Section 3). Pick the lowest. Each solve
is sub-millisecond, so tens of candidates cost nothing offline.

**Step 7. Hand it to the engine.** Emit piecewise-constant rate per beat segment, r_j = 1/q_j, with
symmetric 30 ms ramps centered on each beat so the area (source time consumed) is unchanged; or emit
warp-marker pairs (β_j, u_j) and let the engine interpolate linearly between them, which is the same
thing. Verify the plan in Rust: integrate the rate curve and assert that every B beat lands within
E_j of its A beat. That assertion is the unit test.

### Is DTW the right tool?

Not for the cross-song map. After cleaning, the correspondence between A's and B's beats is
index-to-index by construction (the planner chooses s and m), so the only freedom is timing, which the
QP handles with explicit limits. DTW belongs where two analyses of the same audio must be matched:
small0 versus the classical tracker, small0 versus final0 in the evaluation harness, or the same song
analyzed twice. There, use DTW on beat times with a step cost of |Δt| and skip costs for insertions and
deletions (Kim et al. 2020 used subsequence DTW on beat-synchronous features for mix-to-track
alignment). Soft-DTW is only needed for training.

### Verdict

**Build.** Steps 1 to 4 and 6 are the single biggest improvement available. They replace the model
that is wrong 54 of 73 times with one that matches what DJ software now does, use only data nori
already computes plus one STFT, and cost milliseconds per transition. Step 5 can follow once Step 4 is
measured.

---

## 2. Metrical level and tempo-octave errors

### What is state of the art

- **Beat This!** has no DBN, so it can switch level within a song; on GTZAN: final model beat F1 89.1,
  CMLt 79.8, AMLt 89.8; small (about 2M parameters) 88.8 / 79.4 / 89.0; downbeat F1 77.2 for small. The
  10-point gap between CMLt and AMLt is roughly the share of tracks at the wrong level or phase. The
  paper notes AMLt assumes constant tempo and meter and that the DBN "cannot easily switch to another
  metrical level", which is also why it hurts drifting music. https://arxiv.org/abs/2407.21658
- **Masked diffusion beat tracking** (Foscarin, Korzeniowski, Vogl, ISMIR 2026): about 25M parameters on
  the Beat This! architecture, 8 iterative steps, no DBN. GTZAN beat F1 89.7, CMLt 82.9, AMLt 92.5;
  downbeat CMLt 76.4 (Beat This! 67.3), AMLt 88.5 (79.1). Tempo doubling/halving events fall from
  0.75 to 0.12 per track and consecutive downbeats from 0.25 to 0.02. It names the cause directly:
  single-pass networks output a mixture of competing grid interpretations.
  https://arxiv.org/abs/2608.04624, https://github.com/fosfrancesco/md_beat_this
- **Multi-task tempo and beat** (Böck, Davies, Knees, ISMIR 2019; Böck and Davies, ISMIR 2020): a
  tempo head trained jointly with beats improves both; tempo augmentation widens the tempo range seen.
  https://program.ismir2020.net/poster_4-14.html
- **Preferred tempo prior.** Pulse salience peaks near 120 BPM (van Noorden and Moelants 1999,
  "Resonance in the perception of musical pulse", https://doi.org/10.1076/jnmr.28.1.43.3122). Used as a
  log-Gaussian prior it resolves many octave errors but not 3:2 ones.
- **Human ambiguity.** GlobalTap (ISMIR 2026) crowdsources tapping across cultures; tapping studies show
  that for many songs two levels are both "correct", so the right question for a DJ is not "the true
  level" but "a level consistent with the bar and with the other song".
  https://ismir2026.ismir.net/accepted-papers

### What nori should do (build)

Make the network the authority on level and the classical tracker a witness, then arbitrate the few
disagreements with checks that hold for rock.

```
candidates: period P from small0's clean beats (Section 1, Step 2) times k in {1/2, 2/3, 1, 3/2, 2},
            kept if 55 <= BPM <= 215; plus the classical tracker's period if not already present

features per candidate c (each normalized to roughly [-1, 1]):
  net      agreement of c's grid with small0's beats (share of small0 beats within 40 ms of c's beats,
           and of c's beats near a small0 beat)                                  weight 2.0
  bar      median downbeat spacing in c's beats, n = round(Δdownbeat / P_c):
           n = 4 -> +1, n = 3 -> +0.6, n = 2 or 8 -> -1 (level off by 2), n = 6 -> -0.5 (3:2 or 6/8),
           and require >= 80 % of downbeats within 30 ms of one of c's beats       weight 1.5
  backbeat rock check: per beat, kick-band flux K_k (40-120 Hz) and snare-band flux S_k
           (180-400 Hz plus 2-8 kHz), both in ±30 ms windows; also S'_k at half-beat points.
           c is right if S alternates with period 2 (|corr(S_k, (-1)^k)| high) and S'_k is weak;
           S'_k > 1.5 S_k consistently -> c is half-time; snare every 4th beat and nothing at
           period 2 -> c is double-time                                             weight 1.0
  tatum    from a Fourier tempogram of onset(): the strongest subdivision tau_t in [c, 4c]
           must be c * {1, 2, 3, 4} within 3 %; a ratio of 1.5 or 0.75 means a 3:2 error
                                                                                    weight 1.0
  prior    -(log2(BPM_c / 120))^2 / (2 * 0.45^2)                                   weight 0.5
  ends     both ends of the song pick the same level, or levels whose ratio matches the tempo
           drift measured between them (joint choice over the pair)                weight 1.0
  tag      OpenSubsonic bpm tag, when present, agrees within 4 % at this level     weight 0.5

choose argmax of the weighted sum; report the margin to the runner-up as level confidence
```

Notes:

- Ignore the classical tracker's own confidence for the level decision. The library shows it is
  confident while wrong; its period is useful only as a candidate.
- Start with the weights above, then fit them by logistic regression on the 147-song library (final0
  level as the label, with hand checks where final0 and the ear disagree) plus Ballroom, GTZAN and
  Harmonix references; keep the library as the test set.
- The DP in Section 1 already penalizes local level switches (a factor-2 IBI jump costs about
  alpha = 400 points); keep it, because Beat This! has no global meter model.
- **Level between songs.** If T_B / T_A is outside [0.92, 1.08] but 2 T_B / T_A or T_B / (2 T_A) is
  inside, mix at m = 2 or 1/2 (half or double time); one bar of the slower song equals two bars of the
  faster one, so phrase alignment uses that. Never mix 3:2 automatically.

### Which evaluation numbers matter for mixing

| metric | what it rewards | for DJ mixing |
|---|---|---|
| F-measure, ±70 ms | each beat near some reference beat | Too lax: 70 ms is seven times the 10 ms lock nori needs, and it ignores continuity, so an output with a level switch every few bars still scores well. |
| CMLt (17.5 % of IBI, about ±87 ms at 120 BPM) | correct level and phase, longest continuity | The right shape (correct level and phase over time), wrong tolerance. |
| AMLt | also accepts double, half and **offbeat** | Overstates usefulness. Double and half are fine if consistent; offbeat (kick on the snare) is the worst possible mix. |
| Information Gain | distribution of errors | Diagnostics only. |

Definitions: mir_eval (https://craffel.github.io/mir_eval/#module-mir_eval.beat).

nori should report four numbers per end, in a Rust test harness over the library:

1. **Level accuracy:** period within 4 % of the reference at the same level (no octave folding).
2. **Lock@10 and Lock@20:** the share of 16-beat windows in which every beat is within 10 (or 20) ms
   of the reference after the Section 1 model. This is CMLt with an absolute, DJ-sized tolerance.
3. **Downbeat phase:** the share of bars whose downbeat matches the reference.
4. **Mix error, end to end:** build the Section 1 warp from estimated beats, then measure where the
   *reference* beats of A and B land in output time over the overlap; report max and RMS offset.
   Under 20 ms max reads as tight; 20 to 40 ms as a flam; above that as a train wreck. This is the
   number that predicts what the owner hears.

Make the reference sharper first: run Step 1's refinement on final0's beats too, and hand-tap 20 songs
with snap to onsets as a gold set, since final0 shares the shift-tolerant timing limit.

### Verdict

**Build** the arbitration and the harness. **No** for running masked diffusion on the phone (25M
parameters times 8 steps is about 100 times small0's cost); **later**, use it on the host as a better
reference than final0 for level and downbeat, and as a teacher if a small model is ever fine-tuned.

---

## 3. Structure and phrases at the song ends

### What is state of the art

| method | boundary quality | cost | phone |
|---|---|---|---|
| Foote checkerboard novelty on a self-similarity matrix; MSAF's SF, scluster, OLDA (https://github.com/urinieto/msaf) | Harmonix HR.5F about 0.5 for the best unsupervised (DSF+scluster 0.497) | trivial on bar-synchronous features | yes |
| CNN boundary detectors (Ullrich, Schlüter, Grill 2014-15) | supervised, better than unsupervised at 0.5 s | small CNN | yes |
| All-In-One (Kim and Nam, WASPAA 2023), beats, downbeats, boundaries and labels jointly | Harmonix HR.5F 0.660, label PWF 0.738 with **300K parameters**; the 46K "small" reaches 0.616 | the model is tiny; its input needs Hybrid Transformer Demucs stems per song | the network yes, the separation no |
| TCN without demixed input (same paper, reproduced) | HR.5F 0.594 with 74K parameters | small TCN on the mel spectrogram | **yes** |
| SongFormer (2025), SSL backbones | RWC-Pop HR.5F 0.65, label accuracy 0.81; 2 to 4 s per song on an L40 GPU; weak on EDM | hundreds of millions of parameters | no |
| ISMIR 2026: stem-specialized mixture-of-experts for joint structure analysis | needs stems | | no |

Sources: https://arxiv.org/abs/2307.16425, https://github.com/mir-aidj/all-in-one,
https://arxiv.org/abs/2510.02797, https://transactions.ismir.net/articles/10.5334/tismir.54 (survey).

The useful finding in All-In-One's own table: demixing is worth about 0.03 to 0.07 HR.5F, and the cost
of the network itself is negligible. A no-demix model of 74K to 300K parameters trained on Harmonix
gets about 0.6 HR.5F, close to the 0.66 of the full system, at a phone-trivial cost.

**Cue points.**
- CUE-DETR (Argüello, Lanzendörfer, Wattenhofer, ISMIR 2024): DETR with a ResNet-50 backbone (about
  41M parameters) fine-tuned on mel images of about 11 s; EDM-CUE has 4,710 tracks and about 21k cues
  from four DJs (metadata only, no audio). At a one-beat tolerance with a 16-bar minimum spacing:
  precision 0.62, recall 0.27, F1 0.38 on 16-bar phrase cues; F1 0.46 with an 8-bar radius. Mixed In
  Key scores 0.16 and Zehren's Automix 0.15 on the same task. Cues align best with 16-bar phrases.
  https://arxiv.org/abs/2407.06823, https://github.com/ETH-DISCO/cue-detr
- Zehren, Alunno, Bientinesi (CMJ 2022): rule-based (from DJ interviews) and learned cue detectors from
  novelty features; about 90 % of generated points judged usable on unseen EDM tracks.
  https://arxiv.org/abs/2007.08411
- Human agreement is low: in Kim 2020, two DJs' cue points on the same track coincide in 23.6 % of
  cases, are within one bar in 40.4 %, within 8 bars in 73.6 %. A cue point is a fuzzy target; a rule
  that lands on a plausible phrase start is as good as a model.

### What nori should do

**Build: bar-synchronous phrase segmentation with a phrase-length prior.** nori already computes per-bar
loudness, low band, pulse, voice share and novelty ("change") at each end. Add a dynamic program that
places phrase starts:

```
bars b = 0..N-1 from the Section 1 beat map and the chosen downbeat phase
evidence(b) = z(novelty_b) + 0.7 * z(|Δ loudness_b|) + 0.7 * z(|Δ low band_b|)
              + 1.0 * [vocal onset or offset at b] + 0.5 * z(repetition start_b)
repetition start: lag features on bar-synchronous chroma and MFCC (Serrà et al. structural
              features); high when bar b begins a block that repeats an earlier block
segment length prior (log): 8 bars 0, 16 bars -0.3, 4 bars -0.5, 12 bars -1.5,
                            6 bars -2.5, 2 bars -3.0, other -5
Viterbi over (b, length): best[b] = max_L best[b - L] + logprior(L) + kappa * evidence(b), kappa = 1
output: phrase starts with their evidence; cue-in / cue-out candidates are phrase starts
```

The prior encodes the 8/16-bar regularity DJs rely on (Kim 2020's 32-beat peaks, CUE-DETR's 16-bar
finding); the "other" lengths let rock's 6-bar and 2-bar pickups and extended bars through when the
evidence is strong. Run it over the whole 60 s window at each end, not 30 s; phrases of 16 bars at
120 BPM last 32 s.

Rock cue rules on top (they replace a learned cue model):
- cue-out: the start of the last repeat of the chorus, or the first phrase of the outro, provided the
  overlap ends before any ritardando, fade or ring-out (Section 1, Step 3);
- cue-in: the first phrase start where the drums are in (low band and pulse up) and before the first
  vocal onset; otherwise a phrase start inside the intro;
- prefer overlaps where at most one song has vocals (Section 4).

**Later: a small boundary head.** Train an All-In-One-style no-demix model (about 74K to 300K
parameters, mel input at 50 or 100 fps, dilated convolutions or neighborhood attention) on Harmonix
with 8-fold cross-validation, or a boundary and function head on top of small0's trunk (the same mel
front end, so no extra spectrogram). Expect HR.5F near 0.6 on Harmonix; the phrase DP then uses its
boundary probability as `evidence`. Value: modest over the DP alone, because the planner only needs
two good candidates per end, not a full segmentation.

**No:** CUE-DETR on the phone (a 41M-parameter detector over many 11 s windows, and trained only on EDM,
which differs from rock in exactly the regularity it learns); SongFormer and other SSL-backbone
analyzers; any structure method that needs separation per song.

---

## 4. Singing-voice activity on the device

### What is state of the art

- Best reported frame accuracy on the public Jamendo corpus sits around 90 to 92 %: Schlüter and Grill
  (ISMIR 2015), a CNN with pitch-shift, time-stretch and filter augmentation; recurrent models at
  91.5 %; an LRCN with vocal separation at 92.4 %. Lehner et al.'s random forest on hand-made features
  (MFCC, vocal variance, fluctogram) reached about 88 %, and MFCC-only light systems about 79 %.
  https://arxiv.org/abs/1806.01180 (review), https://www.ismir2015.uma.es/articles/264_Paper.pdf,
  https://research.jku.at/en/publications/towards-light-weight-real-time-capable-singing-voice-detection/
- Knowledge distillation (2020): students up to 1000 times smaller than the teacher reach comparable
  accuracy on Jamendo. https://arxiv.org/abs/2011.04297
- No 2023 to 2026 paper changes this picture for lightweight detection. Recent work moved to
  separation; vocal detection by thresholding a separated vocal stem is accurate but pays for the
  separator: Open-Unmix (LSTM, about 9M parameters per target), Hybrid Transformer Demucs (about 40M),
  DTTNet (ICASSP 2024; vocals cSDR 10.12 dB with 86.7 % fewer parameters than BSRNN). That is one to two
  orders of magnitude more compute than a detector, for a gain of a point or two in detection.
  https://arxiv.org/abs/2309.08684, https://github.com/sigsep/open-unmix-pytorch,
  https://github.com/facebookresearch/demucs
- General audio taggers distilled for phones (EfficientAT MobileNetV3, mn10 at 4.88M parameters, smaller
  variants exist) include AudioSet "Singing" and "Vocal music" classes; untested for frame-level
  singing detection in mixtures, about 1 s resolution. https://github.com/fschmid56/EfficientAT
- Jamendo is small (93 songs) and numbers vary by split, so treat cross-paper comparisons as rough.

### What nori should do

**Build: a vocal-activity head that reuses small0's mel.** nori already computes small0's input (128
mel bands, 50 fps) at each end. Two options, try the first:

1. **Head on small0's trunk.** Freeze the Beat This! small trunk, add a linear or one-layer
   temporal-conv head that predicts vocal presence per frame. Extra inference cost is close to zero.
   Risk: a beat-trained trunk may not encode voice well; check it with a linear probe before anything
   else (an afternoon on the host).
2. **Tiny separate CNN on the same mel** (Schlüter-style, distilled): 1 s context (50 frames), conv
   3×3 with 16-16-pool-32-32-pool-64 channels, dense 64, sigmoid; about 60 to 100K parameters,
   evaluated every 0.1 s. Roughly a few GFLOP per 60 s end, a fraction of a second on one phone core.

Training labels without hand annotation: MUSDB18-HQ vocal stems (vocal RMS above -40 dB relative to the
mix gives the label, 150 songs), DALI note-level vocal annotations (5,358 songs, audio via YouTube),
the Jamendo corpus (93 songs, Creative Commons audio, the standard test), and MedleyDB's vocal
activations. Augment with pitch shift, time stretch and filters (Schlüter and Grill). Postprocess with
a two-state HMM (switch probability 0.01 per 0.1 s step) and report per bar, which is what the planner
uses (`voice share` per bar already exists; this replaces its estimate).

Expected quality: about 88 to 91 % frame accuracy on Jamendo, higher per bar. For the planner the
failure that matters is a missed vocal entry in B's intro or a missed vocal tail in A's outro; tune the
threshold for recall there.

**No:** separation only to detect vocals. **Later:** separation for stems (Section 5), and if it lands,
take vocal activity from the stem for free.

---

## 5. Novel and emerging ideas, judged for a phone

| idea | source | what it gives | phone cost | verdict |
|---|---|---|---|---|
| Per-beat warping with ratio limits (Section 1) | DJ software practice; not published for automatic DJs | locked beats on drifting live music | milliseconds | **build** |
| Drift-tolerant periodicity: PLP and dPLP | Grosche and Müller 2011; Chiu et al., ISMIR 2025 | cleans network beats without a constant-tempo assumption | trivial | **build** (or the DP in Section 1) |
| Masked-diffusion beat tracking | Foscarin et al., ISMIR 2026, arXiv 2608.04624 | coherent grids, far fewer level switches (doubling/halving 0.75 to 0.12 per track) | about 25M parameters times 8 steps | **later**, on the host as reference and teacher |
| Online trackers (BEAST, BeatNet, Beat This! Live, ISMIR 2026) | arXiv 2312.17156, 2108.03576 | real-time tracking | | **no**: nori plans offline |
| Stem-aware transitions, rule based | Mosaikbox (Sowula and Knees, ISMIR 2024): removing stems by rule "significantly enhances the perceived quality of a mix" | vocal swap, drum carry, bass swap without masking | separation of the overlap only | **later**, behind a setting |
| Stem-aware transitions, optimized | MixWeaver (ISMIR 2026): stem-wise gain trajectories over the intro/outro overlap, interpretable penalties, genetic search; reviewers note it beats only a rule-based baseline | the same with search | separation plus a cheap search | **later**, as the planner for stems |
| Sequential stem blending (flow matching) | Yeh, Chan, Hung, Yang, ISMIR 2026, arXiv 2608.05506 | mixing stems into a submix, not song-to-song transitions | large generative model | **no** |
| Learned transition control curves | DJtransGAN (Chen et al., ICASSP 2022, arXiv 2110.06525); a 2025 Princeton thesis predicting fader and EQ curves from 1001Tracklists mixes | fader and EQ curves that resemble human DJs | small models, but EDM-trained | **no** for now: use Kim 2020's statistics as rules |
| Generative bridges (inpainting between songs) | Levy et al. 2023 diffusion guidance (arXiv 2311.00613); SoundMorpher (arXiv 2410.02144); Stable Audio Open Small (341M, about 11 s of audio in under 8 s on an Arm phone) | a newly generated bridge | seconds of full CPU per bridge, a large model download | **no**: unproven quality between two specific recordings, battery cost, and it changes the music the user chose |
| Mixability embeddings (MERT, CLAP, OpenL3) | DJtransGAN's mixability score; Barthet et al. (JAES 2024, OpenL3 distances for transition smoothness) | which songs blend, which transition style | MERT-95M on 10 s is seconds of CPU per song | **no** on device; nori mixes the user's queue order, so mixability only chooses the transition style, which rules already do |

Details worth knowing:

- **Stems on the phone are real but costly.** djay runs stem separation live on iPhones (Neural Engine).
  For nori (CPU, Rust, battery first) the workable form is: separate only the coming overlap (about 2 ×
  20 s), once, while the previous song plays, with a lite model (DTTNet-class) through an inference
  runtime, and only when a setting is on. Measure CPU seconds per transition with
  `tools/perf-host.sh` before committing to it. The first rules to apply (Mosaikbox style): never two
  lead vocals at once; carry B's drums under A's vocals at the end; swap bass on a downbeat.
- **The cheap version of stems already exists in nori's engine:** a low-shelf bass swap on a downbeat
  and a band-limited crossfade. Do these with the Section 1 lock first; much of the benefit of stems
  in published systems comes from avoiding bass and vocal clashes, which these address.
- **Style choice for rock.** Long beatmatched blends are an EDM idiom. For rock, choose by measurement:
  a long blend only when both ends have a steady curve (Section 1), low vocal activity, and a phrase
  boundary; otherwise a short blend over 1 to 2 bars on the downbeat, an echo-out on A's last downbeat,
  or a clean cut after a ring-out. The QP cost (Section 1, Step 6) is a natural "blendability" score.

---

## 6. Public labeled datasets

Most audio is not redistributable; plan on annotations plus your own audio fetch, or on the
preprocessed spectrograms that Beat This! publishes (same mel front end as small0, so usable for the
network-side work without audio; the classical tracker and the Section 1 refinement need audio).

| dataset | labels | size | audio | license / access |
|---|---|---|---|---|
| Harmonix Set (Nieto et al., ISMIR 2019) | beats, downbeats, sections with labels | 912 Western pop/rock/EDM tracks | YouTube URLs with DTW alignment scores; mel spectrograms on request | annotations MIT; spectrograms under an agreement. https://github.com/urinieto/harmonixset |
| Beat This! training sets (preprocessed) | beats, downbeats | 4,556 tracks over 18 sets (Ballroom, Hainsworth, SMC, HJDB, Beatles, Harmonix, RWC, TapCorrect, JAAH, Filosax, ASAP, Groove MIDI, GuitarSet, Candombe, Simac); GTZAN as test | spectrograms released with the code | per source dataset. https://github.com/CPJKU/beat_this |
| GTZAN with rhythm annotations | beats, downbeats | 1,000 × 30 s, 10 genres incl. rock | widely mirrored | research use, unclear copyright; annotations by Marchand and Peeters (via mirdata) |
| Ballroom | tempo, beats, downbeats (Krebs et al. 2013) | 698 × 30 s | ISMIR 2004 contest download | research. https://github.com/CPJKU/BallroomAnnotations |
| Beatles (Isophonics; beats by Davies et al.) | beats, downbeats, sections, chords | 180 songs | your own CDs | annotations free. http://isophonics.net/datasets — the best public proxy for live-played rock without a click |
| SALAMI | sections, two annotators, several levels | 1,359 tracks, 2,200+ annotations | part from the Internet Archive (Live Music Archive concert recordings, fetchable with Nieto's script); rest commercial | annotations CC0. https://ddmal.music.mcgill.ca/research/salami/annotation — its live recordings are a good structure test for drifting rock |
| RWC Popular / Royalty-Free | beats and sections (AIST); vocal activity from later third-party annotations | 100 + 15 songs | purchased discs | research. https://staff.aist.go.jp/m.goto/RWC-MDB/ |
| Jamendo corpus (Ramona et al. 2008) | frame-level singing voice | 93 songs (61/16/16 split) | Creative Commons audio | the standard SVD test set (via the authors and the review at https://arxiv.org/abs/1806.01180) |
| MUSDB18 / MUSDB18-HQ | stems (vocal activity derivable) | 150 tracks (about 10 h) | included | academic use only. https://sigsep.github.io/datasets/musdb.html |
| DALI | vocal notes and lyrics aligned in time | 5,358 songs | YouTube | annotations open (academic). https://github.com/gabolsgabs/DALI |
| MedleyDB 1/2 | stems, instrument and vocal activations | 122 / 196 multitracks | on request | CC BY-NC-SA. https://medleydb.weebly.com/ |
| EDM-CUE (CUE-DETR) | DJ cue points, metronome info | 4,710 tracks, about 21k cues, 4 DJs | none (references only) | code MIT; data via the repository. https://github.com/ETH-DISCO/cue-detr |
| M-DJCUE (Zehren et al.) | DJ cue points | small EDM set | references | via https://arxiv.org/abs/2007.08411 |
| Kim et al. DJ mixes | mix-to-track alignments, cue points, tempo changes | 1,557 mixes, 13,728 tracks, 20,765 transitions | YouTube / 1001Tracklists | code and statistics, https://arxiv.org/abs/2008.10267 (mir-aidj on GitHub) |
| GiantSteps Tempo | global tempo | 664 EDM excerpts | Beatport previews | research. https://github.com/GiantSteps/giantsteps-tempo-dataset |
| GlobalTap (ISMIR 2026) | crowdsourced taps, globally diverse | | | for metrical ambiguity; https://ismir2026.ismir.net/accepted-papers |
| mirdata | loaders, checksums, licenses for most of the above | | | https://mirdata.readthedocs.io/ |

For nori specifically, the most valuable ground truth is the owner's own library: the 147 songs with
final0 beats, sharpened with the Section 1 refinement and a 20-song hand-tapped gold set. Use Beatles,
SALAMI's live recordings and Harmonix to check that tuning on the library does not overfit.

---

## Ranked build list

1. **Evaluation harness in Rust** over the library: level accuracy, Lock@10/Lock@20, downbeat phase,
   end-to-end mix error (Section 2). Refine final0's beats with Step 1 and hand-tap a 20-song gold set.
   Everything below is measured against it.
2. **Analyze 60 s per end** with two overlapping small0 chunks (Section 1 inputs). The tempo curve and
   the phrase DP both need more than 30 s.
3. **Sub-frame beat refinement** against a 3 ms onset function (Section 1, Step 1). Required for a
   10 ms lock because Beat This! is trained shift tolerant at 20 ms frames.
4. **Beat cleaning DP and the Whittaker tempo curve** in place of the constant grid (Steps 2 and 3),
   with ending-type detection (ritardando, fade, ring-out).
5. **The per-beat warp QP** with E = 10 ms, D = 0.4 %/beat, R = 6 to 8 %, the feasibility ladder, and
   overlap selection by warp cost (Steps 4, 6, 7). Unit test: every B beat lands within its tube.
6. **Metrical-level arbitration**: network first, downbeat spacing, backbeat test, tatum test,
   120 BPM prior, both-ends agreement; the classical tracker's confidence no longer decides level.
   Half/double-time mixing via m ∈ {1/2, 2}.
7. **Phrase DP with the 4/8/16-bar prior** and the rock cue rules (Section 3).
8. **Vocal-activity head** on small0's mel (probe the trunk first, else the tiny CNN), trained on
   MUSDB18-HQ, DALI and Jamendo; feeds the planner's "one lead vocal at a time" rule.
9. **Shared-clock warping** for an outgoing song that drifts within the overlap (Step 5).
10. **Later:** a small no-demix boundary head (All-In-One-style, Harmonix); masked-diffusion tracking
    on the host as a better reference and teacher; stems for the coming overlap only, behind a setting
    and measured on the host first, with Mosaikbox/MixWeaver-style rules.
11. **Not for nori:** generative bridges, CUE-DETR or SongFormer on the device, MERT/CLAP mixability on
    the device, online beat trackers, per-song separation for detection only.
