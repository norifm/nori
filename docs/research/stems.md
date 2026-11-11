# Stems for AutoMix

Measured November 2026 on 100 songs of the owner's library (rock, from the desktop app's stream cache).
HT-Demucs separated the vocals as the reference; Open-Unmix UMX-L (vocals only) stood in for what a phone
could run. Both models were exported to ONNX and timed through tract 0.23, nori's runtime for Beat This!.

## Does it help

Ten pairs of songs, planned and mixed by nori's own analysis, planner and mixer (both styles, 20 transitions),
then mixed again with stems: one voice at a time, the outgoing vocal out and the incoming one in at the bass
swap.

- With nori's plans, stems change nothing. No pair had both songs singing in the mix: nori fades in the last
  5 to 14 s (the rock tempos are not steady enough to beat-match), where the outgoing song has stopped singing
  or the incoming one is still in its intro. Stems only cut 0.3 to 1.7 s of a line short in 4 transitions.
- Moved into both songs' singing (4 pairs, as a control), the plain mix has two voices competing for 1.7 to
  2.5 s per transition. With stems: 0 to 0.8 s. The removed vocal is 13 to 55 dB down, but 1 to 2 s of each
  song's line is cut at the handoff.

Stems would pay only if AutoMix mixed long over singing, which it avoids by design (and on this library
cannot do, without a steady beat).

## What it costs

Single core, on the arm64 emulator (an M4 Pro core running tract's NEON kernels, the code a phone runs):

| model | weights | CPU per 10 s of audio | a transition (45 s + 45 s) | peak memory |
|---|---|---|---|---|
| UMX-L, vocals | 113 MB | 0.33 s | about 3 s | 244 MB |
| HT-Demucs, 4 stems | 174 MB | 7.6 s | about 90 s | 811 MB |
| nori playing, for scale | | | 6 s per 3.5 min song, screen on (2.8 % of a core) | |

A phone core is slower: a flagship's big core about 1.5x, the mid and small cores 3 to 10x. In energy, about 10
to 15 J per transition for UMX-L (0.4 % of a 17 Wh battery an hour, about a tenth more than playback with the
screen off) and about 450 J for HT-Demucs (around 12 % an hour, several times playback itself). HT-Demucs'
800 MB would also get a background app killed on most phones.

## What came out of it

The reference showed that the analysis' end-of-singing measure, tuned on synthetic songs, ended early on 21 of
40 real ones. Loud bands keep the voice curve high between lines. It now ends with the last bar at 0.7x the
song's median voice level, and only for an ending the analysis already takes as sung. Of the 22 such songs, it
is right (within two bars) on 15, late on 5, early on 2.
