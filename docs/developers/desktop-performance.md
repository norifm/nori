# Desktop profiling

Applies to the nori 0.7 desktop client (`crates/desktop`) on Linux and macOS.

## Before measuring

- Measure an optimized build with symbols. Debug builds, and presentation timings on Xvfb, do not
  represent a physical display.
- Run automated clicks on Xvfb with a private audio sink, as [Testing](testing.md) describes.
- On a physical display, check that `xset q` says `Monitor is On` before timing: with the monitors off,
  NVIDIA presentation waits about a second per frame.
- To profile a real library, use a private copy of the database and the cover and music caches. Keep
  the copy private: the database holds credentials. A test process must not request provider (`ext-`)
  songs nobody asked to play.

## CPU

```sh
cargo rustc -j4 --release -p nori-desktop -- \
  -C debuginfo=1 -C strip=none -C force-frame-pointers=yes
perf record -e cycles:u -F 99 -g --call-graph dwarf,16384 -p "$NORI_PID" \
  -o build/desktop-cpu.data -- sleep 60
perf report -i build/desktop-cpu.data --stdio --no-children --sort comm,dso,symbol
perf script -i build/desktop-cpu.data | stackcollapse-perf.pl | flamegraph.pl \
  > build/desktop-cpu.svg
```

- The last command uses Brendan Gregg's FlameGraph scripts.
- The `cargo rustc` flags add frame pointers to the desktop crate only. Dependencies need a separate
  `RUSTFLAGS` build.
- NVIDIA's proprietary libraries have incomplete symbols and unwinding: read the flat samples beside
  the call graph.

## Memory

- RSS: `/proc/$NORI_PID/smaps_rollup`, and `smaps` grouped by mapping. RSS includes code, shared
  libraries, driver mappings and allocator slack; it is not the live Rust heap. Anonymous resident
  memory compares more steadily between builds.
- Live heap: run a separate process with jemalloc's `LD_PRELOAD` and
  `MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:17`, and dump it with `mallctl("prof.dump")`
  while it runs (an exit dump misses what shutdown frees). `jeprof --collapsed --inuse_space BINARY DUMP`
  makes stacks for `flamegraph.pl`. Sampling at 128 KiB gives estimates, and branches overlap, so do
  not add them up.

## GPU

- RenderDoc's Vulkan capture and its `EventGPUDuration` counters time each draw and copy. Their sum
  leaves out presentation and some synchronization.
- For frame delays, time image acquisition, painting, encoding, submission and presentation apart.

## What the client keeps to

| rule | where, and its test |
|---|---|
| Only what is on screen holds covers: Home shelves and Top Picks cards in the viewport, grid rows in view, queue rows only while the queue is open. Offscreen items would evict visible covers and download them again. | `crates/desktop-ui/ui/app.slint`; `home_keeps_pick_covers`, `album_views_release_offscreen_covers` |
| Decoded covers live in the desktop's own cache: 32 MiB of small covers, 24 MiB of large ones. | `SMALL_BYTES`, `LARGE_BYTES` in `crates/desktop/src/app.rs`; `cover_bursts_keep_the_cache_bounded` |
| A hidden layer, or a page under the opaque fullscreen player, does not draw. | `fullscreen_does_not_draw_the_hidden_page` |
| Linux draws every layer through one Skia OpenGL context, which owns its cover textures. Borrowed Vulkan images caused NVIDIA GPU faults. macOS uses wgpu with the `MemoryUsage` hint. Each Skia context has a 16 MiB resource cache. | `crates/desktop/src/compositor.rs`, `compositor/gl.rs`; `layers_resize_without_losing_pixels_or_blur` (X11, `--ignored`), `cover_pixels_survive_async_layers_and_mosaics` (macOS) |
| On Linux with glibc, the mmap threshold is 256 KiB and the trim threshold 128 KiB, so freed cover and analysis buffers go back to the system. | `crates/desktop/src/main.rs` |
| Redraws coalesce at the monitor's refresh interval. With nothing to redraw there is no frame timer. Composited X11 presents Immediate; everything else uses AutoVsync. | `redraw_bursts_wait_for_the_next_frame` |
| A window resize changes the layout at once; only opening and closing a panel animates. | `resizing_does_not_animate_layout` |
| The seek bar follows playback in small steps without animating a whole layer. Large seeks and incoming volume changes animate. | `volume_and_seek_animate_between_updates` |
| Mouse wheels scroll with deceleration, as Slint's own winit backend does. | `wheel_scroll_moves_over_time` |
