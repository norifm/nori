# Academic research on automatic DJ transitions: what nori's planner can use

Internal research note, written 2026-11-13. It builds on `automix.md`, `automix-next.md` and `analysis.md` and does not repeat them. A second note covers open-source code and products; this one covers papers, plus the code a paper's authors released when that code holds numbers the paper leaves out.

Labels: **[paper]** the paper states it; **[code]** the authors' released code for that paper; **[derived]** my arithmetic on a paper's numbers; **[inferred]** my reading, not stated by anyone. Every paper below was read in full from its PDF unless it is marked **[abstract only]**.

The question behind this note: on a rock/alt playlist, 85 % of nori's transitions cannot be beat-matched (tempo gaps of 21 to 41 %, drummers who drift), so they all became plain fades. What does research say a planner should do instead, in DJ style and in Smooth style?

---

## 1. Papers, one by one

### 1.1 Kim, Choi, Sacks, Yang, Nam: "A computational analysis of real-world DJ mixes using mix-to-track subsequence alignment" (ISMIR 2020)
[arXiv 2008.10267](https://arxiv.org/abs/2008.10267), companion site [mir-aidj.github.io/djmix-analysis](https://mir-aidj.github.io/djmix-analysis/)

The largest measurement of what real DJs do. 1,557 mixes from 1001Tracklists, 13,728 tracks, 20,765 transitions, mostly house and trance, average tempo 127 BPM **[paper]**.

- **Tempo change:** 86.1 % of tracks are played less than 5 % off their own tempo, 94.5 % under 10 %, 98.6 % under 20 % **[paper]**. The distribution is a sharp double exponential around 0. Real DJs almost never stretch a track by more than 10 %.
- **Key change:** only 2.5 % of 24,202 played tracks were transposed, and 94.3 % of those by one semitone **[paper]**. DJs leave key lock on and do not pitch-shift.
- **Transition length:** the histogram of transition lengths (cue-out of A to cue-in of B) peaks at every 32 beats (8 bars) **[paper]**. Read off the figure **[derived]**: the two tallest peaks are at 32 and 64 beats (8 and 16 bars, about 15 and 30 s at 127 BPM); the bulk lies between 8 and 96 beats; the tail runs past 224 beats (over 100 s). Note that the cue-detection method needs 32 diagonal beats on each side, so very short transitions are under-counted.
- **Where the boundary is heard:** human annotators place the track boundary closest to the cue-in (where only B remains) in 52 % of transitions, the cue-out in 30 %, the midpoint in 18 % **[paper]**. Listeners hear "the new song" when the old one has gone, not when the new one starts.
- **Cue agreement:** for the same track in different DJs' mixes, 23.6 % of cue pairs coincide exactly, 40.4 % lie within 1 bar, 73.6 % within 8 bars, 86.2 % within 16 bars **[paper]**. Cue points are largely a property of the track, not of the DJ.
- **Transition structures** (Kim's 2024 PhD thesis, §5.3.5, [thesis PDF](https://www.dropbox.com/scl/fi/gteey6lhqh5pmtvwvvkqm/taejun_thesis.pdf?rlkey=fg4b21byv131lfi3gya0qqcgj&dl=0)): DTW k-means over the section labels (intro, buildup, drop, outro) under each transition gives 4 prototypes. They differ mainly in "how early the previous song is stopped and how quickly the transition jumps to the main part of the next track"; one prototype skips B's intro, another skips B's intro and buildup **[paper]**.

### 1.2 Kim, Yang, Nam: sub-band analysis of real transitions (NIME 2021; thesis chapter 6.1)
[NIME 2021, DOI 10.21428/92fbeb44.4b2fc7b9](https://doi.org/10.21428/92fbeb44.4b2fc7b9) (the PubPub page refused automated fetches; the same study is chapter 6.1 of the thesis above, which was read), code [mir-aidj/transition-analysis](https://github.com/mir-aidj/transition-analysis)

- 3,930 transitions from 1,216 mixes where both tracks overlap fully. Three bands split at 180 Hz and 3 kHz "following a popular DJ mixer", gains estimated 16 times a second by convex optimization **[paper]**.
- **How often DJs use EQ:** per transition, the method that reconstructs the mix best was the three-band EQ in 2,752 (72.2 %), a fader-only fit in 775 (20.4 %), and a plain linear crossfade in 283 (7.4 %, mostly failures caused by DJ voices or beat-tracking errors) **[paper]**. So roughly **3 in 4 real transitions use EQ and 1 in 4 use only the volume faders**.
- A listening test (14 people, 70 trials) picked the three-band reconstruction as closest to the real mix in 67.1 % of trials, against 18.6 % for a linear crossfade and 14.3 % for fader only **[paper]**.
- No population statistics of the curve shapes (when the bass moves, how long a swap takes) are published here or in the thesis. That gap remains open in the literature.

### 1.3 Kim, Yang, Nam: "Joint estimation of fader and equalizer gains of DJ mixers using convex optimization" (DAFx 2022)
[PDF](https://mac.kaist.ac.kr/pubs/KimYangNam-dafx2022.pdf)

- Dataset: 5,040 mixes from MixesDB with 50,742 transitions **[paper]**.
- The mixer they model is the Allen & Heath Xone:96: low shelf at 180 Hz and high shelf at 3 kHz (Q 1/√2), mid peak at 1 kHz (Q 3). The lowest gains are −80 dB for low and high, but only **−27 dB for the mid** **[paper]**. A DJ can kill the bass or highs; the mids only dip.
- For their synthetic ground truth they encode DJ practice as: transitions of 64 beats (30 to 32 s), **one gain changed at a time per channel in any 4 s ("two hands")**, and all curves monotonic (A only goes down, B only up) **[paper]**.
- **A finding that matters for curve design:** synthetic random curves "have lower energy in the middle of transitions whereas real-world DJ mixes have relatively constant energy through transitions" **[paper]**. Real DJs keep the summed energy flat through the overlap.
- Like the NIME paper, it publishes method accuracy, not the distribution of shapes. The one visualized example shows the DJ raising B's low EQ in one move about 1:33 into the transition **[paper]**.

### 1.4 Vande Veire, De Bie: "From raw audio to a seamless mix: creating an automated DJ system for Drum and Bass" (EURASIP JASMP 2018)
[PDF](https://asmp-eurasipjournals.springeropen.com/track/pdf/10.1186/s13636-018-0134-8), code [lenvdv/auto-dj](https://github.com/lenvdv/auto-dj) (`Application/songtransitions.py`)

The most complete rule-based system, tuned for one genre (drum and bass, 175 BPM).

- **Three transition types, defined by which song sections overlap** ("high" = drop section, "low" = intro or breakdown) **[paper]**:
  - *Double drop:* both songs cued 16 bars before a drop, drops aligned; fade-in 16 bars, fade-out 32.
  - *Rolling:* A cued 32 bars before its next high-to-low boundary, B cued 16 bars before its drop; fade-in 16 and fade-out 16. B's drop continues A's energy.
  - *Relaxed:* A cued 16 bars before its high-to-low boundary, B starts at its beginning; 16 and 16. A rest for the listener.
  - 16 bars at 175 BPM is about 22 s, so these transitions run 44 to 66 s.
- **Variety by a Markov chain** **[paper]**: after relaxed → rolling 0.7, double drop 0.3; after rolling → relaxed 0.2, rolling 0.8; after double drop → relaxed 0.2, rolling 0.8. No double drop after a rolling or another double drop "to avoid layering too many songs at once".
- **Curves** (only described in words in the paper; the numbers are in the code) **[code]**. Each EQ profile value p in [−1, 1] maps to a shelf gain of −26·(1 − p) dB, so 1 is 0 dB, 0.3 is −18 dB, 0.2 is −21 dB and −1 is −52 dB. Low shelf at 70 Hz, high shelf at 13 kHz, Q 0.707, stepped in 20 steps per segment. With P the switch point and L the end (in bars):
  - *Rolling and double drop:* B's volume 0 → 0.2 over the first bar, → 0.8 by P/2, held at 0.8 to P, → 1.0 at P+1. B's bass held at −52 dB until P, then full by P+1 (**the bass swap takes one bar**). B's highs at −52 dB until P/2, rising to −21 dB at P, full at P+1. A's volume 1.0 until P, 0.8 at P+1, held to the middle of the fade-out, then → 0 at L. A's bass full until P, −52 dB at P+1. A's highs −21 dB at P+1, −52 dB by the middle of the fade-out.
  - *Relaxed:* B's volume 0 → 0.7 by P/2 → 1.0 at P; B's lows and highs from −52 dB at P/2 to −18 dB at P, then **jump** to 0 dB at P; A's lows and highs jump to −18 dB at P and fall to −52 dB by the middle of the fade-out; A's volume 1 → 0.7 at the middle of the fade-out → 0 at L.
  - Over the whole overlap the summed output is divided by √(vA² + vB²), a power normalization so that two songs at 0.8 do not sum louder.
  - The paper's rule in words: "the bass and treble of only one song will be heard at any point during the crossfade" **[paper]**.
- **Key rule:** same key, a fifth up or down, or the relative key; songs one semitone off a compatible key are allowed and pitch-shifted **[paper]**.
- **Vocal clash:** per-bar vocal labels (a bar is vocal if it or both neighbors classify as vocal); a clash is two or more vocal bars overlapping, one is tolerated. The detector caught 69.3 % of clashes at the cost of discarding 16.5 % of clean crossfades **[paper]**.
- **Listening test** (18 drum and bass fans): selecting songs by a style descriptor (spectral contrast) won 17 of 18 (mean 3.89 against 2.39 out of 5, p = 7·10⁻⁵). Rhythmic similarity of the overlapping onset curves won 10 of 15 but was not significant (3.76 against 3.41, p = 0.151) **[paper]**. Listeners' complaints about the style-blind mix: "too rushed", "throwing tunes into the mix without much thought". Some noticed bad harmonic mixing and vocal clashes.

### 1.5 Chen, Hsu, Liao, Martínez Ramírez, Mitsufuji, Yang: DJtransGAN (ICASSP 2022)
[arXiv 2110.06525](https://arxiv.org/abs/2110.06525), code [ChenPaulYu/DJtransGAN](https://github.com/ChenPaulYu/DJtransGAN)

- A GAN learns fader and EQ curves; real examples are 7,064 transition sections from 284 livetracklist mixes **[paper]**.
- The curve family it learns: each fade is a clipped ramp ("PReLU1") with a learned start and slope inside the transition window, per band; 4 bands split at 300 Hz, 5 kHz and 20 kHz above 20 Hz **[paper]**. That is, real fades are well fitted by **linear ramps with a delayed start**, not S-curves.
- Pairs: BPM difference ≤ 5, key difference ≤ 2 semitones; transition fixed at **8 bars** following Kim et al. **[paper]**.
- **Listening test** (136 valid of 188 listeners, 46 with DJ experience) on 8 pairs split by vocal type (vocal→vocal, vocal→instrumental, and so on) **[paper]**:
  - Human DJ best; plain sum worst.
  - **No significant difference between the GAN, a linear crossfade and the expert rule baseline.** For listeners without DJ experience, the human was not significantly better than the linear crossfade in preference.
  - Experienced listeners rated everything lower and "rate Linear, Rule and Human higher because they can recognize the mixing technique being used".
  - The GAN was praised for "good use of the high-pass filter when mixing vocals and background music, especially in the vocal → non-vocal type" and criticized for "making the transition too fast", "in particular unpleasant for the vocal-to-vocal type".

### 1.6 Zehren, Alunno, Bientinesi: "Automatic detection of cue points for DJ mixing" (2020; Computer Music Journal 46(3), 2022)
[arXiv 2007.08411](https://arxiv.org/abs/2007.08411), dataset [MZehren/M-DJCUE](https://github.com/MZehren/M-DJCUE)

The "switch point" is where B becomes the dominant song. Three rules from interviews with professional DJs **[paper]**:
1. **Novelty:** it is a moment of high novelty in rhythmic density, loudness, timbre or harmony.
2. **Sections:** it sits on the downbeat that starts a period (every 4 bars in EDM).
3. **Salience:** "the section following a switch point has to be able to stand on its own in the mix"; implemented as the harmonic energy of the next 4 bars ≥ 0.4 × the track's maximum.

Implementation numbers: novelty kernel of 8 bars (4 bars each side), peaks must be the local maximum over 4 bars and above 0.3 × the curve's maximum; features are kick, snare and hi-hat onsets, harmonic and percussive RMS, CQT and pitch class profiles **[paper]**. 134 tracks from 1987 to 2016, 99 to 147 BPM; about 96 % of the generated points were judged usable **[paper]**. A remark worth keeping: a DJ may want to switch "4 or 8 bars before the next track exhibits any element of novelty" to build anticipation **[paper]**.

### 1.7 Argüello, Lanzendörfer, Wattenhofer: "Cue point estimation using object detection" (CUE-DETR, 2024)
[arXiv 2407.06823](https://arxiv.org/abs/2407.06823)

21k cue points by 4 professional DJs (rekordbox libraries) on about 5k EDM tracks **[paper]**. The distance between consecutive cues is most often **16 bars, then 8** **[paper]**. Forcing a minimum spacing of 8 or 16 bars between predicted cues raised precision **[paper]**.

### 1.8 Schwarz, Schindler, Spadavecchia: "A heuristic algorithm for DJ cue point estimation" (SMC 2018)
[PDF](https://zenodo.org/records/1422581/files/smc_2018_036.pdf)

The one cue paper outside EDM: in-store (point-of-sale) background music of all genres, rules from music curators **[paper]**:
- Shorten an intro that is too quiet for too long (more than 4 to 8 beats) or too repetitive, unless the artist is already singing.
- Shorten an outro when "loudness drops significantly but the outro lasts longer than 4 to 8 bars", unless the artist is still singing. Fade out applause.
- Trim silence at both ends to a minimum.
- A loudness threshold of **−9 dB relative to the track's loudest 2 s window** caught every intro or outro the curators had called too quiet.
- In the curators' own cuts, 14 of 30 songs started at their very beginning and only 3 had a fade-in longer than 1 s.
- Why a cue at the end of a structural section works: "at the end of the fade in of the new track, just when it has reached full volume, and the previous song has just vanished, there is a clear change in content ... that catches the ear and clearly signals the start of the track" **[paper]**.

### 1.9 Ishizaki, Hoashi, Takishima: "Full-automatic DJ mixing system with optimal tempo adjustment based on measurement function of user discomfort" (ISMIR 2009)
[PDF](https://zenodo.org/records/1418231/files/IshizakiHT09.pdf)

The only listener measurement of how much tempo change people tolerate in a song-to-song transition.

- **Experiment** **[paper]**: 18 pop songs (RWC), 96 listeners. Each song plays 15 s at its own tempo, then its tempo moves by 0.05 every 3 s, pitch kept (SoundTouch or SOLA), and listeners press when they feel discomfort.
- **Results** **[paper]**: on average discomfort starts at **×1.227 when sped up and ×0.852 when slowed down**, the same for both stretch algorithms. The most common point (histogram peak) is **×1.10 and ×0.90**. Inside 0.90 to 1.10, 11.3 % (SoundTouch) to 15.4 % (SOLA) of listeners already felt discomfort. **Listeners are more sensitive to slowing down than to speeding up.** The authors define a "stricter" range of 0.90 to 1.10 and an "average" range of 0.852 to 1.227.
- **Discomfort function** **[paper]**: L(f) = 0.765·(f − 1) for f > 1 and 1.0·(1/f − 1) for f < 1.
- **Split the change over both songs** **[paper]**: fold A's tempo by 2^C (C from −2 to 2) to the octave nearest B (60 and 120 BPM count as a match), then pick a target tempo between them so that both songs' discomfort is equal: T = [(a − b)·Tlow + √((a − b)²·Tlow² + 4ab·Thigh·Tlow)] / 2a with a = 0.765, b = 1. Because slowing down hurts more, the slower song moves further than the faster one. Each song ramps from its own tempo to T over **5 s** before the crossfade, and back after.
- What the split buys **[derived]** from the formula:

  | Tempo gap | Slow song sped up | Fast song slowed | Inside the strict range? |
  |---|---|---|---|
  | 10 % | +5.5 % | −4.1 % | yes |
  | 21 % | +11.3 % | −8.0 % | just outside (speed-up limit 10 %) |
  | 30 % | +15.9 % | −10.8 % | no; inside the average range |
  | 41 % | +21.3 % | −14.0 % | no; at the edge of the average range |

  The strict range holds gaps up to about 19 % with the equal-discomfort split, or 22 % if the slow song is pushed to exactly +10 % and the fast one to −10 % **[derived]**.
- **Beat phase matters as much as tempo** **[paper]**: some pairs the method scored as low-discomfort were rated worse because "strong beats and weak beats" were aligned. Their fix: correlate beat strength (low-passed at 1.5 kHz, ±50 ms around each beat) across the overlap to choose the alignment.
- Result **[paper]**: 27 listeners rated the split method above single-song stretching on comfort, rhythm and enjoyment (p < 0.001); 50 % of its factors fell in the strict range and 100 % in the average range.

### 1.10 Lin, Lin, Tien, Wu: Music Paste (ISMIR 2009); Lin, Lee, Jang, Wu: "Bridging music via sound effects" (IEEE ISM 2014; IEEE MultiMedia 2015); Y.-T. Lin's PhD dissertation (NTU 2015)
[Music Paste](https://ir.webis.de/anthology/2009.ismir_conference-2009.33) **[abstract only]**, [ISM 2014 record](https://dblp.org/rec/conf/ism/LinLJW14) **[abstract only]**, [dissertation PDF](https://tdr.lib.ntu.edu.tw/bitstream/123456789/4878/1/ntu-104-1.pdf) (read; its text layer is letter-shifted and was decoded)

Pop medleys (Chinese and other pop clips), which makes it the closest work to nori's non-electronic case.
- **Tempo JND measured on their music** **[paper]**: a tempo ratio of 0.95 (0.96 in another chapter) to 1.03 is not perceived for fast clips (120 to 180 BPM), and 0.96 (0.97) to 1.04 for slow clips (40 to 90 BPM).
- **Tempo glide by the beat** **[paper]**: the transition length h (in beats) is extended one beat at a time until the per-beat ratio r = (qB/qA)^(1/h) lies within the JND; then both clips are retimed toward each other ("dual tempo adjustment"). Note that the JND was measured as a static difference between two versions, then applied per beat; that step is the author's assumption.
- **Results** **[paper]**: bar alignment plus dual tempo adjustment beat cutting at the phrase with ordinary tempo adjustment (57 listeners, 7-point scale, Wilcoxon p < 0.05), and "dual tempo adjustment is relatively more important in tempo-dissimilar cases".
- **Pickups:** many pop songs have an anacrusis, so joining at a phrase boundary even with beats matched "sounds like losing tempo"; cut at the phrase but align by bar **[paper]**.
- **Overlap length for pop clips** **[paper]**: with 15 listeners, over 60 % preferred a 4-beat overlap to a 12-beat one; "longer overlap duration is not necessarily more acceptable", and the acceptable overlap "between two rap clips may be shorter than those of two lyric clips". Chroma similarity alone was the preferred way to choose the overlapping windows.
- **Sound effects** **[abstract only]**: inserting sound effects at the join of loosely related clips, with bar alignment and dual tempo adjustment, "greatly enhances the listening experiences among various subjects". The effects and test details were not readable.

### 1.11 Dahl, Granqvist: "Estimating internal drift and just noticeable difference in perception of continuous tempo drift" (Ann. N.Y. Acad. Sci. 2003)
[PDF](https://vbn.aau.dk/ws/portalfiles/portal/65338357/DahlGranqvistVenezia_proc3subm_openacc.pdf)

For a click sequence at 500 ms per beat, the JND for a **continuous tempo drift is 0.27 % per beat** (1.35 ms per interval; 7 listeners) **[paper]**. Earlier work puts the JND for a static tempo difference near 2 %; professional drummers drift about 0.05 % per beat **[paper]**. Music masks more than clicks, so 0.27 % is a floor, not a limit **[inferred]**.

What it means for a glide **[derived]**: at 0.27 % per beat a 10 % change takes 35 beats (9 bars) and a 21 % change 71 beats (18 bars); at 1 % per beat they take 10 and 19 beats; at Lin's 3 % per beat, 3 and 7 beats.

### 1.12 Bittner, Gu, Hernandez, Humphrey, Jehan, McCurry, Montecchio: "Automatic playlist sequencing and transitions" (Spotify, ISMIR 2017)
[PDF](https://archives.ismir.net/ismir2017/paper/000086.pdf)

- **Why bother** **[paper]**: about 95 % of Spotify users leave the fixed crossfade off. In an A/B test on 10 % of users, playlists with DJ-made transitions had a daily return rate 1.4 percentage points higher.
- **Where to cut** **[paper]**: candidates are downbeats; A's transition point must lie in its **last 25 %**, B's in its **first 20 %**. The cost of a pair of windows (fixed length n beats) is a weighted sum of timbre distance and chroma distance over the overlap, inverse loudness ("transition regions with low loudness can often sound awkward and abrupt"), the summed vocal probability of both songs, and a penalty of 1 for each song whose window does not end on a drop or section boundary.
- **Drops** **[paper]**: found from where listeners scrub the playhead (crowd data), not from the audio; peaks in the first 15 s ignored.
- **Tempo** **[paper]**: during an N-beat overlap the n-th beat lasts dₙ = ((N − n)/N)·d₁ + (n/N)·d₂, so the tempo glides linearly in beat duration from A's to B's and each song is stretched to that.
- **Ratings** **[paper]** (48 pairs within 5 BPM, 4 professional curators): Good 64.1 %, OK 28.3 %, Bad 7.6 % (mean 2.56 of 3). Reasons for "bad": beats not aligned 15.2 %, transition mid-vocals 6.5 %, contrasting songs 4.3 %, not on downbeat 2.2 %, awkward transition points 2.2 %, **key clash 0 %**.

### 1.13 Davies, Hamel, Yoshii, Goto: AutoMashUpper (ISMIR 2013; IEEE/ACM TASLP 2014)
[ISMIR 2013 PDF](https://archives.ismir.net/ismir2013/paper/000077.pdf); the journal version ([AIST PDF](https://staff.aist.go.jp/m.goto/PAPER/IEEETASLP201412davies.pdf)) timed out twice, so its weights and listening results are **[not read]**.

"Mashability" between a phrase of one song and every beat offset and key shift (0 to ±6 semitones) of another, from the cosine similarity of beat-synchronous chromagrams, gated by a user tempo ratio; the journal version adds rhythmic similarity and spectral balance **[paper]**. Phrases come from a novelty curve on a downbeat-synchronous semitone spectrogram **[paper]**. Mosaikbox (1.16) reuses its harmonic and spectral-balance terms.

### 1.14 Gebhardt, Davies, Seeber: "Harmonic mixing based on roughness and pitch commonality" (DAFx 2015) and "Psychoacoustic approaches for harmonic music mixing" (Applied Sciences 2016)
[DAFx PDF](https://mediatum.ub.tum.de/doc/1292551/369694.pdf), [Applied Sciences](https://doi.org/10.3390/app6050123)

Scoring the overlap itself instead of comparing keys.
- Method **[paper]**: the 20 strongest partials below 5 kHz per sixteenth-note frame; sensory roughness (a modified Hutchinson and Knopoff model) and pitch commonality (Parncutt) between all partial pairs; one song's partials scaled over ±6 semitones in 1/8-semitone steps to find the least rough alignment.
- Listening test **[paper]** (28 trained listeners, 10 beat-matched mixes of 8 s, 5 conditions): consonance and pleasantness ratings correlate at **r = 0.94**; roughness against rated consonance **r = −0.75**. The least-rough shift was rated significantly more consonant and pleasant than the key match from Traktor's circle of fifths (more consonant in 8 of 10 mixes, more pleasant in all 10; p < 0.0001 against the best of the others). Adding the harmony model did not help significantly. Caveat: Traktor's key detection may have struggled on 8 s excerpts.

### 1.15 Schwarz, Fourer: UnmixDB (ISMIR 2018 late-breaking) and "Methods and datasets for DJ-mix reverse engineering" (CMMR 2019); André, Fourer, Schwarz: multi-pass NMF (2024)
[UnmixDB PDF](https://zenodo.org/records/1422385/files/schwarz-fourer-ismir2018late-breaking-unmixdb.pdf), [CMMR 2019](https://hal.archives-ouvertes.fr/hal-02172427), [arXiv 2410.04198](https://arxiv.org/abs/2410.04198)

- These do **not** report which effects real DJs use. UnmixDB is synthetic: beat-synchronous mixes of 3 track excerpts with a 4-bar linear fade, rendered with no effect, bass boost, compression or distortion, and with no time scaling, resampling or pitch-keeping stretch **[paper]**. The CMMR paper estimates volume curves assuming linear fades **[paper]**.
- Useful vocabulary **[paper]**: "level 1, broadcast mixing" (a plain volume crossfade), "level 2, lounge mixing" (beat-synchronous with tempo adaptation and EQ fades, tracks mostly unchanged), "level 3, performative mixing" (effects, loops, mashups). nori's Smooth is level 2; DJ reaches into level 3.

### 1.16 Sowula, Knees: Mosaikbox (ISMIR 2024)
[PDF](https://repositum.tuwien.at/bitstream/20.500.12708/212628/1/Sowula-2024-Mosaikbox%20Improving%20Fully%20Automatic%20DJ%20Mixing%20Through%20Rule-ba...-vor.pdf), code [robaerd/mosaikbox](https://github.com/robaerd/mosaikbox)

- Mixability weights found by listening **[paper]**: rhythm 0.3, timbre 0.75, key 0.2, harmony 0.2, spectral balance 0.1 (plus lyrics 0.25, which did not help). Picking the single best beat offset without structural constraints gave "unnatural downbeat intervals (7, 9, 15, 17)", so transitions were restricted to segment boundaries.
- Loudness normalized to −14 LUFS; tempo matched only within **±8 %**; transition **16 bars**, 8 before the switch and 8 after; songs play at least 55 s **[paper]**.
- Standard curves (read off their Figure 1) **[paper]**: B's volume 0 → 1 over bars 0 to 4; B's bass at 0.2 gain, rising to 0.3 by bar 8, then **switched to full at bar 8**; B's treble rising linearly 0.3 → 0.8 over bars 0 to 8, then to 1.0 by bar 16; A's bass cut to 0.3 at bar 8 and 0.2 by bar 12; A's treble 1 → 0.8 at bar 8, 0.7 by bar 12; A's volume full until bar 12, then → 0 at bar 16.
- Adaptive rules **[paper]**: if timbre similarity < 0.95, cut A's mids; if rhythm similarity ≥ 0.95, cut the bass less; with B's drums removed, bring B's highs in earlier and less attenuated. With stems (HT-Demucs): if rhythm similarity < 0.95, mute B's drum stem during the overlap; if vocals overlap more than 2 s, attenuate A's vocal stem.
- Listening test **[paper]** (30 listeners, drum and bass): mean transition rating AutoMashUpper 2.49, Mosaikbox without stems 3.08, **with stem rules 3.46** (p < 0.0001 against without), with lyrics 3.03.

### 1.17 Hsu, Lin, Lai, Huang, Liu, Su, Yang: "DJustify: reason-and-verify DJ planning for pop transitions and story-driven sets" (arXiv, October 2026)
[arXiv 2610.33434](https://arxiv.org/abs/2610.33434), code [ddman1101/DJustify](https://github.com/ddman1101/DJustify)

The newest work and the only one built for **vocal-rich pop**, with tempos from 75 to 140 BPM. An LLM proposes a cue-out, a cue-in, an overlap length and a technique; deterministic checks on the rendered audio send back repairs.
- **Results** **[paper]** (103 listeners, 28 with DJ experience, 1 to 10 scale): human DJ 6.7, DJustify 5.6, the same pipeline seeded with Bittner's cues 4.8, a beat-matched crossfade after Bittner 4.7, Mosaikbox 4.0. **"Most of the margin comes from cue placement."** The reasoning step and the checker did not change average ratings (5.7, 5.5, 5.6), but the checker fixed rare mid-phrase vocal cuts that DJ listeners rated **2.0 and 1.8; fixed, 5.0 and 4.8**.
- **Variety** **[paper]**: sampling the planner 10 times per pair gave a median of 6.5 distinct transitions (technique, cut point or entry point), against 1 for a deterministic rule.
- **Techniques** **[paper]**: "a continuum of overlap, from blend moves (long overlap, e.g. a chorus riding over an outgoing outro) to cut moves (abrupt handover with an effect: echo, brake, backspin, filter)".
- **When they beat-match** **[code]** (`render_dsp.py`): the tempo ratio is folded into 0.75 to 1.5 and B is synced **only when |log₂ ratio| ≤ 0.05 (within ±3.5 %)**; B then returns to its own tempo over 8 bars ("the DJ loosens the pitch fader"). Beyond that, no stretching: the pair gets a cut move or a blend over beatless material.
- **Technique card** **[code]** (`prompts/zh/05_FX_CARD_technique_card.json`, translated):
  - *Echo out:* A's last 1 to 4 beats (capped at 2 in the renderer) into a delay of 0.5 to 1.5 beats; best on a phrase end followed by a rest, with a strong last word (a shout or long note). B's next vocal must start at least **4.8 × the delay** beats later, so the tail never covers B's singing.
  - *Reverb tail:* 2 to 8 beats; for ballad or ambient endings; B's vocal at least 6 beats after the cut.
  - *Brake:* at least 4 beats (4 to 8); the source must have energy (≥ 0.35 × the preceding level); best "at an energy peak, a sudden stop, B bursts in"; must land on the last word or the instrumental tail, because a brake over a sung line "drags the singer".
  - *Backspin:* 1 to 2 beats; hip-hop and groove material; lands on the last word or a drum hit.
  - *Blend 16:* a 16-beat equal-power volume crossfade, no filter; B's 16 beats before its cue-in must be instrumental; best when A's interlude or bridge lies over B's intro and B's cue-in is the start of its verse or chorus.
  - *Blend + echo:* A's chorus rides over B's intro for 16 beats (B low-passed and opening), A's last chorus word is echoed out over B's verse.
  - *Loop:* 2 bars of A looped while B rises 0.35 → full; *loop in:* 2 bars of B's instrumental intro looped as a bed (low-passed, quiet, opening) for about 16 beats until A's phrase ends.
  - *Filter out (low-pass or high-pass):* 2 to 16 beats; 2 to 4 when A sings up to its phrase end, 8 to 16 over an instrumental tail; overlap 2 to 16 beats. Its weakness, in the card's words: **"the safest and least memorable; used repeatedly it sounds like an automatic crossfader"**; a sweep over vocals muffles them; a long high-pass residue becomes harsh.
  - Entries: *filter in* (B from lows only to full over 8 beats, "surfacing"), *filter in high-pass* (from highs only over 8 beats; not after a low-pass exit), *spin-up* (B accelerates in over 4 beats), *direct* (B as is; only if B's first 4 beats have at least 0.25 × A's exit level, otherwise use filter-in or spin-up; a direct vocal entry after a smooth exit is abrupt).
  - DSP values **[code]**: echo feedback 0.62, band-limited 250 Hz to 3.8 kHz, 12 repeats; brake speed (1 − t)^1.8 with level 1 → 0.5; backspin 2 × length reversed, speed 1 → 3.5, low-passed at 5 kHz, level 1 → 0.45; filter out 24 dB/oct ladder, 16 kHz → 120 Hz (low-pass) or 30 Hz → 8 kHz (high-pass), exponential in 24 steps; filter in 120 Hz → 16 kHz over 8 beats with level 0.4 → 1 (sine²); level match only when the gap exceeds 1.5 dB, clamped to ±2 dB.
- **Checker gates** **[code]** (`10_checker_MEAN_gate_meanings.json`), each with its repair: cut while A is singing (move cue-out to the phrase end; changing effect parameters does not help); B entered mid-phrase; both vocals in the overlap (B enters on an instrumental window, shorten the overlap, or end A's vocal earlier); effect tail still sounding when B starts singing; effect window over A's singing; effect source nearly silent; B's entry far weaker than A's exit (energy cliff); B's body louder than A's exit during a blend (A drowned); B never reaches its verse or chorus; A handed over too early (**A must play at least 45 s and past 20 % of its length**); a ride with **more than 1 s of quiet**; **a cut inside A's fade-out** ("an exit effect on a dying signal leaves only an energy vacuum"; cue-out before the fade starts).

### 1.18 Kınay, Tekdemir, Gökyılmaz, Yavuz, Ay, Balcısoy: "DJ AI" (Audio Mostly 2025)
[PDF](https://research.sabanciuniv.edu/52373/1/AI_DJ_AM2025.pdf)

MusicGen generates bridge audio conditioned on a blend of the two songs; MERT embeddings pick the windows from the last 30 s (10 s windows, 2 s hop); tempo within ±2 BPM; a raised-cosine crossfade and a low-pass to hide artifacts **[paper]**. The evaluation is objective metrics plus a blind comparison with a fixed 15 s crossfade, no numbers given in the PDF read. It cites a time-frequency graph-cut crossfade by Robinson and Brown **[not read]**. Weak evidence; listed for completeness. A 2023 form-generation paper reports diffusion models failing to bridge two fixed endpoints ("a noisy sound ... or a singular state") ([arXiv 2310.19842](https://arxiv.org/abs/2310.19842)) **[abstract only]**. No diffusion DJ-transition paper with a listening test was found.

### 1.19 MixWeaver (ISMIR 2026, UC San Diego)
Only the public reviews were readable ([reviews](https://cseweb.ucsd.edu/~jmcauley/reviews/ismir26c.txt)). Per stem gain trajectories and window lengths over an intro/outro overlap, scored by penalties for stem overlap, curve smoothness and energy stability, searched with a genetic algorithm. In a 23-person blind test it beat Mosaikbox on most dimensions but not on loudness continuity; reviewers note it has no plain-crossfade or human anchor and covers only smooth blends, not cuts **[reviews, not the paper]**.

### 1.20 Others checked, with little to add
- Hirai, Doi, Morishima, MusicMixer (MMM 2016; JIP 2018): mix points by beat and LDA topic similarity of chroma ([JIP](https://www.jstage.jst.go.jp/article/ipsjjip/26/0/26_276/_article/-char/en)) **[abstract only]**.
- Huang, Chou, Yang, music puzzle games for medleys (AAAI 2018, [arXiv 1709.04384](https://arxiv.org/abs/1709.04384)): a learned similarity embedding orders clips; DJtransGAN used it as a "mixability" score **[abstract only]**.
- Yeh et al., "Rethinking automatic music mixing as sequential stem blending" (ISMIR 2026, [arXiv 2608.05506](https://arxiv.org/abs/2608.05506)): multitrack studio mixing, not song transitions.
- Hein, "A supervised learning framework for generating DJ transitions" (Princeton senior thesis 2025, [record](https://theses-dissertations.princeton.edu/entities/publication/86237923-6f8a-4172-a6e7-d639e97d38d1/full)): trained on one example; no results.

### 1.21 What research has not answered
- No listening test compares transition types (blend, echo-out, brake, filter, cut) for pairs whose tempos differ by 20 to 40 %, or for rock. DJustify's pop set is the closest, and its technique choice is made by an LLM, not tested per technique.
- No published distribution of real EQ curve shapes or bass-swap durations; only method papers (1.2, 1.3) and hand-made curves (1.4, 1.16).
- No study of preferred overlap length for full songs outside EDM, except Lin's 4-beat preference for pop medley clips and DJtransGAN's "too fast for vocal-to-vocal".
- Cliff's "Hang the DJ" (HP Labs 2000), the origin of tempo trajectories between songs, could not be fetched.

---

## 2. Synthesis: what the research says a planner should do

### 2.1 Choosing a transition type for a pair
Research splits transitions into **blends** (long overlap, both songs audible, tempo-locked or beatless) and **cut moves** (short overlap, an effect hands over) [DJustify]. The deciding inputs, in the order the papers weight them:
1. **Tempo compatibility after octave folding.** Real DJs stay within 5 % for 86 % of tracks [Kim 2020]; DJustify syncs only within ±3.5 %; Mosaikbox within ±8 %; listeners begin to object near ±10 % [Ishizaki]. Pairs inside the window get tempo-locked blends; pairs outside it get cut moves or beatless blends.
2. **Vocals at the two ends.** Vocal-on-vocal is the most consistent complaint in every listening test [Vande Veire, Bittner, DJtransGAN, DJustify, Mosaikbox]. A sung end calls for an echo or brake that lands on the last word, or a blend whose overlap is instrumental on B's side.
3. **Energy at the two ends** (section labels high/low). Vande Veire's three types are exactly energy pairings: high into high on drops (double drop), high continuing into a drop (rolling), high into a song's beginning (relaxed). DJustify's direct entry needs B's first 4 beats at ≥ 0.25 × A's exit level.
4. **Recent history**, for variety: no type twice in a row where avoidable, no stacked heavy types [Vande Veire's Markov chain]; sampling among near-equal plans [DJustify].

Key and timbre come after these (2.6).

### 2.2 Length
- EDM DJs: multiples of 8 bars; peaks at 8 and 16 bars (15 to 30 s at 127 BPM), long tail [Kim 2020]. Rule systems use 8 bars [DJtransGAN], 16 bars [Mosaikbox], 32 to 48 bars [Vande Veire].
- Pop and vocal material wants **shorter** overlaps: 4 beats preferred for pop medley clips [Lin], 16-beat blends in DJustify, "too fast" only criticized for vocal-to-vocal [DJtransGAN]. The overlap length is limited by how much of each end is instrumental, not by a fixed number.
- Cut moves: 1 to 8 beats of effect plus whatever tail B lies under [DJustify].

### 2.3 Placement
- B's switch point (where it takes over) is on a downbeat that starts a section, with novelty, and the section after it must stand on its own [Zehren]; that content change is what tells the listener the new song has begun [Schwarz 2018], and listeners place the boundary there [Kim 2020].
- A's cut point is after A's sung phrase ends, never mid-phrase [DJustify, Bittner], and before A's own fade-out starts [DJustify C8].
- Candidate regions: A's last 25 %, B's first 20 % [Bittner]; A must have played at least 45 s and 20 % of its length [DJustify].
- Cues cluster on 8- and 16-bar lines [CUE-DETR, Kim 2020]; within a track, DJs agree within 8 bars 74 % of the time [Kim 2020], so per-song cue candidates can be computed once at analysis time.
- In pop, many phrases start on a pickup before the downbeat; join by bar, not by phrase start [Lin].
- Avoid overlaps where both songs are quiet: they "sound awkward and abrupt" [Bittner]; quiet intros longer than 4 to 8 beats and fading outros longer than 4 to 8 bars are trimmed, unless someone is singing [Schwarz 2018].

### 2.4 EQ and fader shapes
- **Use EQ in most transitions, not all:** 72 % of real transitions use EQ, 20 % only faders [Kim NIME/thesis]. A plain fader crossfade is a legitimate fifth of the mix, and statistically hard to beat [DJtransGAN].
- **Keep the summed energy flat** through the overlap [Kim DAFx]; power-normalize the two gains [Vande Veire].
- **One low end at a time.** B's bass is cut (−20 to −50 dB) until the switch and A's after it; the swap takes **one bar** [Vande Veire rolling] or is **instant on the downbeat** [Vande Veire relaxed, Mosaikbox].
- **Highs lead the bass.** B's highs open before the switch: from −52 dB to −21 dB over the half before the switch [Vande Veire] or 0.3 → 0.8 linearly [Mosaikbox]. A's highs go after the switch, within half the remaining fade.
- **Mids only dip.** DJ mixers cut mids by about 27 dB at most [Kim DAFx]; cut A's mids when the timbres differ [Mosaikbox]; a high-pass on the singing deck in a vocal/instrumental pair was the GAN's best-received move [DJtransGAN].
- **Volume:** B rises fast (0.2 in the first bar, 0.8 by halfway to the switch) and A holds near full until the switch, then falls [Vande Veire, Mosaikbox]. Fades are well modelled by linear ramps with a delayed start [DJtransGAN]. Controls move one at a time, about one change per 4 s per deck [Kim DAFx].
- **Band edges** differ by source: 70 Hz and 13 kHz shelves [Vande Veire code], 180 Hz and 3 kHz [Xone:96 in Kim], 300 Hz / 5 kHz [DJtransGAN].

### 2.5 Tempo gaps
1. **Fold octaves first.** 60 and 120 BPM are a perfect match [Ishizaki]; nori already does this.
2. **Split the change between both songs, weighted against slowing down** (slowing is 1.31 times as uncomfortable as speeding up: 1/0.765) [Ishizaki]. This roughly doubles the gap that stays in the comfortable range compared with stretching one song.
3. **Glide, do not step.** Change tempo gradually, per beat [Bittner, Lin]: imperceptible below 0.27 % per beat on clicks [Dahl], tolerated up to about 3 % per beat in pop clips [Lin]; Ishizaki ramped over 5 s; DJustify returns B to its own tempo over 8 bars.
4. **Limits:** comfortable to ±10 % per song, with a noticeable minority objecting even inside it; about half of listeners object by +23 % or −15 % [Ishizaki]. Dual split covers a 19 to 22 % gap in the strict range and about 44 % in the average range [derived].
5. **Beyond that, do not beat-match.** Hand over with a cut move (echo, reverb, brake, backspin, filter exit) whose shared audio is too short for beats to clash, with B entering softly or on its own downbeat [DJustify]; or overlap where at least one side has no beat. A brake is "classic" exactly for a stop-then-new-song handover [DJustify].
6. **Phase matters.** Matching tempo with strong beats on weak beats is rated worse than a larger tempo change with correct phase [Ishizaki]; "beats do not align" is the top failure among curated transitions (15 %) [Bittner].

### 2.6 Scoring compatibility
- For a fixed queue (nori), compatibility chooses the window and the technique, not the next song.
- **Key distance is a weak predictor**: curators flagged key clash in 0 % of beat-matched dance transitions [Bittner]; a key match from the circle of fifths lost to a measured roughness score [Gebhardt]. A signal-level consonance score (roughness of the two overlapping windows' partials) predicts rated pleasantness (r = −0.75 against consonance, which tracks pleasantness at 0.94) [Gebhardt].
- **Timbre/style matters most at the set level**: 17 of 18 listeners preferred style-aware selection [Vande Veire]; timbre carries the largest weight (0.75) in Mosaikbox's mixability; timbre and chroma distance over the overlap are Bittner's main cost terms.
- **Rhythmic similarity of the overlap** helps little (n.s.) [Vande Veire], but rhythm mismatch is where stems pay: muting B's drums when rhythms differ raised ratings from 3.08 to 3.46 [Mosaikbox].
- Window choice by chroma similarity was preferred for pop clips [Lin].

### 2.7 What listeners reward and punish
- **Punished:** misaligned beats (15 %) [Bittner]; cutting or entering mid-phrase (DJ ratings 2.0 against 5.0 fixed) [DJustify, Bittner 6.5 %]; two vocals at once [all]; vocal-to-vocal transitions that are too fast [DJtransGAN]; contrasting songs (4 %) [Bittner]; quiet or empty overlaps, more than 1 s of silence, energy cliffs [Bittner, DJustify]; "too rushed" mixes [Vande Veire]; effects over a dying fade-out [DJustify].
- **Rewarded:** good cue placement above everything else (+0.75 of 10 from cues alone) [DJustify]; techniques experienced listeners recognize [DJtransGAN]; a high-pass on the singing deck in vocal/instrumental pairs [DJtransGAN]; style continuity [Vande Veire]; stem-aware muting of clashing parts [Mosaikbox].
- **Ceiling:** human DJs still lead by about 1 point of 10 [DJustify], but untrained listeners often cannot tell a human from a linear crossfade [DJtransGAN].

---

## 3. Rules nori could adopt

Each rule is testable in `eval.rs` or the planner's unit tests, unless marked as needing a listening check. "A" is the outgoing song, "B" the incoming one.

### Tempo
1. **Dual tempo split.** When both grids are reliable and the folded ratio R ≤ 1.22, move both songs toward T from Ishizaki's equal-discomfort formula (a = 0.765, b = 1) instead of stretching B alone; each song stays within ×0.90 to ×1.10. *Test:* for R = 1.21 the plan stretches A by about −8 % or +11 % and B by the complement, never one song by 21 %. *Source:* Ishizaki 2009 (1.9). This turns nori's 20 % single-song cap into about 22 % of gap with less stretch on each song.
2. **Slowing costs more than speeding.** When only one song can move, prefer speeding the slower song up over slowing the faster one down, for equal ratio. *Test:* planner picks the speed-up when both options exist. *Source:* Ishizaki (1.9).
3. **Glide rate.** Ramp each song's tempo toward T at no more than about 1 % per beat (and back after the switch, over 8 bars as DJustify does). *Test:* the per-beat ratio in the rendered stretch schedule never exceeds 1.01; a 10 % change takes at least 10 beats. *Source:* Dahl 2003 (floor 0.27 %/beat), Lin 2015 (3 %/beat tolerated), DJustify code (8-bar return). The exact rate needs a listening check.
4. **No beat-matching beyond the comfortable range.** With R > 1.22 after folding (or one grid unreliable), do not stretch; choose a cut move (rules 9 to 12) or a beatless overlap (rule 13). *Test:* no stretch schedule appears for R > 1.22. *Source:* Ishizaki (1.9), Kim 2020 (98.6 % of real tracks within 20 %), DJustify (syncs only within ±3.5 %).
5. **Phase by beat strength.** Among downbeat alignments of equal score, prefer the one maximizing the correlation of low-band (< 1.5 kHz) beat strength across the overlap. *Test:* a synthetic pair with a strong 1 and 3 is not aligned 1 on 2. *Source:* Ishizaki (1.9), Bittner (beats misaligned is the top failure).

### Choosing the type (DJ style)
6. **Pick by tempo, vocals and energy, in that order.** Tempo-locked blend within rule 1's range; otherwise a cut move; sung ends constrain which effect; section energies pick among blends (drop to drop, high into drop, high into B's start). *Test:* table test over (R, A sung at end, B sung at start, A high/low, B high/low) → expected family. *Source:* Vande Veire (1.4), DJustify (1.17).
7. **Markov variety.** Keep the last transition's family and draw the next from a transition matrix that forbids repeats of heavy moves (no two double drops or two brakes in a row; Vande Veire's matrix as a starting point: after a relaxed move 0.7 rolling / 0.3 double drop; after rolling or double drop 0.2 relaxed / 0.8 rolling). Choose among candidates within a small score margin of the best. *Test:* over a synthetic queue of 20 compatible pairs, no family exceeds a set share and no heavy family repeats back to back. *Source:* Vande Veire (1.4), DJustify (median 6.5 distinct plans per pair).
8. **Filter sweeps are the fallback, not the default.** Cap the share of filter-only exits in DJ style. *Test:* in the transition harness, filter exits are under a fixed share when other moves are feasible. *Source:* DJustify technique card ("least memorable; repeated use sounds like an automatic crossfader").

### Cut moves (for tempo gaps; adapted to nori's "B starts at the transition, nothing is a hard cut")
9. **Echo out.** A's last 1 to 2 beats into a 0.5 to 1.5-beat delay, feedback about 0.6, band-limited to about 250 Hz to 3.8 kHz, on a phrase end with a strong last word; B's first sung onset at least 4.8 × the delay after the cut. *Test:* plan rejects an echo when B's vocal starts sooner. *Source:* DJustify code (1.17). nori already has a beat-synced echo; the new parts are the vocal gap and landing on the last word.
10. **Brake.** A slows to a stop over 4 to 8 beats with speed (1 − t)^1.8 and level 1 → 0.5, only over A's last word or instrumental tail and only when that window has energy (≥ 0.35 × the level before); B comes in on its downbeat as the brake ends, under the brake's last beat rather than after silence (nori's no-hard-cut rule). *Test:* no brake window overlaps A's vocal; the gap between A's audible end and B's onset is under 1 s. *Source:* DJustify code (1.17). New effect for nori; it is the classic DJ answer to an unmatched tempo.
11. **Backspin** (hip-hop, groove material): 1 to 2 beats reversed from twice that length, speed 1 → 3.5, low-passed near 5 kHz, on a drum hit or last word. *Test:* as rule 10. *Source:* DJustify code. Optional.
12. **Soft entry after a cut move.** If B's first 4 beats are below 0.25 × A's exit level, enter with a filter-in (low-pass 120 Hz → 16 kHz over 8 beats, level 0.4 → 1, sine²) or a 4-beat spin-up; otherwise enter directly. Never follow a smooth filter exit with a direct vocal entry. *Test:* plan picks filter-in for a quiet-intro B. *Source:* DJustify code (1.17).
13. **Beatless overlap for tempo gaps.** When A's last bars or B's first bars have no beat (a pad, a breakdown, a rubato intro), overlap there without tempo change; the beatless side must be the one carrying the overlap. *Test:* a synthetic pair with a 30 % gap and a beatless outro gets a blend over the outro, not a fade over drums. *Source:* [inferred] from Ishizaki (tempo mismatch is the discomfort) and Zehren/Cliff (transitions in segments with no clear pulse); no listening study isolates it.

### Placement
14. **A finishes its phrase.** A's cut or effect window never covers a sung syllable; move the cue-out to the end of the phrase rather than shortening the effect. *Test:* the harness's voices-competing measure gains a "cut mid-phrase" count that must be 0 on the synthetic set. *Source:* DJustify (ratings 2.0 → 5.0 when fixed), Bittner (6.5 %).
15. **B enters at a phrase start, aligned by bar.** B's switch point is a section start; a pickup before it may sound under A. *Test:* B's switch lies on a phrase line in every beat-matched plan; nori's harness already counts this. *Source:* Zehren rules 2 and 3, Lin (anacrusis).
16. **Salient arrival.** The 4 bars after B's switch point have harmonic energy ≥ 0.4 × B's maximum (or reach the song's body, which nori's drop detector tests). *Test:* landing candidates failing it score as "straight in". *Source:* Zehren (1.6), Schwarz 2018 (content change at the end of the fade-in).
17. **Not inside A's fade-out.** Effects and cuts start before A's level falls below its fade-out start; a cut move on a fading signal is replaced by a plain blend. *Test:* no exit effect window starts after the breakdown/fade-out point. *Source:* DJustify gate C8. nori's `breakdown` exit already aims the swap there; extend the rule to effects.
18. **A plays long enough.** Never hand over before A has played 45 s and 20 % of its length. *Test:* trivially checked on short songs. *Source:* DJustify gate C9.
19. **No dead air in a blend.** During a blend, the summed level never stays more than 1 s below a threshold (for example −20 dB under the louder song's body). *Test:* harness measure "silence inside the overlap" ≤ 1 s. *Source:* DJustify gates C3, C5b; Bittner (quiet overlaps sound abrupt).
20. **Search windows only near the ends.** A's candidates in its last 25 %, B's in its first 20 %, intersected with nori's 15 s skip cap. *Test:* candidate generator bounds. *Source:* Bittner (1.12).

### EQ and gain (both styles)
21. **One bass at a time, swapped in one bar or on the downbeat.** Keep nori's bass swap; make its length a choice between instant and one bar, the bar version for rolling or high-into-high blends. *Test:* the low band of both decks is never above −12 dB at once outside the swap bar. *Source:* Vande Veire (1.4), Mosaikbox (1.16).
22. **Highs lead.** B's highs rise from about −50 dB to about −20 dB over the half before the switch and are full one bar after; A's highs fall to about −20 dB at the switch and are gone halfway through the rest. *Test:* band levels at checkpoints in the rendered mix. *Source:* Vande Veire code (1.4), Mosaikbox (1.16). nori holds B's highs down then releases them; this fixes the shape.
23. **Mids only dip, and only on a timbre clash or two voices.** Cut A's mids (at most about 27 dB) when the two songs' timbres differ; otherwise leave mids. *Test:* mid cut present only when the timbre distance is above a set value. *Source:* Kim DAFx (Xone:96 mid range), Mosaikbox (1.16), DJtransGAN (high-pass on vocals).
24. **Flat energy.** Normalize the two deck gains so that the overlap's loudness stays within about 1 LU of the louder song's body, with no dip in the middle. *Test:* nori's harness loudness measure, extended with a minimum-loudness bound. *Source:* Kim DAFx ("relatively constant energy"), Vande Veire's √(vA² + vB²) normalization.
25. **B rises fast, A holds.** Volume: B reaches about 0.8 by halfway to the switch; A stays near full until the switch and falls after. Use linear ramps with delayed starts for the bands. *Test:* gain checkpoints. *Source:* Vande Veire code, Mosaikbox, DJtransGAN.
26. **Fader-only transitions are allowed.** In Smooth style, let roughly a quarter of beat-matched transitions be fader-only blends when both ends are sparse or the pair is very compatible, rather than always running the full EQ program. *Test:* share of EQ-free blends in the harness. *Source:* Kim NIME/thesis (20 % fader-only, 72 % EQ), DJtransGAN (linear crossfade not significantly worse).

### Length
27. **Length by content, in 4-bar units.** Beat-matched instrumental-on-instrumental blends: 8 or 16 bars. Any overlap with a voice on either side: 4 bars or less, or a cut move. *Test:* table test of overlap length against vocal state. *Source:* Kim 2020, Mosaikbox, Vande Veire (EDM lengths); Lin (4 beats for pop clips), DJtransGAN (too fast for vocal-to-vocal means vocal pairs need a different move, not a shorter blend). Needs a listening check on rock.

### Compatibility
28. **Score the overlap by roughness, not only Camelot.** Compute a roughness score between the two windows that would sound together (20 strongest partials below 5 kHz per sixteenth note) and use it where nori now uses Camelot distance to pick the length and the filter shape. *Test:* on synthetic chords, a tritone overlap scores rougher than a fifth; nori's existing Camelot rules still hold on clean cases. *Source:* Gebhardt 2015/2016 (1.14), Bittner (key clash 0 % of complaints). Pitch-shifting to fix roughness is out of scope: DJs transpose 2.5 % of tracks [Kim 2020].
29. **Timbre and chroma distance over the overlap pick the window.** Among candidate windows, prefer lower timbre (and chroma) distance between the overlapping parts. *Test:* a pair with one matching and one clashing outro window picks the matching one. *Source:* Bittner (1.12), Lin (chroma preferred), Mosaikbox weights.

### Evaluation
30. **Add the research's failure counts to the harness:** mid-phrase cuts, mid-phrase entries, effect tails over B's vocal onset, silence over 1 s inside an overlap, energy cliff at B's entry (< 0.25 × A's exit), beat-phase errors (strong on weak). Each is a gate in DJustify's checker or a curator complaint in Bittner, and each is measurable on nori's synthetic songs. *Source:* DJustify (1.17), Bittner (1.12).
31. **Listening check before shipping each new move,** with the owner's rock playlist: DJ ratings for "same pair, technique A against B" are what the literature lacks for big tempo gaps (1.21).

---

## Sources

- Kim, Choi, Sacks, Yang, Nam, ISMIR 2020: https://arxiv.org/abs/2008.10267 ; https://mir-aidj.github.io/djmix-analysis/
- Kim, PhD thesis, KAIST 2024: https://www.dropbox.com/scl/fi/gteey6lhqh5pmtvwvvkqm/taejun_thesis.pdf?rlkey=fg4b21byv131lfi3gya0qqcgj&dl=0
- Kim, Yang, Nam, NIME 2021: https://doi.org/10.21428/92fbeb44.4b2fc7b9 ; https://github.com/mir-aidj/transition-analysis
- Kim, Yang, Nam, DAFx 2022: https://mac.kaist.ac.kr/pubs/KimYangNam-dafx2022.pdf
- Vande Veire, De Bie, EURASIP JASMP 2018: https://asmp-eurasipjournals.springeropen.com/track/pdf/10.1186/s13636-018-0134-8 ; https://github.com/lenvdv/auto-dj
- Chen et al., DJtransGAN, ICASSP 2022: https://arxiv.org/abs/2110.06525 ; https://github.com/ChenPaulYu/DJtransGAN
- Zehren, Alunno, Bientinesi 2020: https://arxiv.org/abs/2007.08411
- Argüello, Lanzendörfer, Wattenhofer 2024: https://arxiv.org/abs/2407.06823
- Schwarz, Schindler, Spadavecchia, SMC 2018: https://zenodo.org/records/1422581
- Ishizaki, Hoashi, Takishima, ISMIR 2009: https://zenodo.org/records/1418231/files/IshizakiHT09.pdf
- Lin et al., Music Paste, ISMIR 2009: https://ir.webis.de/anthology/2009.ismir_conference-2009.33 ; Bridging music via sound effects, ISM 2014: https://dblp.org/rec/conf/ism/LinLJW14 ; Y.-T. Lin dissertation, NTU 2015: https://tdr.lib.ntu.edu.tw/bitstream/123456789/4878/1/ntu-104-1.pdf
- Dahl, Granqvist 2003: https://vbn.aau.dk/ws/portalfiles/portal/65338357/DahlGranqvistVenezia_proc3subm_openacc.pdf
- Bittner et al., ISMIR 2017: https://archives.ismir.net/ismir2017/paper/000086.pdf
- Davies et al., AutoMashUpper: https://archives.ismir.net/ismir2013/paper/000077.pdf ; https://staff.aist.go.jp/m.goto/PAPER/IEEETASLP201412davies.pdf (not read)
- Gebhardt, Davies, Seeber, DAFx 2015: https://mediatum.ub.tum.de/doc/1292551/369694.pdf ; Applied Sciences 2016: https://doi.org/10.3390/app6050123
- Schwarz, Fourer, UnmixDB: https://zenodo.org/records/1422385/files/schwarz-fourer-ismir2018late-breaking-unmixdb.pdf ; CMMR 2019: https://hal.archives-ouvertes.fr/hal-02172427 ; André, Fourer, Schwarz 2024: https://arxiv.org/abs/2410.04198
- Sowula, Knees, Mosaikbox, ISMIR 2024: https://repositum.tuwien.at/bitstream/20.500.12708/212628/1/Sowula-2024-Mosaikbox%20Improving%20Fully%20Automatic%20DJ%20Mixing%20Through%20Rule-ba...-vor.pdf
- Hsu et al., DJustify, 2026: https://arxiv.org/abs/2610.33434 ; https://github.com/ddman1101/DJustify
- Kınay et al., DJ AI, Audio Mostly 2025: https://research.sabanciuniv.edu/52373/1/AI_DJ_AM2025.pdf
- MixWeaver reviews, ISMIR 2026: https://cseweb.ucsd.edu/~jmcauley/reviews/ismir26c.txt
- Others: https://www.jstage.jst.go.jp/article/ipsjjip/26/0/26_276/_article/-char/en ; https://arxiv.org/abs/1709.04384 ; https://arxiv.org/abs/2608.05506 ; https://arxiv.org/abs/2310.19842 ; https://theses-dissertations.princeton.edu/entities/publication/86237923-6f8a-4172-a6e7-d639e97d38d1/full

Could not open: the NIME 2021 PubPub page (HTTP 403; the thesis chapter was read instead), the AutoMashUpper journal PDF (timed out twice), Cliff's "Hang the DJ" and hpDJ HP Labs reports, the full text of Lin et al.'s ISM 2014 and IEEE MultiMedia 2015 papers.
