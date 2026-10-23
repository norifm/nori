---
title: YouTube Music
description: A profile that searches and plays YouTube Music without a server, signed in to your account or not, best effort.
sidebar:
  order: 5
---

Applies to the nori release after 0.6.2, on Android, the desktop, the terminal and the iPod touch. Not on
the web client. Signing in is on Android, the desktop and the terminal.

Besides your own server, nori can add a **YouTube Music** profile. It needs no server: nori asks YouTube
directly, the way its own apps do. An account is optional.

:::caution[Best effort]
This is not an official YouTube client. YouTube can change what it answers at any time, and then search
or playback stops working until nori is updated. Your own server is the reliable way to use nori.
:::

## Adding it

- **Android**: on the sign-in screen, or in Settings, Servers, **Add server**, pick **YouTube Music**
  instead of **Your server**, then **Add YouTube Music**. A name is optional.
- **Desktop**: on the sign-in page, pick **YouTube Music**, then **Add YouTube Music**.
- **iPod touch**: on **Add a server**, pick **YouTube Music**.
- **Terminal**: start `nori-cli --provider youtubeMusic`.

It shows in the server list like any other profile, marked best effort, and you switch between it and
your servers the same way. Adding it again opens the same profile.

## Signing in (optional)

Signed in, Home is your own YouTube Music Home, Library has your saved albums, your liked songs (as
Favorites) and your playlists, the favorite seal on a song likes it on YouTube Music, and songs you hear
go into your YouTube history (with Scrobbling on, as for a server). Signed out, tapping the seal offers
the sign-in.

nori signs in with the cookies of a browser session, since YouTube no longer lets other apps sign in
with an account's password or a code.

### Sign in with Google

- **Android**: right after you add the profile, nori offers **Sign in with Google** (**Not Now** skips
  it). Later, it is in Settings, Servers: tap the YouTube Music profile (it must be the one in use),
  then **Sign in with Google**.
- **Desktop** (from 0.7.0): Settings, Accounts, **Edit** on the YouTube Music profile in use, then
  **Sign in with Google**. The page opens in a small window of its own (the system's web view; on
  Linux, WebKitGTK).
- **Terminal** (from 0.7.0): Settings, Servers, Enter on the YouTube Music profile in use, then Enter
  again. The same window opens, from the terminal's own program. Where no window can open (over SSH,
  say), the sheet turns to pasting cookies.

Google's own sign-in page opens inside nori. Sign in as you would in a browser. Once the page reaches
YouTube Music, nori keeps that session's cookies, wipes everything the sign-in page stored (its
cookies and site data), and closes it, so the page never keeps the session going behind nori's back.
**Cancel** closes it and wipes it the same way. The page is used for signing in and nothing else:
music never plays through it.

### Pasting cookies (the fallback)

**Paste cookies instead** under **Sign in with Google** opens the field for this, on Android and the
desktop; in the terminal's sheet it is **p**.

1. Open a **private** (incognito) browser window and sign in at music.youtube.com.
2. Open the browser's developer tools, Network, click any request to music.youtube.com, and copy the
   value of its **Cookie** request header. (A cookies.txt export of that window works too.)
3. **Close the private window.** A browser that keeps the session open keeps changing it, and the copy
   nori has would stop working.
4. Paste it:
   - **Android**: Settings, Servers, tap the YouTube Music profile (it must be the one in use),
     **Paste cookies instead**, paste into **Cookies**, **Sign In**.
   - **Desktop**: Settings, Accounts, **Edit**, **Paste cookies instead**, then **Sign in**.
   - **Terminal**: in the sheet, **p**, paste, Enter. Or save it to a file and run
     `nori-cli --script --provider youtubeMusic --sign-in cookies.txt --home`.

nori keeps the session fresh by itself from then on: it renews it whenever the profile is used and it
is more than ten minutes old, and keeps every change YouTube makes to it at once. A network failure, or
YouTube answering oddly while it cannot be renewed, never signs you out; nori on the desktop and in the
terminal share one session, and take up each other's renewals. Only YouTube refusing a renewed session
ends it (you signed out everywhere, changed your password, or YouTube decided so). Then nori says
**YouTube signed you out** at once, offers to sign in again, and goes on as a signed-out profile. It asks
YouTube once each time it starts whether the session works again, and offers to sign in each time it
starts until you sign in or choose **Don't Show Again** (which lasts until a session ends again).
**Sign Out** forgets the cookies on this device.

Music always plays without the account: only Home, Library and likes use it.

:::caution[Your account]
Unofficial apps are against YouTube's terms, and YouTube may flag an account that uses one. If that
matters to you, use YouTube Music signed out.
:::

## What works

- Home: YouTube Music's own shelves (Quick picks, new releases, playlists), everyone's when signed out.
  The desktop and the terminal show their albums; Android and the iPod show the songs and playlists too.
  Home stays as it is until the app starts again. Pulling to refresh (Android) keeps every shelf and
  pick where it was and adds only what YouTube has new, at the end of its shelf or as a new shelf below.
- Searching as YouTube Music does: songs, videos, albums, artists, profiles and playlists, best match
  first, and opening albums, artists and playlists. Podcasts are not offered.
- An artist's page shows its albums and its top songs. A profile without albums (a channel that posts
  mixes, say) shows all its videos as its songs, and its Play plays them.
- Seeking in a long song or mix, or starting it partway, takes about a second: nori fetches songs in
  parts of 10 MB, which YouTube sends at full speed (it sends longer requests only as fast as they play).
- Signed in: your saved albums, liked songs and playlists in Library, and favorite seals that like songs.
- Playing, the queue, AutoMix, the equalizer and the rest of the sound settings.
- Lyrics from the lyrics services (there is no server to ask).
- Songs played once are kept in the stream cache like any other, and play again from there.
- Live streams (a 24-hour lofi radio, say) play as a radio station, from a little behind live and for
  as long as you listen. On Android, "Live" stands where the seek bar is and lyrics are off.

## What is off

Signed out there is no library: search and Home are the way in. Editing playlists, downloads,
scrobbling, listening history, mixes, song radio and sharing are not offered, signed in or not.
**Remote control and jams** are off in this profile, even when switched on in the settings. nori fetches
at most the next song ahead, so it asks YouTube for nothing you did not queue.
