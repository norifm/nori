# Linux desktop GPU investigation, October 11, 2026

Issue #33 remains open. These measurements do not reproduce the owner's native
desktop jank or demonstrate an unbounded memory leak.

Host: Arch Linux, NVIDIA GeForce GTX 960, driver 580.178.04. Release desktop
with test-control, isolated Navidrome account and data directory. Interactive
measurements used a private 1920 × 1080 Xvfb display and VirtualGL 3.1.5's EGL
device backend. Playback used a private PulseAudio null sink through an explicit
ALSA Pulse plugin configuration; the default PipeWire ALSA hooks bypassed the
initial sink setting, so the initial playback run is excluded.

| Workload | Process RSS, KiB | Settled CPU |
| --- | ---: | ---: |
| Start of track-change run | 185864 | — |
| Maximum sampled over 100 distinct track titles | 200784 | — |
| Paused after track changes | 191940 | 0% |
| Paused after fullscreen resize | 202320 | 0% |
| Paused Home after resize | 251708 | 0% |
| Paused Home after repeating resize | 249992 | 0.67% over three seconds |

Each resize run delivered 240 programmatic size changes over eight seconds,
then restored 1280 × 820. Screenshots retained the expected final layout. This
does not test dragging a border under the owner's window manager. RSS includes
allocator and driver caches and is not a measurement of live application heap.

Fullscreen playback averaged 41.92% of one CPU core through VirtualGL. Its
framebuffer readback and NVIDIA synchronization make that unsuitable as a
native-desktop CPU result. An initial profile also showed driver waits during
the VirtualGL swap path. Do not infer a product CPU regression from this setup.

The existing GPU pixel regression failed after enlarging its window through
VirtualGL: the expanded region read back as transparent black. The same test
passed directly on NVIDIA EGL on DISPLAY=:0, with its override-redirect window
offscreen. The owner's active window stayed unchanged. No renderer fix was
made based on the VirtualGL failure.

Artifacts: build/linux-desktop-gpu/{track-memory.json,settled-paused.txt,
after-resize-idle.txt,after-home-resize-idle.txt,
after-home-resize-repeat-idle.txt,playing-pulse-memory.txt,
gpu-resize-test.log,gpu-resize-native-before.log,perf-report.txt}.
