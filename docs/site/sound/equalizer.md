---
title: Equalizer and effects
description: The graphic and parametric equalizer, presets, and the effects around it.
sidebar:
  order: 1
---

Applies to nori 0.6.3 and later. Every client has the equalizer and the effects. Before 0.6.3 the effects
had their own controls (bass boost, crossfeed presets, a virtualizer slider, the compressor's six knobs, a
noise gate and loudness compensation); nori 0.6.3 moves them over once, the first time it starts, as
below.

The equalizer runs in nori's own sound chain, so it sounds the same on every client and every output. A
change is heard at once, seamlessly, while the music plays: nothing stops and nothing is decoded again.

## Graphic or parametric

- **Graphic** (the default): 10 sliders, or 5, 15 or 31, from −12 to +12 dB.
- **Parametric**: any number of bands, each a peak, a low or high shelf, a low or high pass, a band pass
  or a notch, on both channels or one. Paste a preset from Equalizer APO, or a `GraphicEQ.txt`, to
  import it.

The pre-amp is set automatically so a boost cannot clip, and a look-ahead limiter catches what is left.
Built-in presets cover bass, treble, vocals and loudness, and your own setups can be saved as profiles.
**Bass boost** is one of the presets: a gentle low shelf at 100 Hz. A bass boost set before 0.6.3 is moved
onto the equalizer as that shelf, at the same strength.

## Effects

| Effect | What it does |
|---|---|
| Headphone sound | Off; Natural, which blends a little of each channel into the other as speakers do (bs2b crossfeed); or Wide, a bigger stage (the virtualizer) |
| Compressor | brings quiet passages up and loud ones down, tuned for Light (a little evening out), the Car (quiet passages stay above road noise) or the Night (loud peaks tamed for listening low) |
| Volume boost | up to 12 dB louder, kept clean by the limiter (with [volume leveling](replaygain-and-loudness.md)) |
| Balance, mono | left and right, or both channels as one |

From before 0.6.3: crossfeed on becomes Natural and the virtualizer Wide; the compressor's knobs become the
nearest of its three settings.

Any of these turns [offload](offload.md) off while it is on, since it needs the samples.

While the app is on screen, Android keeps the audio buffer short, so a band you move is heard straight
away; in the background the deep buffer that saves battery comes back. The desktop and terminal do the
same while their equalizer page is open.
