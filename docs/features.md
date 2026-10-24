# Planned features

Internal note, not published. What to build next, in rough order. Delete a line once it ships.

## Next

**Queue**
- Clear the upcoming songs (Android).
- Save the queue as a playlist.
- Total and remaining time in the queue header.

**Playlists**
- Rename, reorder, remove songs.
- Add to several playlists at once, with an "already in this playlist" hint.

**Library and search**
- Filter chips by type in search: songs, albums, artists, playlists.
- "Appears on" for artists.
- Sync the library on app open when the last sync is older than N hours, Wi-Fi only.

**Player**
- Sleep timer: end of queue, and a fade out.
- Output sample rate and bit depth on the output button.

**Sharing and radio**
- Shares: expiry, description, list, delete.
- Edit a radio station.
- Share lyrics as text.

**Downloads**
- Pause and resume.
- Stop when storage runs low.
- Store covers with the download, so the image cache can't evict them.
- Auto-download rules: keep a playlist or the favorites downloaded.
- Keep a song that is already in the stream cache as a download, without fetching it again.

**AutoMix**
- Analyze old downloads while charging.

**Settings**
- Backup and restore with a password (plugin settings go into the same backup).

**Android**
- Launcher shortcuts for playlists.
- Check on a phone, frame by frame: the sleeve's soft bottom through swipes, sheet pulls and panel
  changes; moving covers coming and going; sung lyrics (word-timed, duets, backing vocals); a skip to a
  song whose cover isn't loaded.

## Plugins

Plan: `docs/research/plugins.md`. These go there instead of becoming settings:
- casting: Snapcast, UPnP/DLNA, Chromecast;
- headphone control (ANC, battery) over Bluetooth;
- YouTube Music as a provider plugin;
- the removed sound knobs (crossfade curves, crossfeed levels, compressor, expander, loudness
  compensation) and the hardcoded values, as overrides and sound-stage plugins;
- autoplay when headphones connect, headset click mapping, audio focus behavior;
- lyrics over Bluetooth (the line in the title field);
- automation intents for Tasker-style apps.

## AutoMix, later

- A mix preview: play only the last and first 20 s of each pair in the queue, to judge many transitions
  quickly; and a library survey of how many songs get a usable beat grid.
- A "Best" quality step: the full Beat This! model, a heavy whole-song pass while charging on Wi-Fi.
  Offer it only once it is measured to help on real music.
- Transition choice computed ahead when the next song is known, cached per pair.
- Stems around the transition ("Studio"): vocals, drums, bass, the rest, for the last and first 45 s.
  Vocal ducking and instrumental loops by stem; fall back to today's mix when not ready.
- An optional vocal stutter style, off by default.

## Later

When everything above is done; each behind a switch:
- FFmpeg decoding for DSD, APE, WavPack and WMA.
- USB exclusive output driver.
- DSD output.
- 32-bit integer output.
- Web: lock-screen playback in iPhone Safari.

## Not planned

Other server types, local files, Wear OS, Android TV, audiobook mode, star ratings, Genius, a grid/list
toggle, the A-Z rail.
