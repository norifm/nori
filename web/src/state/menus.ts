// Menus: a song's (the core's `song_menu`, worded and done here), a card's, adding to a playlist, a
// playlist's name. What each does is the core's; this offers it.

import { ask } from "../rpc";
import { W } from "../words";
import { library } from "./library.svelte";
import { player } from "./player.svelte";
import { remote } from "./remote.svelte";
import { go, route } from "./route.svelte";
import { ui, type Anchor, type Head, type Item } from "./ui.svelte";
import type { MenuAction, PlaylistChoice, Song, Whole } from "./types";

/** Picks a playlist for `songs`, or makes a new one with them there. */
export async function addToPlaylist(songs: Song[]) {
  const songIds = songs.map((s) => s.id);
  ui.open = null;
  const picking = (ui.picking = { songIds, choices: null });
  const choices = await ask<string>("playlist_choices", JSON.stringify(songIds)).then((j) => JSON.parse(j) as PlaylistChoice[], () => []);
  if (ui.picking === picking) ui.picking = { songIds, choices };
}

/** An album's, playlist's, artist's or mix's card menu, opening `at`, as the desktop's: play it, play it
 * next, add it to the queue, start a jam with it where one can start, and for a playlist the account may
 * change (`editable`), delete it. */
export function wholeMenu(kind: Whole, id: string, head: Head, at: Anchor, editable = false) {
  ui.menu(
    head,
    [[
      { label: W.play, icon: "play", run: () => player.playWhole(kind, id, false) },
      { label: W.playNext, icon: "playlist-play", run: () => player.queueWhole(kind, id, true) },
      { label: W.addToQueue, icon: "playlist", run: () => player.queueWhole(kind, id, false) },
      ...(remote.jam?.start === "Offered" ? [{ label: W.startJam, icon: "community", run: () => jamWith(kind, id) }] : []),
      ...(editable ? [{ label: W.deletePlaylist, icon: "trash", run: () => deletePlaylist(id, head.title) }] : []),
    ]],
    at,
  );
}

/** Plays album, playlist, artist or mix `id` and opens a jam around it, as a song's Start jam does. */
async function jamWith(kind: Whole, id: string) {
  await player.playWhole(kind, id, false);
  await remote.act(() => ask("jam_open"));
  await player.refresh();
  ui.show("devices");
}

/** Deletes playlist `id` (called `name`) once asked whether to; whether it was. */
export async function deletePlaylist(id: string, name: string): Promise<boolean> {
  if (!(await ui.confirm(W.deleteConfirm(name), W.deleteLine, W.delete))) return false;
  await library.change({ kind: "deletePlaylist", id });
  return true;
}

/** Asks for a name and makes a playlist of `songIds` called that. */
export async function newPlaylist(songIds: string[] = []) {
  const name = await ui.ask(W.newPlaylist);
  if (name) await library.change({ kind: "createPlaylist", name, songIds });
}

/** The sleep timer's choices; the timer itself is the core's. */
async function sleepMenu() {
  const choices = JSON.parse(await ask<string>("sleep_choices")) as { minutes: number; end: boolean; songs: number }[];
  ui.menu({ title: W.sleepTimer }, [choices.map((c) => ({ label: W.sleepChoice(c.minutes, c.end, c.songs), run: () => ask("sleep_pick", c.minutes, c.end, c.songs) }))]);
}

/** What a song menu's action is, or null for one a browser has nothing for (downloads, sharing,
 * details). */
function item(a: MenuAction, s: Song): Item | null {
  switch (a.action) {
    case "Favourite":
      return { label: a.on ? W.addToFavorites : W.removeFromFavorites, icon: a.on ? "seal" : "sealed", lit: !a.on, run: () => library.change({ kind: "star", what: "song", id: s.id, on: !!a.on }).then(() => player.refresh()) };
    case "PlayNext":
      return { label: W.playNext, icon: "playlist-play", run: () => player.enqueue([s], true) };
    case "AddToQueue":
      return { label: W.addToQueue, icon: "playlist", run: () => player.enqueue([s], false) };
    case "AddToPlaylist":
      return { label: W.addToPlaylist, icon: "plus-circle", run: () => addToPlaylist([s]) };
    case "GoToAlbum":
      return { label: W.goToAlbum, icon: "compact-disc", run: () => ((ui.full = false), go("album", a.id!)) };
    case "GoToArtist":
      return { label: W.goToArtist(a.name ?? "", !!a.named), icon: "user", run: () => ((ui.full = false), go("artist", a.id!)) };
    case "Lyrics":
      return { label: W.lyricsTimingMenu, icon: "quote-message", run: () => (ui.lyricsInfo = true) };
    case "Radio":
      return { label: W.radio, icon: "antenna-signal", run: () => player.radio(s) };
    case "SleepTimer":
      return { label: W.sleepTimer, icon: "half-moon", run: sleepMenu };
    default:
      return null;
  }
}

/** Song `s`'s menu, opening `at`, laid out as the core places it: the seal beside the title, the bare glyphs
 * under it, then the lines most reached for, the album and artist, and the rest. Opened on a row it starts
 * with Play (`play`, what clicking the row does); `extra` the page's own entries; `inPlayer` when opened
 * from the player. */
export async function songMenu(s: Song, at: Anchor, { play, extra = [], inPlayer = false }: { play?: () => unknown; extra?: Item[]; inPlayer?: boolean } = {}) {
  const lyrics = inPlayer && !!player.shown && player.shown.song === s.id;
  const page = inPlayer ? "" : route.top;
  const { items: actions, quality } = JSON.parse(await ask<string>("menu", JSON.stringify(s), inPlayer, lyrics, page, route.arg)) as { items: MenuAction[]; quality: string | null };
  const of = (place: MenuAction["place"], more?: boolean) =>
    actions.flatMap((a): Item[] => {
      const i = a.place === place && (more === undefined || !!a.more === more) ? item(a, s) : null;
      return i ? [{ ...i, disabled: !a.enabled }] : [];
    });
  const first: Item[] = play ? [{ label: W.play, icon: "play", run: play }] : [];
  ui.menu(
    { title: s.title, line: s.line || s.artist, cover: s.cover ?? "", glyphs: of("Title"), quality: quality ?? undefined },
    [[...first, ...of("Line", false), ...extra], of("Names"), of("Line", true)],
    at,
    of("Glyphs"),
  );
}
