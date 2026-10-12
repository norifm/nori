# Android stream reads, October 12, 2026

Applies to Android 0.6.0-beta.3. Arch Linux, Android 14 x86_64 emulator, local Navidrome,
MP3 fixture `Long Track 01`, a proxy delivering 8 KiB chunks at 32 KiB/s. AutoMix,
offload, and autofill disabled. The app's stream cache was cleared before each run.

`RustBody.read` filled its entire 256 KiB JNI buffer before returning available bytes.
The Rust loader already owns read-ahead, so this added a second wait on slow connections.
A read now returns the bytes the Android data source supplies, using the same reusable buffer.

| Read behavior | Cold start to more than one second of decoded PCM reaching AudioTrack |
| --- | ---: |
| Fill the JNI buffer | 18,493 ms |
| Return available bytes | 10,635 ms |

Each row is one run. The measurement waits for AudioTrack to start, more than 176,400 sink
bytes, and the engine's playing state; opening AudioTrack alone does not establish sustained
playback. This measures the cold Android streaming path, not a complete remote handoff.
The remaining delay includes the Rust loader's 256 KiB readiness threshold.

The Kotlin regression supplies a first packet and fails if the bridge requests a second packet
before returning the first. It failed with buffer filling and passed with one data-source read.
All core JVM tests, the debug APK, workspace Rust tests, smoke (34 checks), full audio
(33 checks), and notification/remote device checks (35 checks) passed.
