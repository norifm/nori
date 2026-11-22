# Desktop profiling

This page shows how to measure the desktop app's CPU, memory and GPU on Linux and macOS, and lists the
cost rules its code keeps. For the engine and Android, see [Performance](perf.md).

## Before you measure

- Measure an optimized build with symbols. Debug builds do not represent a real display, and neither do
  presentation timings on Xvfb.
- Run automated clicks on Xvfb with a private audio sink, as [Testing](testing.md) describes.
- On a physical display, check that `xset q` says `Monitor is On` before you time anything. With the
  monitors off, NVIDIA's presentation waits about a second per frame.
- To profile a real library, use a private copy of the database and of the cover and music caches. Keep
  the copy private, because the database holds credentials.
- A test process must not request provider (`ext-`) songs that nobody asked to play.

## Profile the CPU on Linux

Build with frame pointers and symbols, record the running app with `perf`, and make a flame graph:

```sh
cargo rustc -j4 --release -p nori-desktop -- \
  -C debuginfo=1 -C strip=none -C force-frame-pointers=yes
perf record -e cycles:u -F 99 -g --call-graph dwarf,16384 -p "$NORI_PID" \
  -o build/desktop-cpu.data -- sleep 60
perf report -i build/desktop-cpu.data --stdio --no-children --sort comm,dso,symbol
perf script -i build/desktop-cpu.data | stackcollapse-perf.pl | flamegraph.pl \
  > build/desktop-cpu.svg
```

- `NORI_PID` is the process id of the running `nori-desktop`.
- The last command needs Brendan Gregg's FlameGraph scripts.
- The `cargo rustc` flags add frame pointers to the desktop crate only. For the dependencies, build
  again with the flags in `RUSTFLAGS`.
- NVIDIA's proprietary libraries have incomplete symbols and unwinding. Read the flat samples beside the
  call graph.

## Profile the CPU on macOS

On macOS, `tools/flame.py` records the whole process with samply. `tools/desktop-tour.py` gives it the
same workload each time. It needs a build with `--features test-control`, and a short socket path under
`/tmp`.

```sh
cargo build -j4 --profile profiling -p nori-desktop --features test-control
python3 tools/flame.py -o build/flame/desktop --for 60 -- \
  target/profiling/nori-desktop --data "$DATA" --control-socket /tmp/nori-desk.sock &
python3 tools/desktop-tour.py /tmp/nori-desk.sock 1 0.5 <album id>
python3 tools/flame.py --profile build/flame/desktop/profile.json.gz --per-second
python3 tools/flame.py --profile build/flame/desktop/profile.json.gz --since 15 --until 55 --thread nori-desktop
```

`tools/desktop-tour.py SOCKET [rounds] [pause] [album id]` walks every page, panel and the full player.
It does that `rounds` times (default 3), with `pause` seconds on each (default 0.5). With an album id, it
plays that album first, so you profile while music plays. [Performance](perf.md) describes the other
`tools/flame.py` options.

### What a normal profile looks like

- While a song plays, the window draws one frame per step of the seek bar. A step is a quarter pixel of
  the bar, or the next second of its times (`nori_look::motion::wait_ms`). On a 2x display, a
  four-minute song steps about 9 times a second.
- Each step repaints only the bar and its playhead, about 2 % of the player.
- Idle, the window costs under 1 ms a second.
- mdns-sd checks the network interfaces once a minute.

## Measure memory

- **RSS.** Read `/proc/$NORI_PID/smaps_rollup`, and `smaps` grouped by mapping. RSS includes code, shared
  libraries, driver mappings and allocator slack, so it is not the live Rust heap. Anonymous resident
  memory compares more steadily between builds.
- **Live heap.** Run a separate process with jemalloc through `LD_PRELOAD`, and
  `MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:17`. Dump the heap with `mallctl("prof.dump")`
  while it runs, because a dump at exit misses what shutdown frees.
- `jeprof --collapsed --inuse_space BINARY DUMP` makes stacks for `flamegraph.pl`. Sampling at 128 KiB
  gives estimates, and the branches overlap, so do not add them up.

## Measure the GPU

- RenderDoc's Vulkan capture and its `EventGPUDuration` counters time each draw and copy. Their sum
  leaves out presentation and some synchronization.
- For frame delays, time image acquisition, painting, encoding, submission and presentation apart.

## Rules the desktop app keeps

The desktop app keeps these cost rules. Tests in `crates/desktop` guard most of them:

- **Covers on screen only.** Home shelves, Top Picks cards and grid rows hold covers only in view, and
  queue rows only while the queue is open (`crates/desktop-ui/ui/app.slint`). Offscreen items would evict
  visible covers and download them again.
- **A bounded cover cache.** Decoded covers live in the app's own cache: 32 MiB of small covers and 24 MiB
  of large ones (`SMALL_BYTES`, `LARGE_BYTES` in `crates/desktop/src/app.rs`).
- **Hidden pages do not draw.** Neither does a page under the opaque full player.
- **One compositor context.** Linux draws every layer through one Skia OpenGL context, which owns its
  cover textures. Borrowed Vulkan images caused NVIDIA GPU faults. macOS uses wgpu with the
  `MemoryUsage` hint. Each Skia context has a 16 MiB resource cache (`compositor.rs`, `compositor/gl.rs`).
- **Freed memory goes back.** On Linux with glibc, the mmap threshold is 256 KiB and the trim threshold
  128 KiB (`crates/desktop/src/main.rs`).
- **One frame per change.** An animation that can be instant is turned off with `enabled`. An animation
  of 0 ms still reads the animation clock and draws again on the next tick. The composite is
  one render pass, and uniforms are written only when they change.
- **Retained layers.** A layer keeps its texture between frames and repaints only what changed
  (`SkiaWGPU30Renderer::retained` on macOS, the OpenGL layers on Linux).
- **Frame pacing.** Redraws coalesce at the monitor's refresh interval, and with nothing to redraw there
  is no frame timer. Composited X11 presents with Immediate, and everything else with AutoVsync.
- **Little animation.** A window resize changes the layout at once, and only opening and closing a panel
  animates. The seek bar follows playback in small steps without animating a layer. Large seeks,
  incoming volume changes and mouse-wheel deceleration animate.
