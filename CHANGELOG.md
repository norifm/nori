# Changelog

What changed in each release, newest first. Sections are grouped from the commit log by
`tools/changelog.py` — `feat:` becomes Added, `fix:` becomes Fixed, and so on — then read over by
hand. Versions follow [semantic versioning](https://semver.org).

## [Unreleased]

## [0.7.1] - 2026-10-27

### Added

- Favorites in albums filter any order, as they do in songs
- Drop the Previous goes back a song setting
- Drop the Speed setting
- Drop the Pitch setting
- Drop the Skip silence setting
- Play and pause always fade over 150 ms
- Drop moving covers
- Desktop menus on a right press of the pill's song and any artist (fixes #119)
- Desktop backing vocals fill word by word as they are sung (fixes #120)
- Artist menus on android and the terminal, card menus on the web
- An equalizer curve you drag, with free parametric bands (#123)
- The desktop equalizer curve you drag, bands added on it
- The equalizer curve on the web, terminal and iPod (#125)
- Lyrics are marked with a speech bubble on every client
- Nightly nori builds that update themselves, and checks on every push
- Equalizer bands take typed values and pick their kind from a list (#128)
- The web as the desktop, playlist management, deletes ask first (#126)
- Right-click menus on the desktop's queue, search preview and artists list
- The web queue takes a song out by its row's button, its menu the song's own

### Changed

- Home rows read and page as the core says on every client

### Fixed

- Moving playback between devices no longer crashes the app
- A deeply nested lrclib lyrics file no longer crashes the app
- A refused favorite no longer freezes a device serving remotes
- A song with no audio on repeat no longer hangs playback
- The remote-control door no longer spins the cpu
- Changing the streamed music space can no longer freeze playback
- A request made while the player reconnects runs once
- A download that keeps restarting from zero now fails
- Fetching ahead no longer refetches songs that overflow the cache
- An empty beat model answer no longer repeats the request forever
- A server that ignores paging no longer makes lists load forever
- A radio station that keeps dropping is no longer redialed nonstop
- A truncated ogg file on android is fetched again once, not forever
- The terminal client no longer spins when a jam ends signed out
- Desktop lyrics memory, menus, back navigation and smoother redraws (#109)
- The playing song in the queue opens the player; title says Queue
- A song playing during a profile switch stays in its own profile
- Lyrics move from line to line on an e-ink screen (fixes #110)
- The random albums widget refreshes with the refresh glyph
- Text in desktop menus follows the day or night theme (fixes #118)
- The desktop queue scrolls to each new song unless scrolled away (fixes #115)
- A pause fades out fully before the desktop's sound stops (fixes #116)
- The random albums row keeps its draw until a refresh (fixes #113)
- A lyric line is left once its last word is sung (fixes #121)
- The landscape cover reaches the edges as burn-in protection moves it (fixes #112)
- The mini player's title walks again after a motion change (fixes #117)
- Automix holds one level through a mix and ends at the new song's
- A pause says where the music stopped, not a burst before (fixes #114)
- The sideways player pill keeps clear of the gesture bar
- Desktop seal in full ink, bundled icon left to macOS (fixes #122)
- Device lists show a song only where it plays now
- A like on the device playing shows on every page (fixes #124)
- Desktop player buttons stay quiet until switched on
- Music playing on another device shows in ink, not the accent
- The mini player's quiet glyphs share one gray
- The seek bar has one track and an accent playhead everywhere
- The terminal's full player seal is in ink beside the title
- Desktop lyrics keep their rows once sung, so nothing shifts
- A like on the web shows at once on the device playing the song
- The desktop's full player always shows one panel, in its own sheet
- The web calls its devices panel Devices, as the desktop does
- The web's top bar shows the nori wordmark on the page's edge
- The sideways player pill stays put again, off the gesture bar's sides only
- The desktop's full player panel is borderless again, the side sheet closes
- The desktop's full player lists devices as they appear
- The seek bar holds still while a song loads
- Switching devices shows the handed song and place, no old cover
- The seek bar moves a quarter pixel at a time again, as smoothly as before
- A download's files reach the disk before it counts as downloaded
- Moving music to another device mid-mix carries the mix on there
- An MP3 started mid-song streams from there instead of loading up to it
- A song streamed at a low bitrate no longer stalls when it opens partway
- A download media3 lost track of can still be removed
- A device played on stays controlled when its network announcement drops
- Handing playback to another device never shows its old song first
- Lyrics stay blurred round the next line before the first is sung
- Taking playback back shows the other device until its queue is here
- Nori nightly installs the next nightly instead of calling it not newer
- Nori nightly no longer shows the beta updates switch

### Performance

- The seek bar redraws once per pixel, a quarter as often
- Small moving things repaint only themselves, not the whole screen (#127)
- The lyrics list draws in an island of its own while nothing round it moves (#129)

## [0.7.0] - 2026-10-23

**nori has a new look.** It's called Nori Seasons, and every app wears it: Android, the desktop, the
terminal and the iPod. It's the biggest change to how nori looks so far, so we'd really like to hear
what you think. If something looks off, is harder to use than before, is broken, or you just don't like
it, please [open an issue](https://github.com/norifm/nori/issues/new) and tell us. A screenshot helps a
lot.

**Many settings were removed or merged.** Most were rarely changed, so where one well-tuned behavior
serves everyone, nori now just uses it. Your old choices carry over to the new options where they
exist. The list is under "Removed" below. If you relied on one that's gone, open an issue and tell us
how you used it.

### New

- **Nori Seasons, the new look.** Pages are paper in a season (Spring 春, Summer 夏, Autumn 秋,
  Winter 冬) that follows the calendar or one you pick, in System, Day, Night or Midnight (true black).
  New icons, and a favorite is now the seal 好 instead of a heart. On Android, home, the library, album
  and artist pages, menus and the player are reworked. The desktop, terminal and iPod keep their own
  layouts in the new style, and the terminal was redesigned to feel like a terminal app.
- **Lyrics from eleven services**, asked at once and checked against the singing, so the timing lines up
  with the voice. You can nudge a song's timing in 50 ms steps.
- **YouTube Music works like a server.** Sign in with Google, search, browse artists with every album
  and single, play, download, radio and live streams, Android Auto, widgets and jams. Works in the
  terminal too.
- **The server can go away mid-song.** An unreachable server no longer pauses playback: the song waits
  and goes on, and the offline queue hands back to yours as soon as the server is back.
- Albums, playlists and mixes offer Play next and Add to queue.
- Search leads with the top song's artist and album, then what you play most.
- Albums can be sorted into a row of covers per artist.
- Burn-in protection: shifts the screen, dims it and hides the bars while nori keeps the screen on.
- E-ink motion: the seek bar fills in blocks and counts whole minutes, nothing else moves.
- Storage shows downloads for every profile, and each profile's download size is its own.
- The desktop's song and player menus have everything Android's do (sleep timer, lyrics timing and
  more), and its artist pages shelve singles, EPs and live albums apart.
- Linux media controls show the cover and follow seeks.

### Better

- Songs start sooner, especially on a slow connection, and seeks land faster.
- Less work while playing, less memory and fewer requests.
- Settings that depend on another switch stay in place, grayed out, instead of jumping around.
- Download notifications show a progress bar and a steadier time left.

### Removed

These settings are gone. Where something replaces them, it's named here.

- **Pure black and Cover colors.** Pick the Midnight appearance for true black (it turns cover colors
  off). In every other appearance cover colors are always on.
- **Volume leveling's preamp, gain boost and untagged gain.** Leveling is now Off, Song, Album or Auto
  (album in order, song in shuffle), at a Quiet, Normal or Loud level. Untagged songs are measured and
  clipping is always prevented. Volume boost stays.
- **Loudness compensation and the noise gate.** Removed. **Bass boost** is now an equalizer preset.
- **The compressor's knobs.** It's a switch with Light, Car and Night presets.
- **Crossfeed presets and the virtualizer slider.** Now one choice, Headphone sound: Off, Natural or
  Wide.
- **AutoMix's sub-switches and the crossfade curve.** AutoMix is one switch (plus better beat
  detection), and crossfade is Off or 1 to 12 seconds on a fixed curve.
- **Reduce motion, E-ink screen and Animate anyway.** Now one choice, Motion: Full, Reduced or E-ink.
  nori's choice wins over Android's animation setting.
- **UI scale.** nori follows Android's display size.
- **The favorite notice.** Removed.
- **Lyrics prefetch and skip on error.** Always on now.
- **Playlist descriptions and hide import notes.** Merged into one switch.
- **Search delay, scrobble point, downloads at once and covers ahead.** Fixed at tuned values.
- **Lyrics from Genius, Megalobiz, Portato, SimpMusic and YouTube captions.** Dropped after measuring
  them on hundreds of songs; the other services find more, with better timing.
- **The separate Keys section.** BetterLyrics takes its key on its own row.

### Fixed

- Dozens of fixes, mostly playback that stuck on loading, remote control and jams, scrobbles a proxy
  refused, lyrics after quick skips or a lost network, and the Linux desktop app.

## [0.6.2] - 2026-10-17

### Added

- Pick several music folders per server (#64)
- Play, pause and skip buttons have softly rounded corners
- Home offers to continue the queue another device left on the server
- Shuffle lives in library and home's corner rearranges its rows
- A server's libraries are picked under it in the servers list
- Home rows are set in settings, appearance
- Library lists its sections as pages, with history in its corner
- Library's corner keeps downloads and history as outline buttons
- Going back follows the finger and leaves at its speed, swiped anywhere on a page

### Fixed

- The player keeps its colours when the server's address changes
- Audio output and start a jam look like the other rows of play on

## [0.6.1] - 2026-10-16

### Added

- A cover's page colours follow the cover, not the theme

### Fixed

- A headset play with nothing saved no longer crashes the app
- A cover has the same colours at every size
- A jam host shows to its guests once it can hear their commands
- A device just started no longer drops off the device lists
- A guest listening along gets the host's playback in order
- A serving device the relay lost says its state again
- A device that stops serving leaves after its last poll
- The speaker is filled only while another device plays
- The speaker is lit only for a device the devices list shows
- Pale and tinted covers get a light paper page
- A pastel cover's dark page keeps its soft colour
- Accents read as text and keep a black cover's own colour
- A page takes the colour that touches the artwork's bottom, never a mix
- A page takes the colour the cover visibly dissolves from, not a hidden line
- A cover's fine texture gives its page the colour the blur shows
- A page takes the cover colour it dissolves into most softly

## [0.6.0] - 2026-10-15

### New

- **Play on your other devices.** Control and mirror the phone, desktop, terminal and iPod from each other's player.
- **Jams.** Host from any device and invite people with a QR code or link. Guests ask for songs and can listen along in sync.
- **Sing.** Turns the vocals down in any song, processed on your device.
- **Listening stats from all your devices**, using your Navidrome play history.
- **Desktop app for macOS and Linux** (disk image, AppImage, Flatpak), plus a terminal client.
- Favorites sync between nearby devices. Updates install on their own, and you can opt into beta updates.

### Better

- Equalizer, compressor, limiter and crossfades use much less CPU.
- Android Auto: every album, faster search, steadier covers, and the phone wakes far less often.
- The desktop uses less memory and draws nothing while hidden.

### Fixed

- Over 150 fixes, mostly remote control, jams, lyrics timing, covers and the desktop app.

## [0.5.1] - 2026-10-04

### Added

- IPod touch client for iOS 12
- A white accent colour makes the app monochrome
- The iPod app keeps its log in nori.log

### Changed

- Nori-host takes the client's output, volume and memory budget

### Fixed

- Offload takes the next song again after a pause and a CPU song
- Albums in order stay gapless however they were queued
- Star notes on desktop and in the terminal follow Confirm favorites
- IPod songs play at their own speed when the hardware rate moves
- A song of another rate keeps its speed when tapped again
- A song of another rate starts at once on the iPod and desktop
- The iPod's playing mark follows the song in every list
- Tapping the song that plays opens the iPod's player
- The iPod's For you tiles show their covers without a refresh
- A heart set anywhere on the iPod shows at once everywhere
- IPod albums and songs can remove their downloads

### Performance

- Covers come at the core's two renditions, scaled to each view

## [0.5.0] - 2026-10-03

### Added

- A mix's page shows its covers as one picture, blurred into the page like an album's, upright and on its side
- With a black background mix pages follow the albums and playlists choice, as the playlists they are
- On its side a cover is a wider band that reaches in under the words and controls beside it, going soft over a wider edge
- Home-screen widgets: a resizable player, the cover, for you, random albums and lyrics
- The app icon follows android's themed icons
- Android auto gets tabs, mixes, artists, genres, voice by name and covers
- In a car the controls and tabs sit on the driver's side, the cover on the other
- A car's controls can be set to the driver's side, the left or the right
- The lyrics' source and timing setting hides where they came from too

### Changed

- The rust cleanup, seamless sound changes and the bugs it found (#20)
- Server hosts and network cost are the platform's, not globals
- Each client keeps its own AutoEQ fetch flag
- The stream cache's evictor holds its key order as a handle
- A lyrics clock is made from the lyrics, with no global store
- The cover loader owns its net and transport, not globals
- Download doors take the core's downloads as a handle
- Player and measurer get the client and analyses as handles
- An engine owns its fetches' stall watch and memory figures
- Drop player helpers nothing calls
- One stream cache order for the engine's store and media3's
- Drop unused exports and the forwarding engine host
- Drop an ignored output event and a dead track parameter
- Queue session and settings are held by the platform
- The queue says if a song is starred or explicit, not bits
- One day length in the library
- Drop offload's unused fallback parameter
- A transition note's kind is an enum, not a string
- The download notice says what changed with an enum
- Android's memory door sits before the tests
- A cover warm-up is its own kind of flight, not a size
- Lyrics search hits are ranked by one rule for every service
- Home rows reach the library as the settings' enum
- A built-in smart playlist is known by its kind, not its id
- An autofill's picks travel with its songs, not in a stash
- The track writer alone runs its clock and counts flushed room
- A handover says which way it goes instead of a flag
- A flush the device was not told of is one state, not three
- The offloaded stream's end is one state, not two flags
- The sink's start time and queue end are states, not flag pairs
- A called off request says why with one state
- Each measuring shelf asks its own bridge, not a global
- An album list read names its order with the enum
- No limit and no engine word yet are options, not sentinels
- Radio ids and the planner's view of a song have one home
- One biquad response and one gain-kind test for the equalizer
- Which offload count is read and believed are states, not flags

### Fixed

- On its side the library's album cards are large again, filling the row
- On its side shelves run to the screen edges under the camera and the tab rail, fading out smoothly on both sides
- On its side the player's controls stand in the middle of the screen's height
- On its side the page leaves the tab rail its strip from the first frame, so shelves never show unfaded as the app opens
- On its side a cover reaches less far under the words and controls beside it, so they read clearly over its soft edge
- On its side a page whose cover did not load keeps the same layout, with the plain plate standing in for the picture
- The lyrics widget opens the player on its lyrics when the app was closed
- Any window wider than tall lays out across, so a car or tablet shows the controls
- A car's media app opens nori, and shows its now playing cover, icons and buttons
- The words on a cover-coloured play button read on it, dark on a pale one
- A car keeps nori on screen while driving, so music can be played and controlled
- A car's dock and media card open nori's own screens, not the car's plain media player
- A cache write racing a measurer's stop no longer uses freed memory
- Mobile data is known as metered before the player has ever started
- On the desktop a song let go hangs up its request at once
- Downloading a whole album says how many songs were queued
- A request failing in the stall watch no longer keeps its timer
- The offline bridge no longer ends on the network that just failed
- An app hidden while connecting to the player lets it go
- A closed offload track's late event no longer hits the new track
- AutoMix no longer measures a provider playlist's songs
- Download speed holds between songs instead of dropping to zero
- Playback no longer crashes when the queue changes under it
- Engine callbacks reach the player that started the engine
- Replaygain follows the song the player holds during a queue edit
- Crossfeed and limiter set by name stay in their range

## [0.4.8] - 2026-09-29

### Added

- A mix's page shows its covers as one picture, blurred into the page like an album's, upright and on its side
- With a black background mix pages follow the albums and playlists choice, as the playlists they are
- On its side a cover is a wider band that reaches in under the words and controls beside it, going soft over a wider edge
- Home-screen widgets: a resizable player, the cover, for you, random albums and lyrics
- The app icon follows android's themed icons

### Fixed

- On its side the library's album cards are large again, filling the row
- On its side shelves run to the screen edges under the camera and the tab rail, fading out smoothly on both sides
- On its side the player's controls stand in the middle of the screen's height
- On its side the page leaves the tab rail its strip from the first frame, so shelves never show unfaded as the app opens
- On its side a cover reaches less far under the words and controls beside it, so they read clearly over its soft edge
- On its side a page whose cover did not load keeps the same layout, with the plain plate standing in for the picture
- The lyrics widget opens the player on its lyrics when the app was closed

## [0.4.7] - 2026-09-29

### Added

- The app keeps a day of its log and a report of a problem shares it, with drop-outs logged (#17)
- Sideways polish, status bar and keep awake settings, original quality on mobile data (#18)

### Changed

- State handed through globals, magic numbers and strings made explicit across the core, the player and the clients

### Fixed

- Downloads and their analysis keep the CPU awake with the screen off (closes #12)
- The keyboard no longer comes up over the player opened from search (closes #16)
- The album autoplay carries on with stays gapless with keep albums gapless on
- A song waiting only for its beats no longer crashes the downloads screen, the phase is one enum
- The desktop app keeps one downloader and an album added to the queue whole stays gapless
- A heart pressed on one server no longer shows on another, the marks are the core's
- A nameless DAC or Bluetooth device is no longer searched in AutoEQ by its placeholder name
- An Android player going no longer stops the fetching ahead of the one that replaced it
- The music is no longer left silent while it plays, the chip follows every fade and a fade left down is brought back

## [0.4.6] - 2026-09-28

### Added

- **desktop:** A native Slint desktop player over the core and nori-engine, laid out as the terminal client
- **desktop:** The window looks and behaves like Apple Music on macOS: Skia on Metal, the system's sidebar blur, SF Pro, a floating player, Now Playing, the native menu bar
- **desktop:** The sidebar runs the window's full height over the system's sidebar material, its rows and the page where they were
- **desktop:** Settings, synced lyrics, long lists that open at once, a smooth seek bar and Apple's animations
- **desktop:** The player floats on the system's Liquid Glass, artist pages open on their picture, the sidebar takes the page's colour
- **desktop:** Music's own colours measured from the Mac, album headers and buttons as Music has them, the player at its size
- **desktop:** Our own Liquid Glass: the window is drawn by our compositor, the page seen through glass shaders under the sidebar and the player
- **desktop:** The sidebar's glass is lit by what lies beside it on the page, softly and well into the pane
- **desktop:** Settings centred in four tabs as the Mac's own, the sidebar's edge soft
- **desktop:** Lyrics and the queue docked on the right as Music's; Now Playing rebuilt, its lyrics sharp only on the line sung; bigger icons
- **desktop:** Every setting the Android app has that a Mac can use, in its words: an equalizer page, lyrics sources in order, accounts
- **desktop:** A Find field in the toolbar, Artists in two columns, the playlist as a table, an account row
- **desktop:** Lyrics fill word by word and a click on a line seeks to it, the queue's rows fold and open, the traffic lights level with the toolbar as Music's
- **desktop:** The line sung as the Android app draws it: a soft-edged fill, each word and syllable rising as it is sung, a held note swelling
- On its side the app puts the tabs on a floating rail at the left, Search on its own below them
- On its side the player puts the cover, lyrics or queue on the left and the controls on the right
- On its side an album, artist or playlist page shows the cover and buttons on the left and the songs on the right
- On its side the tab rail stands at the right edge and pages keep clear of the camera's punch hole
- On its side the player's cover goes soft at its right edge into a wash under the controls, and lyrics and queue keep clear of the punch hole
- Album, artist, playlist and mix pages drop the back button over the cover, the back gesture leaves them
- On its side an album, artist or playlist page shows its cover as the player does, soft to the page from the screen's edge, with the name and buttons beside it
- On its side the tab bar keeps its size and order and only its glyphs turn, and a tinted page leaves the screen's edges with it

### Fixed

- **desktop:** The player's glass shows over a plain page, as a faint lit pane with a rim
- **desktop:** The sidebar's light from the page is faint and fades within a few dozen points of its edge
- **desktop:** The sidebar's light is a soft glow, gathered from the page blurred until no edge is left
- **desktop:** The player's glass is nearly invisible, as Music's: a neutral veil, no colour of its own, a faint rim
- **desktop:** The player's glass is clear as Music's: a light blur, the page's own colours, a thin quiet rim
- **desktop:** Covers on screen are never let go, so they stop flashing, and the ones on screen load first
- **desktop:** A word-timed line fades from white as the next one is sung, rather than dimming at once
- **desktop:** American spelling, as the app and the terminal client have it
- Shuffle albums keeps each album gapless instead of mixing between its songs
- Turning the phone keeps the player, its lyrics and the text size as they were
- On its side a page's cover fits beside the buttons, Play fits Pause, and the library grid keeps its upright card size
- On its side the player's status bar shade fades with the cover's soft edge instead of ending on a line
- On its side a page's cover is a card level with the first song, clear of the back button, and the artist page no longer crashes
- On its side the player keeps its title and controls in place for the lyrics, and clear of a punch hole on the right
- On its side the now playing bar has no wash under it, which ran over the cover and stopped at the camera
- On its side the now playing bar sits level with the foot of the tab rail

## [0.4.5] - 2026-09-27

### Fixed

- A song tapped again in its album no longer leaves the app paused over the music
- Shuffle albums and shuffle songs keep going after the queue ends, autoplay on or off

## [0.4.4] - 2026-09-27

### Fixed

- The app stays upright until it has a landscape layout
- Songs that left the queue no longer stay drawn over it
- Shuffle albums plays at once and never a provider's songs
- Tapping the song playing in its album keeps it going and opens the player
- Closing the queue dissolves and the title row moves with the controls

## [0.4.3] - 2026-09-27

### Added

- **cli:** TUI rewritten as a desktop music player: sidebar, pages, panel, player bar, equalizer console, card covers; late terminal replies over ssh no longer taken as keys
- Playlist descriptions can be turned off, and the server's auto-import notes are hidden
- Album and playlist pages share one keep-the-cover's-colours choice with a black background
- Home's menu shuffles songs or whole albums instead of one shuffle everything
- The app icon's shortcuts open Search or shuffle songs or albums

### Fixed

- The app and the terminal client use American spelling (color, favorite, license, analyze)
- A shuffle keeps going the way it started, random songs or whole albums, whatever autoplay adds

## [0.4.2] - 2026-09-27

### Added

- Every download is analysed after it is saved, and the beat model can read downloads with consent

### Fixed

- An artist's biography opens in full on a tap instead of stopping at four lines
- A stop the engine made before a play already sent no longer leaves the app paused over music
- **build:** Tools/bump-version.sh leaves nori-uniffi-jni-runtime at upstream's version in Cargo.lock
- Remote (octo-fiesta) songs: autoplay after them, library only, album cloud clears

## [0.4.1] - 2026-09-27

### Breaking

- To update from 0.3.x, install 0.4.0 first; later versions no longer bring over its servers, settings and downloads

### Added

- Remote items show only the cloud, not the provider's name
- Nori tells you when a newer release is out, and updates itself from inside the app
- The queue opens on the song playing, with the songs already played dimmed above it
- Lyrics sync listens to the centre of the stereo image, so panned guitars no longer read as singing
- Headphones taken off pause at once, and can resume fading in when put back on
- A graphic equalizer beside the parametric one, and a compressor, virtualizer, bass boost and volume boost
- ReplayGain turns quiet songs up, levels to a loudness target and measures untagged songs
- Crossfeed presets, no processing per output, crossfade curves, a noise gate, a 5-band EQ and loudness compensation
- High quality output keeps the effects in float, 16-bit output is dithered, and a real resampler with a highest rate per output
- The tab bar takes the cover's colours, and black background can keep them per page

### Fixed

- Undo after taking a song out of the queue puts it back again
- Downloads say when they are still finding lyrics or analysing, and the time left counts down steadily
- **build:** The Rust tests build and pass on macOS
- The last lyric line dims once it is sung, and better-timed lyrics arriving mid-song no longer step back a line
- A new album started on its first song shows that song as playing
- **build:** The dev server keeps its database in a docker volume, as SQLite on a mac's shared folder corrupts it
- Songs of one album played one after another never mix with AutoMix, whatever their tags say, each next song from its start
- **build:** Tools/app.sh launches the app on emulators without hardware keys, where monkey refuses to run
- Headphone play and pause keys follow the fade setting, no resume switch
- A long beat-matched AutoMix no longer shows the next song early and replays its first lyric line
- Keep albums gapless only for an album played or added whole, other songs of one album mix
- A page started from a song row reads Pause at once, its origin is republished when the service sets the queue

## [0.4.0] - 2026-09-26

The player is rewritten: playback, the queue, the library, lyrics and downloads now run in a Rust core
shared with a new terminal client, and the Android app draws the screens.

### Added

- A new player engine in Rust replaces ExoPlayer's playback, with offload to the audio chip that lets the CPU sleep
- Better beat detection (the Beat This! model, downloaded from its authors on first use) for AutoMix
- A terminal client (nori-cli) that plays through the same core
- Swipe a song out of the queue, with undo
- Every lyrics service is on by default, word-timed lyrics are sought first, and synced lyrics are checked against the song's voice and shifted when early or late
- Downloaded songs keep their lyrics offline, and a finished download shows while its lyrics and analysis are found
- Updating from 0.3.4 keeps your servers, settings, downloads and queue

### Fixed

- Playback no longer goes silent after fast skipping, a hung remote song or an engine error, and a song never ends stuck at its last second
- Transcoded songs play whole and start at once on mobile data
- Albums stay gapless with AutoMix on, across discs too
- The seek bar stays right after the app comes back and through tempo-matched mixes
- The first equalizer change plays on with no gap, and dialogs, sheets and the selection bar animate even with system animations off
- Covers never show the previous song's picture, playlist covers show offline, and a cover that cannot load settles on a placeholder

### Performance

- Far fewer wakeups while playing, and offload tops up only when the chip asks
- Less memory: songs already cached are read from disk, covers ahead wait on disk, and the beat model hands its memory back
- Long playlists open smoothly

## [0.3.4] - 2026-09-22

### Added

- Favourite messages replace each other at once and can be switched off
- Automix beat-matches band-played songs on their intro and outro grids
- About page with build details and third-party licences

### Fixed

- No dropout at the end of an AutoMix, and the bar moves to the next song as soon as it is heard
- Light and dark covers keep their own colour, and a big light strip at the bottom gets a light page
- The page moves to the next song when its mix is heard and never flips back
- Light pages keep the cover's own lightness instead of fading to white
- A song skipped to at another sample rate is converted instead of stuttering
- Taps on the full-screen player no longer reach the page underneath

## [0.3.3] - 2026-09-22

### Added

- Album and playlist pages pause their own queue, and Shuffle shows when it is on
- The player's cover blurs as it melts into the page
- Settings pages are split into short named sections, with plainer descriptions

### Fixed

- Page colours follow what the cover really is: a big field of colour beats a black or white border, and a dark strip at the bottom no longer leaves a band
- White covers keep a white page
- Controls stay readable on white pages, and the colours change smoothly between songs
- The player's controls follow the cover's colour while you swipe, with no jump halfway

### Performance

- Snappier page push and pop, and a quicker finish to the back gesture
