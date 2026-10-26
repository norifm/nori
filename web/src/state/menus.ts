// Menus: a song's (the core's `song_menu`, worded and done here), adding to a playlist, a playlist's
// name. What each does is the core's; this offers it.

import { ask, read } from "../rpc";
import { W } from "../words";
import { library } from "./library.svelte";
import { player } from "./player.svelte";
import { remote } from "./remote.svelte";
import { go } from "./route.svelte";
import { ui, type Anchor, type Head, type Item } from "./ui.svelte";
import type { MenuAction, Song, Whole } from "./types";

/** Picks a playlist for `songs`, or makes a new one with them; the menu opens `at` (where the last one
 * did when not said). */
export async function addToPlaylist(songs: Song[], at?: Anchor) {
  const { playlists } = await read<{ playlists: { id: string; name: string }[] }>({ kind: "playlists" });
  const songIds = songs.map((s) => s.id);
  ui.menu(
    { title: W.addToPlaylist },
    [
      { label: W.newPlaylist, icon: "plus", run: () => newPlaylist(songIds) },
      ...(playlists ?? []).map((p) => ({ label: p.name, icon: "playlist", run: () => library.change({ kind: "addToPlaylist", id: p.id, songIds }) })),
    ],
    [],
    at,
  );
}

/** An album's, playlist's or artist's card menu, opening `at`: play it, play it next, add it to the queue. */
export function wholeMenu(kind: Whole, id: string, head: Head, at: Anchor) {
  ui.menu(
    head,
    [
      { label: W.play, icon: "play", run: () => player.playWhole(kind, id, false) },
      { label: W.playNext, icon: "playlist-play", run: () => player.queueWhole(kind, id, true) },
      { label: W.addToQueue, icon: "playlist", run: () => player.queueWhole(kind, id, false) },
    ],
    [],
    at,
  );
}

/** Asks for a name and makes a playlist of `songIds` called that. */
export async function newPlaylist(songIds: string[] = []) {
  const name = await ui.ask(W.newPlaylist);
  if (name) await library.change({ kind: "createPlaylist", name, songIds });
}

/** What a song menu's action is, or null for one a browser has nothing for (downloads, the sleep
 * timer, sharing, details). */
function item(a: MenuAction, s: Song): Item | null {
  switch (a.action) {
    case "Favourite":
      return { label: a.on ? W.addToFavorites : W.removeFromFavorites, icon: a.on ? "seal" : "sealed", run: () => library.change({ kind: "star", what: "song", id: s.id, on: !!a.on }).then(() => player.refresh()) };
    case "PlayNext":
      return { label: W.playNext, icon: "playlist-play", run: () => player.enqueue([s], true) };
    case "AddToQueue":
      return { label: W.addToQueue, icon: "playlist", run: () => player.enqueue([s], false) };
    case "AddToPlaylist":
      return { label: W.addToPlaylistMenu, icon: "plus-circle", run: () => addToPlaylist([s]) };
    case "GoToAlbum":
      return { label: W.goToAlbum, icon: "compact-disc", run: () => ((ui.full = false), go("album", a.id!)) };
    case "GoToArtist":
      return { label: W.goToArtist(a.name ?? "", !!a.named), icon: "user", run: () => ((ui.full = false), go("artist", a.id!)) };
    case "Lyrics":
      return { label: W.lyrics, icon: "quote-message", run: () => (ui.lyricsInfo = true) };
    case "StartRadio":
      return { label: W.startRadio, icon: "antenna-signal", run: () => player.mix(s, true) };
    case "InstantMix":
      return { label: W.instantMix, icon: "sparks", run: () => player.mix(s, false) };
    case "StartJam":
      return {
        label: W.startJam,
        icon: "community",
        run: async () => {
          await player.soundCard();
          await remote.act(() => ask("jam_start", JSON.stringify([s]), 0));
          await player.refresh();
          ui.show("devices");
        },
      };
    default:
      return null;
  }
}

/** Song `s`'s menu, opening `at`: what one opens it for, then the rest under More. Opened on a row it
 * starts with Play (`play`, what clicking the row does); `extra` the page's own entries; `inPlayer` when
 * opened from the player. */
export async function songMenu(s: Song, at: Anchor, { play, extra = [], inPlayer = false }: { play?: () => unknown; extra?: Item[]; inPlayer?: boolean } = {}) {
  const lyrics = inPlayer && !!player.shown && player.shown.song === s.id;
  const actions = JSON.parse(await ask<string>("menu", JSON.stringify(s), inPlayer, lyrics)) as MenuAction[];
  const of = (more: boolean) =>
    actions.flatMap((a): Item[] => {
      const i = !!a.more === more ? item(a, s) : null;
      return i ? [{ ...i, disabled: !a.enabled }] : [];
    });
  const first: Item[] = play ? [{ label: W.play, icon: "play", run: play }] : [];
  ui.menu({ title: s.title, line: s.line || s.artist, cover: s.cover ?? "" }, [...first, ...of(false), ...extra], of(true), at);
}
