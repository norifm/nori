// Menus: a song's (the core's `song_menu`, worded and done here), adding to a playlist, a playlist's
// name. What each does is the core's; this offers it.

import { ask, read } from "../rpc";
import { W } from "../words";
import { library } from "./library.svelte";
import { player } from "./player.svelte";
import { remote } from "./remote.svelte";
import { go } from "./route.svelte";
import { ui, type Item } from "./ui.svelte";
import type { MenuAction, Song } from "./types";

/** Picks a playlist for `songs`, or makes a new one with them. */
export async function addToPlaylist(songs: Song[]) {
  const { playlists } = await read<{ playlists: { id: string; name: string }[] }>({ kind: "playlists" });
  const songIds = songs.map((s) => s.id);
  ui.menu(W.addToPlaylist, [
    {
      label: W.newPlaylist,
      icon: "playlist",
      run: async () => {
        const name = await ui.ask(W.playlistName);
        if (name) await library.change({ kind: "createPlaylist", name, songIds });
        void library.readPlaylists();
      },
    },
    ...(playlists ?? []).map((p) => ({ label: p.name, run: () => library.change({ kind: "addToPlaylist", id: p.id, songIds }) })),
  ]);
}

export async function newPlaylist() {
  const name = await ui.ask(W.playlistName);
  if (!name) return;
  await library.change({ kind: "createPlaylist", name, songIds: [] });
  await library.readPlaylists();
}

/** What a song menu's action is, or null for one a browser has nothing for (downloads, the sleep
 * timer, sharing, details). */
function item(a: MenuAction, s: Song): Item | null {
  switch (a.action) {
    case "Favourite":
      return { label: a.on ? W.favorite : W.unfavorite, icon: a.on ? "heart-line" : "heart", run: () => library.change({ kind: "star", what: "song", id: s.id, on: !!a.on }).then(() => player.refresh()) };
    case "PlayNext":
      return { label: W.playNext, icon: "next-up", run: () => player.enqueue([s], true) };
    case "AddToQueue":
      return { label: W.playLater, icon: "queue", run: () => player.enqueue([s], false) };
    case "AddToPlaylist":
      return { label: W.addToPlaylist, icon: "playlist", run: () => addToPlaylist([s]) };
    case "GoToAlbum":
      return { label: W.goToAlbum, icon: "album", run: () => ((ui.full = false), go("album", a.id!)) };
    case "GoToArtist":
      return { label: W.goToArtist(a.name ?? "", !!a.named), icon: "artist", run: () => ((ui.full = false), go("artist", a.id!)) };
    case "Lyrics":
      return { label: W.lyricsInfo, icon: "lyrics", run: () => (ui.lyricsInfo = true) };
    case "StartRadio":
      return { label: W.startRadio, icon: "radio", run: () => player.mix(s, true) };
    case "InstantMix":
      return { label: W.instantMix, icon: "shuffle", run: () => player.mix(s, false) };
    case "StartJam":
      return {
        label: W.startJam,
        icon: "jam",
        run: async () => {
          await player.play([s], 0);
          await remote.act(() => ask("jam_open"));
          ui.show("devices");
        },
      };
    default:
      return null;
  }
}

/** Song `s`'s menu; `extra` for the page's own entries; `inPlayer` when opened from the player. */
export async function songMenu(s: Song, extra: Item[] = [], inPlayer = false) {
  const lyrics = inPlayer && !!player.shown && player.shown.song === s.id;
  const actions = JSON.parse(await ask<string>("menu", JSON.stringify(s), inPlayer, lyrics)) as MenuAction[];
  const items = actions.map((a) => item(a, s)).filter((i): i is Item => i !== null);
  ui.menu(s.title, [...items, ...extra]);
}
