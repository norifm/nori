# WIP handoff for continuation on Arch Linux

Updated October 11, 2026. The owner explicitly requested this new handoff and a WIP
commit to move work from the Mac to Arch. This replaces the purpose of the old,
deleted handoff; do not restore that obsolete file. Read AGENTS.md first.

## Owner's scope and preferences

- Continue the work, test it, commit it, and push it. Refresh GitHub issues assigned
  to **filipton**, especially milestone **0.6.0**, rather than relying on this
  snapshot. Work from easiest to hardest and combine related fixes.
- README and website issues belong to **sosjalapeno**, not this batch. Current
  examples are #37 and #51. Do not take unassigned issues automatically.
- Use a few coherent commits, conventional one-line subjects under 72 characters,
  no body, attribution, or trailers. Add `(Closes #N)` only for completed,
  verified fixes. This WIP intentionally closes nothing.
- Pull/fetch before continuing. Other owner work can advance master during a
  session: beta.3 was released while this batch was in progress. Preserve it.
- Use American English; this is already in AGENTS.md.
- Performance matters: lower RAM, CPU, GPU work, build/test wait time, and smooth
  animations and resizing. Measure costs with traces/flamegraphs and actual
  output. Keep blur, smoothness, and low RAM together; removing blur is not an
  acceptable final tradeoff. Do not declare #33 fixed based only on the Mac.
- Linux machine of interest is the owner's SSH PC, **X11**. Normally use a
  **1920x1080 Xvfb** display and a private audio sink, click the app yourself,
  and leave the owner's checkout, running app, focus, and audio alone. The owner
  also authorized real-display testing if Xvfb cannot reproduce a problem.
- Multiple Android emulators are allowed. Lease devices and serialize heavy
  builds using the existing tools; never exceed four Cargo jobs. Bazel pilot,
  faster incremental Android builds, and device leasing already exist.
- The owner will verify the physical iPod volume fix later; no device was available.
- Desktop Mixing text must match Android: between the times under the seek bar.
  That change is already committed (e084a258). The owner questioned whether the
  miniplayer needs a Mixing label; avoid redundant clutter.
- Check Android scrobbling too. An opt-in `scrobble` feature section exists;
  do not assume the latest playback changes have passed it.
- The two docs branches were already incorporated without attribution, their
  branches/PRs removed. Do not redo that work.

## Latest jam request: highest-priority unfinished work

The owner tested **0.6.0-beta.3**, with desktop host and mobile guest using its
speaker. Initial listening is still slightly out of sync. Sometimes the guest
plays the beginning for about a second, then jumps to the host's place but stays
slightly off. Pausing/resuming the guest fixes it. **No audio should start before
the guest is synchronized.** Audio also sometimes cuts when out of sync.

Pausing the host during an AutoMix transition and resuming can leave the guest
silent until the next song. Lyrics timing is also odd; investigate the audible
position/tempo through joins, corrections, and mix pause/resume.

After the playback fixes, implement these requested settings/UI changes:

- Merge the separate global remote-control and jam switches into **one switch,
  enabled by default**. Preserve intended stored-setting migration and update
  all clients; defaults are intentionally changing.
- Make **Let guests listen along** a **per-room** setting, default **on**, instead
  of a global settings row. Core already has `Remote::jam_along` and Hosted.along.
- Add a per-room **remote songs** permission, default **off**. Likely means guest
  provider/ext-track requests; confirm the model while inspecting the code.
  Do not prefetch provider songs the owner did not request.
- Put a compact settings button near Invite. Do not make the devices/jam page
  taller. Invite/settings should replace the current drawer's content with a
  back action, instead of stacking another drawer on top.
- Reconsider redundant playing-device labels when already controlling that
  device; preserve useful device identity in the picker.

None of those new settings/drawer changes is implemented in this WIP.

Entry points: `crates/settings/src/settings.rs` (remote_control, jam, jam_along),
`crates/core/src/remote.rs` (Hosted, JamView, jam_open, jam_along, request handling),
`crates/remote/src/jam.rs` (wire state), `crates/host/src/session.rs`,
`core/.../remote/Remotes.kt`, `app/.../ui/RemoteScreens.kt`, `app/.../ui/Jam.kt`,
`app/.../vm/SettingsPages.kt`, desktop settings/app, CLI settings, and iPod screens.
JamControls currently opens InviteSheet inside the existing sheet.

## What this WIP contains

### Download grouping (#54) and Android Auto cache work (#53)

