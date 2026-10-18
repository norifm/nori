# Lyrics and downloads, the next step

Internal planning note, not published.

Written on 2026-09-26, from the owner's requests. Read `docs/features.md` (lyrics) and `crates/lyrics` first. The
current trust score is in `crates/lyrics/src/trust.rs`, and the race is in `race.rs`.

## Requests
1. **Analyse when downloading.** Songs downloaded for offline are analysed for AutoMix as they download when AutoMix
   is on. This was done on 2026-09-25: MeasuringSink on Android, and the desktop Downloader.
2. **Catch up.** Downloads made while AutoMix was off, or before "Better beat detection" was on, should be analysed
   later. Do it automatically, but only while the phone is charging (and ideally idle, with the screen off), never
   on battery.
3. **Lyrics with downloads.** When a song is downloaded, also fetch its lyrics. Run the full race over all enabled
   sources and wait for it to finish rather than taking the first acceptable answer, then keep the best one
   durably, so offline plays have lyrics.
4. **More sources by default.** DONE 2026-09-26: every source on, KuGou and SimpMusic in the first wave, and the race keeps asking word-timing services while the best answer is only line-timed. Before: Today 9 of 17 are on: PAXSENIX,
   BINILYRICS, UNISON, BETTER_LYRICS, KUGOU, NETEASE, LYRICS_PLUS, SIMPMUSIC and LRCLIB. Off are GENIUS (plain
   text only), MEGALOBIZ, MUSIXMATCH, PAXSENIX_MUSIXMATCH, PAXSENIX_SPOTIFY, PORTATO, YOUTUBE_CAPTIONS and
   YOUTUBE_MUSIC. Decide from measured reliability and cost (latency, failures, junk), not by count.
5. **Check that synced lyrics really are in sync.** Find a good way to tell whether a synced answer's timing fits
   the song.

## What exists and what others do
- **nori's trust score already covers:**
  - the metadata match (title, artist, album, length);
  - timing level (word over line over none) and plausibility (in order, within the song, no long silences, the
    last line near the end);
  - agreement between sources;
  - each service's prior;
  - junk penalties.
