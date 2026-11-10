# Stems for AutoMix

Measured November 2026 on the host (Apple M4 Pro, PyTorch 2.15 on the CPU), separating 30 s of 44.1 kHz stereo.
Quality figures are the models' published MUSDB18-HQ vocal SDR; the only songs in `.dev/music` are synthetic
tones, so they could not judge separation.

| model | weights | one core | eight cores | peak memory | vocal SDR |
|---|---|---|---|---|---|
| HT-Demucs (4 stems) | 42 M params, 168 MB | 14.4 s (2.1x real time) | 6.0 s | 1.8 GB | about 9 dB |
| Open-Unmix UMX-L (vocals only) | 28 M params, 113 MB | 0.8 s (35x real time) | | 1.0 GB | about 7 dB |

What this means for a phone, whose big core is about half as fast at this and whose analysis runs on one
background core:

- Whole songs are out of reach: HT-Demucs would spend 3 to 5 minutes of a core on each one.
- The transition window alone is: the outgoing song's last 45 s and the incoming song's first 45 s. UMX-L would
  take about 5 s per mix and HT-Demucs about a minute and a half, computed while the outgoing song plays.
- The cost is the model, not the time: a 110 to 170 MB download, and hundreds of MB of memory while it runs. It
  would be an opt-in setting with its model fetched on first use, off by default.
- Running in nori means a Rust inference crate (tract runs ONNX on every client, including the web). UMX-L is
  STFT, LSTM and masks, all of which ONNX can express; HT-Demucs mixes a spectral and a waveform branch with a
  transformer between them and has to be exported with its STFT outside the model.

What it would buy over today's analysis: the voice curve already finds where the singing ends within a beat on
the evaluation songs. Stems would add mixing by part: the outgoing vocal out while its beat carries on, an
incoming acapella over the outgoing instrumental.
