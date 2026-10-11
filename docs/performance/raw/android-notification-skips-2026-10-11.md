# Android notification skips — October 11, 2026

Arch host, Android 14 x86_64 `nori-e2e` emulator, local Navidrome fixture account,
Long Album (MP3 and FLAC). Eight successive `cmd media_session dispatch next`
commands while nori was in the background. Times come from device epoch logcat;
the new-song event is the engine's event, not a microphone measurement.

The previous Android path classified every media3 seek as an explicit song jump.
With fades off, that imposed `ToSong`'s 120 ms minimum fade before each skip.
Next and previous now retain their controller-selected index and use Rust's
existing `Skip` policy. Explicit song selections retain their song-jump policy.
User-configured fades still apply.

| Interval | Before | After, warm run |
| --- | --- | --- |
| Shell dispatch marker to engine new-song event | 130–182 ms | 20–144 ms |
| JNI target command to engine new-song event | 122–141 ms | 4–57 ms |

The first post-install run overlapped startup and workspace compilation, taking
66–471 ms overall. It is not a steady-state comparison. Even there, track release
followed the JNI command immediately instead of waiting 120 ms. The warm run
still showed variable Android main-thread dispatch latency, so the engine-side
numbers isolate the removed delay; these measurements do not prove all remaining
notification or physical-device latency is eliminated.

The virtual-clock `indexed_skip_is_prompt` regression reads the card's samples
and requires the target song within 80 ms with fades off. It failed with the old
song-jump path and passes with the skip path. Workspace tests, Android build,
smoke (34/34), notification checks (6/6), and audio (33/33) passed.

Local diagnostics: `build/indexed-skip-{before,after}.log`,
`build/notification-next-after-idle.log`, `build/notification-next-trace.log`,
`build/notification-skip-{workspace,apk,smoke,device,audio,clippy}.log`.