- Shared Rust DownloadedLibrary groups completed songs by nonempty album ID;
  loose songs stay separate. Same album titles with different IDs stay distinct.
  Saved album detail contains only finished downloads, sorted by disc/track,
  with Downloads queue origin.
- Android Downloads uses album cards and a local downloaded-album route.
  CLI, iPod, and Android Auto use the same grouping/detail rules. Root play and
  shuffle still cover all saved songs. Local detail does not fetch missing tracks.
- Android Auto cached browse pages refresh when stale and fall back offline.
- Fixed a Favorites race: a read started before a star mutation could restore
  the evicted stale cache afterward. Cache writes now validate a revision under
  the DB lock. The regression failed before the fix and passes afterward.
- Android Auto wrong Home covers are **not reproduced**: six cover URIs/bitmap
  hashes were distinct. Do not close #53 based on the cache fixes alone.
- New opt-in `download-albums` feature section exercises partial downloads,
  native MediaBrowser folders, actual offline UI playback, and Favorites.

### Jam startup and Android timeline

- `crates/remote/src/clock.rs`: a faster exchange following one delayed startup
  exchange no longer discards the startup clock burst. Jump detection includes
  both exchanges' round-trip uncertainty. A two-direction delayed-first-response
  regression failed before and passes after; genuine clock-restart test passes.
- `PlaybackService.guestQueue` now calls `EnginePlayer.showFollowingQueue` rather
  than `setMediaItems`, which issued another native seek to zero after Rust had
  already aligned the guest. Actual pre-fix trace showed Start at 4697 ms followed
  by `rust player: to song 1 at 0 ms`.
- Initial device rerun exposed a crash in that new display-only path:
  `IllegalArgumentException: Empty playlist only allowed in STATE_IDLE or STATE_ENDED`.
  Cause: cached QueueTimeline was not invalidated after replacing items.
  **Now fixed** by clearing `timeline` in replaceItems. The APK rebuilt, but this
  last cache fix has **not been retested on the device**.
- `crates/engine/tests/along.rs::guests_resume_a_paused_mix` is an intentional,
  **currently failing regression**. It joins before a mix, pauses the host during
  the active mix, resumes, checks audible samples before the next song, and
  compares actual audio phase. It covers streamed/downloaded sources and deep
  output buffering, but fails at the first streamed/unbuffered case, so later
  cases have not run yet. Fix the cause; do not remove/weaken it just to pass.

Reproduce:

```sh
python3 tools/with-resource.py build cargo test -j4 -p nori-engine \
  --test engine guests_resume_a_paused_mix -- --nocapture
```

Latest failure: `hold=0, downloaded=false`: **9 readings, 6.0 ms average,
14.4 ms worst** versus required 10 readings, <=2.5 ms mean, <=5 ms worst.
The resumed guest was audible; this run demonstrates timing trouble rather than
the owner's complete silence. Trace: Pause here 20585 ms, host 20500 ms; resumed
Start at 21382.75 ms, transition late hold 1382 ms in. Investigate overlap
reconstruction and resume alignment; distinguish missing readings from drift.

Existing relevant tests: guests_play_the_host_mix, a_guest_joins_mid_mix,
a_pause_cancels_a_guest_starting_a_mix. Use `behind_ms` on actual output during
mixing; `in_step`/Jam::gaps skips mixing and cannot prove overlap alignment.
Master already contains the owner's 634f22d9 disk-source priming fix.

### Device tooling, partly finished

- UiDump Java/helper can perform native accessibility clicks on the matching
  node/clickable parent. feature-e2e uses these instead of stale coordinates.
  Build succeeded; latest full run exercised most screens successfully.
- TestBridge now accepts an optional request ID and includes it in its reply.
  Legacy calls still receive plain replies. **tools/app.sh is not yet updated**
  to send/filter IDs. Its log-line-count matching can accept another request's
  reply or fail when logcat rotates. Finish this before trusting flaky debug
  command acknowledgments. Use proper device-shell quoting (e.g. shlex.join),
  and test concurrent read-only state/remote-view calls.
- Do not edit a running Bash script: Bash can read later commands at old byte
  offsets. This caused trouble during earlier iterations. Wait/stop first.

## Verification at the handoff

This is **WIP, not a passing release**. Owner requested a checkpoint despite
unfinished checks and the newly failing regression.

- Full `cargo test -j4 --workspace`: passed after download/cache/clock changes,
  **before** adding guests_resume_a_paused_mix. With that test, workspace is red.
- Core library: 133 tests passed including cache-race regression.
- Remote: 19 tests passed, including delayed clock burst and real restart cases.
- Download grouping/CLI/local album regressions passed earlier.
- Android APK: latest build passed, including timeline cache fix and request-ID
  receiver. Device has the **previous APK** unless explicitly reinstalled.
