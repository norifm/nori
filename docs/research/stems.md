# Stems for AutoMix

Measured November 2026 on 100 songs of the owner's library (rock, from the desktop app's stream cache).
HT-Demucs separated the vocals as the reference; Open-Unmix UMX-L (vocals only) stood in for what a phone
could run. Both models were exported to ONNX and timed through tract 0.23, nori's runtime for Beat This!.

## Does it help

Every ordered pair of 98 songs, both styles (18,309 transitions inside the separated edges), planned by nori's
analysis and planner, each deck's level taken through the mixer, against HT-Demucs' vocal level at that point
of each song. Eight instrumental songs (vocals 45 to 71 dB down) count as never singing. Stems here hand the
voice over at the bass swap (the middle of a fade): the outgoing vocal out, the incoming one in.

| | Smooth | DJ |
|---|---|---|
| two voices within 10 dB for 1 s or more | 11 % | 11 % |
| for 3 s or more | 2 % | 5 % |
| beat-matched blends with 1 s or more | 22 % | 35 % (1.8 s on average) |
| the swap rule cuts 1 s or more of a line sung alone | 47 % | 40 % |

- The overlaps are mostly DJ's long beat-matched blends (up to 39 s), where the incoming song starts singing
  before the swap. Fades rarely overlap: the outgoing song has usually stopped or the incoming one is in its
  intro (a first sample of ten pairs had none).
- On the 12 worst transitions, Open-Unmix cuts the overlap from 3.6 to 11 s down to 0 to 1.8 s; the removed
  vocal is 10 to 52 dB down (HT-Demucs: nearly gone).
- A fixed handoff costs more than it gives: it removes singing that had nothing to clash with in about 4 of
  10 transitions. Stems would have to duck a voice only while the other one sings, and mostly for DJ blends.

Clips of these transitions were rendered for listening (Today, Open-Unmix, HT-Demucs).

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
song's median voice level.

On 89 sung songs the voice-band share the planner used to tell a sung end or opening was near chance (it
missed 25 of 61 sung endings and 19 of 29 sung openings, and took 30 of 60 unsung openings as sung). So every
song now gets where its singing starts (the first two bars at that level) and ends from the voice curve, for
any song whose loudest bar reaches the curve's floor, and the planner prices each second both songs sing at
once. Within two bars, the start is right on 62 % of songs (21 % early, 17 % late) and the end on 60 % (31 %
late, 9 % early); a share of the loud bars instead of the median does no better. Seven of the eight
instrumental songs still read as sung, which only keeps their mixes careful. Over every pair, DJ mixes with
both voices for 3 s or more fell from 10.3 % to 5.0 % (beat-matched ones from 31 % to 9.6 %), and its loops
from 61 to 34, the rest having looped sung bars.

Synced lyrics say it better still. Once lyrics shown for a song are heard to fit its voice (or the listener set
their timing), their first and last sung lines replace both ends (`store::put_sung`, kept per server). Only
lyrics already looked up for a screen count; AutoMix fetches none.

## Computing them once

Separation depends on the song alone, not on the pair, so it can be done once per song, like the analysis, and
mixing with it is only a gain on the vocal stem. A song needs only its edges: the first and last 45 s (a DJ
blend reaches 40 s). With UMX-L that is about 3 s of one emulator core per song. Kept as audio, a 16-bit
stereo vocal stem of 90 s is 16 MB; a mask instead (64 bands every 46 ms, 8 bits) is about 125 KB, applied
when the mix plays. Separating while the next song is queued, as AutoMix already measures songs coming up,
would need no store at all.
