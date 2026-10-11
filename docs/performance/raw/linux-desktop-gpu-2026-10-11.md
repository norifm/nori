# Linux desktop GPU investigation, October 11, 2026

The retained-layer repaint regression reproduces unnecessary full-layer work
in issue #33. The track-change measurements do not demonstrate an unbounded
memory leak after the cover ownership fixes.

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

## Native retained-layer rendering

The Linux compositor keeps each layer's GPU surface between paints. Its Skia
renderer nevertheless disabled partial rendering and reported a new back buffer
on every paint. The fix enables partial rendering for retained OpenGL layers and
reports buffer age one only when the surface survived the previous paint. New
and resized surfaces still repaint completely.

The native GPU regression changes an 8 × 8 control in a 72 × 64 layer, asserts
a bounded dirty region, then checks every pixel in the layer. It fails before
the change because no repaint region is tracked. After the change, Skia reports
1.39% repainted, and the surrounding pixels retain their expected colors. The
existing resize, cover ownership, mosaic and blur checks also pass.

Fullscreen playback of Long Track 04 at 1280 × 820, directly on NVIDIA EGL with
DPMS on and a private PulseAudio sink, averaged 12.15% of one CPU core before
and 2.75% afterward over 20 seconds. These are short process CPU measurements,
not a frame-rate or battery measurement. The after run used its own fixture
account to exclude commands from concurrent device checks. Its average RSS was
134618 KiB; differences in startup caches prevent interpreting that as a heap
reduction. The desktop window remained offscreen without taking focus.

Artifacts: build/{desktop-retained-before.log,desktop-retained-after.log,
desktop-retained-damage.log}, build/linux-desktop-native/{playback-cpu.txt,
playback-retained-isolated-cpu.txt}.