- **LRCLIB** only answers when the duration matches its record within ±2 s ([LRCLIB docs](https://lrclib.net/docs)).
- **syncedlyrics** (a Python library) queries Musixmatch, LRCLIB, NetEase, Megalobiz, Genius and Tencent, and
  prefers synced over plain ([pypi](https://pypi.org/project/syncedlyrics)).
- **LiriQo** ranks multi-provider results by sync level: syllable, then word, then line
  ([GitHub](https://github.com/AlFarrizi-Studio/LiriQo)).
- **Checking against the audio.** The full pipeline is vocal separation (Demucs), then recognition (Whisper), then
  forced alignment (wav2vec2 / WhisperX) ([usersync](https://github.com/iamjrmh/usersync),
  [auto-lrc](https://github.com/liu-xiaoran/auto-lrc), [lyrics-sync](https://github.com/mikezzb/lyrics-sync)). The
  research method is acoustic models with DTW ([arXiv 1906.10369](https://arxiv.org/pdf/1906.10369)). These are too
  heavy for a phone, and forced alignment needs the words (fine: we have them, but the models are hundreds of MB).
- **The cheap on-device check [inferred]:**
  - A **vocal activity curve**: energy in the vocal band (about 200 Hz–4 kHz) against the rest, with spectral flux
    or harmonicity. Good enough to tell sung from instrumental stretches and to find phrase onsets.
  - Compare the **line start times** with the vocal onsets. This gives:
    1. a **sync score** (the share of lines whose start sits near a vocal onset, lines inside instrumental
       stretches counting against);
    2. a **global offset**, the best shift found by cross-correlating a line-start impulse train with the onset
       curve within ±3 s. That lets the app fix lyrics that are uniformly early or late;
    3. **drift**: a first vs last half offset difference means the timing was made for another version
       (radio edit, live).
  - It runs in the same decode pass as the AutoMix analysis, so there's no extra decode. Store it with the track
    analysis. It feeds `trust.rs` as another term, and the offset is applied when the lyrics are shown.
  - Word-timed answers can be checked more finely (word onsets vs vocal onsets).
- **Test data.** Synthetic songs with known vocal lines (`automix/synth.rs` already makes a sung line) and made-up
  lyric timings, including shifted and drifted variants. **Never use real song lyrics in code, tests or logs.**

## Plan
1. **Downloads:**
   - Catch-up analysis while charging: WorkManager with `requiresCharging`, battery not low, and ideally
     `requiresDeviceIdle`, low priority, stopping when unplugged. Show the progress in settings (a kind and counts,
     no sentences from the core).
   - Lyrics fetched with each download through a full race, and kept durably for downloaded songs (not evicted by
     the lyrics cache; "Clear lyrics cache" leaves them, or refetches them).
2. **Sync check:** the vocal activity curve plus line-onset matching, as above. It gives a score, an offset and
   drift, used in `trust.rs` and when showing the lyrics. Measure it on the synthetic set first, then on the
   user's library by hand, with the perf report showing scores and offsets.
   *Done on the synthetic set, 2026-09-26* (nori-player `automix/vocal.rs`, nori-lyrics `sync.rs`):
   - The curve is "pitched movement" in 250 Hz-4 kHz: how far the log level moves from frame to frame in the
     bins around the band's clear spectral peaks (vibrato, glides and syllables move; drums are not peaks, pads
     do not move). Sung vs unsung frames of the eleven sung AutoMix songs: AUC 0.987 (the band's power share,
     AutoMix's `vocal`, 0.71). 17 frames a second, a byte each, about 1 KB a minute in `vocal_curve` beside
     `track_analysis` (analysis version 10). It costs 20 ms per four-minute song on the host (5 ms a minute,
     about 11 % of the analysis front end).
   - The check slides the lines' sung spans (words, or line start to end capped by the length of the text)
     and their starts over the curve within ±3 s: best offset, how clearly it wins, a score (lines that start
     where the voice is heard and rises, less for spans over instrumental stretches and long unlined singing),
     and drift (the line cut under which the two parts fit best each at its own offset). On 11 songs × 14
     made-up timings (invented words): offset median error 19 ms, worst 224 ms, 99 of 110 within 60 ms; 74
     of 77 shifted timings corrected (the rest left alone as unsure); every exact timing reads Fits; all 11
     half-edited versions read Drifts; good scores min 0.58 mean 0.93 against wrong songs' max 0.51 mean 0.30
     (AUC 1.00). `cargo test --release -p nori-lyrics sync_eval -- --ignored --nocapture`.
   - *Tuned on the owner's library, 2026-09-26* (nori-lyrics `sync_tune.rs`, two ignored tests; timings only,
     no words kept): 59 songs (pop 9, rock 9, alt 8, metal 8, electronic 9, classical 6, rap/R&B 4, live 5,
     folk 1), every default service asked, 340 answers. Answers of at least two independent timings with the
     same line starts (within 150 ms) were taken as good (69, copies of one catalogue counted once), the good
     ones shifted and half-shifted by hand, and laid on another song's curve as wrong. Electronic and classical
     had no lyrics; live had no two services agreeing.
     - Pitched movement hardly tells singing from the rest in real mixes: AUC about 0.7 in pop, rock, rap
       and alt, 0.5 in metal (guitars read as voice). The timing still works. 300 Hz-3 kHz and a peak five
       times its surroundings (was 250 Hz-4 kHz, ten times) put the offsets' p90 error from 136 to 38 ms
       (analysis version 11, curve version 2). No cheap variant fixed metal.
     - The best score alone hardly told another song's timing from the right one in metal, rap and rock (AUC
       0.52-0.75): anything fits a restless curve somewhere. How much better the lines fit at their offset than
       slid 7-23 s away (`LIFT`) does (AUC 0.94 overall). Poor is now `POOR` 0.35 or a lift under 0.06.
     - Good answers came out 100 ms early (the curve hears a vowel after its consonant), line-timed ones
       another 140 ms (lines are shown ahead of the voice); both are taken off (`LAG_MS`, `LINE_LEAD_MS`)
       before an offset is applied. `OFFSET_SURE` 0.4.
     - Before → after: good answers demoted 5.8 % → 5.8 % (Poor 2.9 → 1.4, Drifts 2.9 → 4.3), shifted 25 % →
       13 %; another song's timing caught 45 % → 55 %; timings shifted by hand put right 77 % → 84 %;
       half-shifted ones Drifts 40 % → 44 %. `DRIFT_MS`, the part thresholds and `W_SYNC` were kept.
     - Uncertain: rock still lets another song's timing through (10 % caught); only 4 rap songs; "good" is
       agreement, not truth (one metal song's services all agree on a timing 2 s off this file).
   - *Checked on real audio, 2026-10-18*: 20 songs streamed from YouTube Music through nori-cli (its own
     analysis), every service asked, 108 answers; truth taken as each answer's median line time less Apple's
     word timing (BetterLyrics/BiniLyrics), which spectrograms showed on the voice for the songs looked at.
     - No fixed latency in the curve: frame times are the window's end less 1.28 hops plus half a curve frame
       (about 20 ms), and Apple's timings fit at 0 ± 200 ms. The offsets scatter, though: the best offset's
       median error is 181 ms (median +58 ms late), so moves under `OFFSET_MIN_MS` are noise.
     - The large error was beat-sized: on Climbing Up the Walls an instrument swells 1.1 s before every line
       and the curve hardly hears the voice over it, so the rise at the line starts (half the fit) followed the
       swell and the right lyrics were moved +1176 ms. The word spans there fit no better moved (contrast
       0.041 → 0.043, against +0.03 to +0.3 for real moves), so a move of half a second or more of word-timed
       lyrics now needs the spans to gain `SPAN_GAIN` too. Line-timed spans are guessed and tile the singing,
       so they cannot say this. Wrong moves 11 → 9, moves to within 300 ms unchanged (8); median absolute
       error of what is shown 40 → 29 ms, mean 167 → 145 ms.
     - Left: answers whose words tile the whole line (KuGou) can still slide by a section (Vitamin R,
       −3285 ms where the answer is ~0.8 s late); several 350-400 ms moves have an unclear truth (Teardrop,
       Change). The owner's wanted offsets (+300 ms, +500 ms) are 0.3-0.5 s sooner than the voice on these
       streams for both songs, which points to how the playhead is heard on the phone rather than the check.
   - *Voice told from instruments, 2026-10-18* (classical, no model; `vocal.rs`, analysis version 13, curve
     version 3): the curve now sums, per frame, only over the peaks of the strongest harmonic series (F0
     80-1000 Hz, found among peaks that last three frames: 8 strongest × harmonics 1-8, scored by amplitude
     decaying 0.92 per harmonic): their level movement as before plus 0.5 × amplitude × how far their frequency
     moved over two frames (full at 30 cents: vibrato, glides). Noise, drums and cymbal swells have peaks that
     come and go and fit no series; a held note does not move. Measured on the same 22 songs' audio again
     (nori-cli renders, 6 dB under the stream, so fed back at ×2: that reproduces the stored curves' results).
     - Truth fixed for Smells Like Teen Spirit: BiniLyrics' (Apple) timing runs 1.0 s before the voice the
       spectrogram shows (first word 33.5 s, voice from 34.7 s); Unison's (34.57 s) is on it and is now the
       reference. Old and new curves both moved those lyrics by about +1.1 s, counted wrong until now.
     - All 128 answers (wrong = moved > 300 ms off, Teardrop's doubtful truth apart; missed = off 0.3-6 s
       and left alone): old curve wrong 0 (Teardrop 4), right 5, missed 19, median error 17 ms; new wrong 0
       (Teardrop 3), right 13, missed 12, median 17 ms. Shown answers (22): right 0 → 2, missed 2 → 1, wrong
       0. Frames inside Apple's words told from frames away from any word: AUC 0.73 → 0.78 (19 songs).
     - Climbing Up the Walls' swell is broadband noise, not a chord: its KuGou and LyricsPlus answers (414 ms
       late) are now moved to within 164 ms. A synthetic noise swell before every line reads 1.05 of the voice
       on the old curve, 0.38 on the new (`a_swell_is_not_a_voice`). A swelling chord of detuned strings
       still reads as voice (its notes form a series and beat): not solved.
     - Change's Apple timing is wanted −0.4 s by the new curve; its word spans gain 0.0094 there, so
       `SPAN_GAIN` went 0.01 → 0.02 (real moves gain 0.03-0.3) to keep it Fits in every nearby setting
       tried. Nearby settings (frequency weight 0.25-1, peak reach 1.25-2 bins) gave right 11-14, missed
       12-14; one moved California's LRCLIB answer 301-305 ms off (right direction, just past 300).
     - Cost: the analysis front end 47.7 → 61.6 ms CPU per minute of audio on the host (×1.29).
     - The synthetic voice (no consonants) is heard about 120 ms late now, the busy synthetic funk band's moves
       are mostly passed up (8 of 10, was 3), and the corpus AUC fell from ≥ 0.98 to ≥ 0.95 mean: the
       series waits for a note's harmonics to last. Real word-timed answers sit at −2 ms median (was +29).
     - Left: KuGou answers whose words tile the line (Vitamin R) still missed; pads and string chords; the
       real-song set is 22 songs, so the thresholds are tuned on little.
3. **Sources:** survey all 17 on a sample of the user's library (the feature-e2e `lyrics-services` section exists
   and is opt-in): hit rate, sync level, latency and junk. Then choose the default set with numbers.