- iPod build passed, `build/ios/nori-ipod-0.6.0-beta.3.ipa`; not hardware tested.
- Download-albums device section: 18/18 passed before the latest jam edits.
- Full audio suite: 33/33 passed before latest clock/timeline changes.
- Smoke: 34/34 passed before latest Favorites/timeline changes.
- Latest full feature run passed early sections, native Favorites, and remote
  controls, then failed guest jam checks due to the timeline crash above. It was
  stopped at the owner's handoff request. No passing final full run exists.
- Clippy previously had baseline warnings only; repeat after finishing changes.
- `git diff --check` passed at handoff.

Diagnostic logs/artifacts are **gitignored, remain on the Mac, and will not arrive
on Arch via git**. Key paths under `build/crossplay/`:
`downloads-jam-final-workspace.log`, `downloads-all-final-feature.log`,
`downloads-jam-live.log`, `jam-paused-mix.log`, `jam-timeline-rebuild.log`,
`favorites-race-before.log`, `favorites-race-after.log`, `jam-clock-before.log`,
`jam-clock-after.log`, `downloads-device.log`, `downloads-full-audio.log`,
`downloads-clock-smoke.log`, `downloads-all-final-ipod.log`.
The essential failures are recorded above so those files are not prerequisites.

## Next steps on Arch

1. Fetch/pull, read AGENTS.md, inspect this WIP, and refresh assigned issues:
   `gh issue list --repo norifm/nori --assignee filipton --milestone 0.6.0 --state open`.
   Read each issue body and comments before implementing it.
2. Fix active-mix guest pause/resume and initial synchronization. Finish request
   correlation, rebuild/reinstall Android, rerun jam and download-albums checks.
   Investigate lyrics against audible position/tempo, not just the UI timer.
3. Implement the single enabled-by-default global control/jam switch and compact
   per-room settings/invite content. Cover shared behavior in Rust first and wire
   every client with the relevant screen. Stored format changes must be intentional.
4. Run workspace tests and builds, smoke, touched device sections, and full
   audio/feature suites for the completed batch. Test scrobbling explicitly.
5. Finish/review #54 and partial #53; only close fully verified issues. Continue
   easiest assigned issues, grouping related work such as #43 and #50.
6. Measure remaining Linux performance on Arch, including rapid live resize.

Open assigned 0.6.0 snapshot: #54 downloads; #53 Auto covers/Favorites/Downloads;
#50 lockscreen skip latency; #49 native Android crossplay output picker;
#43 shuffle during mix; #40 device-handoff delay; #33 Linux performance;
#29 iPod remote volume. #56 Discord RPC and #38 desktop release are currently
unassigned. Refresh this list: new issues may have arrived.

For #43, preliminary lead: shuffle changes playlist order around the heard
outgoing track while the pipeline may already read the incoming mix track.
`Pipeline::queue_changed`/`read_astray` can undo that incoming read. Write an
actual-audio regression before changing it; no fix has been made yet.

#40 has not reproduced a long stall: measured pick 106 ms, control 222 ms, audio
283 ms in one test. Preserve evidence and investigate the owner's real setup;
do not close it from that measurement. #29 fix was pushed but awaits hardware.

## Linux performance acceptance

Earlier commits reduced cover memory and shared GPU blur rendering; those are
already on master. #33 remains open after follow-up reports. Remaining complaints:
fast resize changes borders before content, copied gray background, elements
outside the viewport/disappearing, and sidebar appearing unexpectedly. Test
rapid corner/side dragging, not only slow scripted size steps. Compare frame
cadence and layout to Chrome. Keep blur and clipping correct during live resize.
Previous profiling still showed several expensive GPU blur layers with paint
p95 above 16.7 ms; remeasure on the actual Linux GPU before choosing a fix.

No horizontal shelf scrollbars: the owner explicitly rejected them. Keep smooth
mouse-wheel scrolling, useful horizontal carousel mouse interaction, and Show
All opening a scrollable grid. Top Picks collage seams must be invisible at all
scales, including edges and the fade boundary. Memory must stay bounded after
many skips; investigate anonymous heap growth with measurements rather than
assuming the earlier cover fix solved every allocation source.

The Slint vendor directory contains upstream renderer/windowing changes needed
for the Linux fixes; inspect existing changes before upgrading/removing it.
Build-performance documentation and Bazel's player pilot are already present;
do not replace Cargo/Gradle blindly or rerun expensive checks unnecessarily.
