---
title: Volume leveling
description: Evening out the volume between songs and albums.
sidebar:
  order: 5
---

Applies to nori 0.7.0 and later on every client. Before 0.7.0 this was **Even out volume**, with a
pre-amp, a loudness target, a boost cap and a level for songs without tags; nori 0.7.0 moves those
settings over once, the first time it starts (any leveling becomes Automatic, the target the nearest
level).

**Volume leveling** reads the ReplayGain tags your files carry (Navidrome hands R128 tags over as
ReplayGain, so those work too):

| Choice | What it does |
|---|---|
| Off | every song as it was mastered |
| Per song | every song at the same loudness |
| Per album | each album at one level, keeping its quiet and loud songs as they are |
| Automatic (the default) | per album while an album plays in order, per song in shuffle or a mix |

**Level** sets how loud that is: Quiet (−23 LUFS, broadcast), Normal (−18 LUFS, ReplayGain's own, the
default) or Loud (−14 LUFS, as streaming services play).

Songs without tags play at the loudness AutoMix measured, once it has; until then, at −6 dB. Quiet songs
are turned up by up to 6 dB with the limiter behind them, so nothing ever clips.

Turning a song down is only a volume change, so [offload](offload.md) stays on. Turning one up needs the
samples, so that song plays on the CPU.

**Volume boost**, next to it, turns everything up (up to 12 dB) with the limiter behind it.
