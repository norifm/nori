// Action sheets and the small asks: a song's menu, adding to a playlist, a playlist's name, hearts.
// What each does is the core's (`change` in the worker); this offers it.

import { ask, read } from "./rpc.js";
import { h, icon } from "./dom.js";
import { enqueue, Song } from "./player.js";
import { W } from "./words.js";

type Item = { label: string; icon?: string; run: () => unknown; danger?: boolean };

/** An action sheet over the page; `title` names what it is for. */
export function sheet(title: string, items: Item[]): void {
  const d = h("dialog", { class: "menu" }) as HTMLDialogElement;
  const close = () => {
    d.close();
    d.remove();
  };
  d.append(
    h("p", { class: "menu-title" }, title),
    ...items.map((it) => h("button", { class: it.danger ? "menu-item danger" : "menu-item", on: { click: () => (close(), void it.run()) } }, it.icon ? icon(it.icon) : "", it.label)),
    h("button", { class: "menu-item cancel", on: { click: close } }, W.cancel),
  );
  d.addEventListener("click", (e) => e.target === d && close());
  document.body.append(d);
  d.showModal();
}

/** Asks for a line of text (a playlist's name); null when cancelled. */
export function askText(title: string, value = ""): Promise<string | null> {
  return new Promise((answer) => {
    const d = h("dialog", { class: "menu" }) as HTMLDialogElement;
    const field = h("input", { class: "link", value, "aria-label": title }) as HTMLInputElement;
    const done = (v: string | null) => {
      d.close();
      d.remove();
      answer(v && v.trim() ? v.trim() : null);
    };
    const form = h("form", { method: "dialog" }, h("p", { class: "menu-title" }, title), field, h("div", { class: "menu-row" }, h("button", { type: "button", class: "pill", on: { click: () => done(null) } }, W.cancel), h("button", { class: "pill" }, W.done)));
    form.onsubmit = (e) => (e.preventDefault(), done(field.value));
    d.append(form);
    document.body.append(d);
    d.showModal();
    field.select();
  });
}

/** Asks the server for a change; the page is drawn again with it. */
export async function change(c: Record<string, unknown>): Promise<void> {
  await ask("change", JSON.stringify(c));
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  window.dispatchEvent(new Event("nori-changed"));
}

/** A heart on `what` (song, album, artist) `id`, drawn as `on`, that turns it the other way. */
export function heart(what: string, id: string, on: boolean): HTMLButtonElement {
  const b = h("button", { class: on ? "plain heart on" : "plain heart", "aria-label": on ? W.unfavorite : W.favorite, "aria-pressed": on ? "true" : "false" }, icon(on ? "heart" : "heart-line")) as HTMLButtonElement;
  b.onclick = (e) => {
    e.stopPropagation();
    void change({ kind: "star", what, id, on: !on });
  };
  return b;
}

/** Picks a playlist for `songs`, or makes a new one with them. */
export async function addToPlaylist(songs: Song[]): Promise<void> {
  const { playlists } = await read<{ playlists: { id: string; name: string }[] }>({ kind: "playlists" });
  const ids = songs.map((s) => s.id);
  sheet(W.addToPlaylist, [
    {
      label: W.newPlaylist,
      icon: "playlist",
      run: async () => {
        const name = await askText(W.playlistName);
        if (name) await change({ kind: "createPlaylist", name, songIds: ids });
      },
    },
    ...(playlists ?? []).map((p) => ({ label: p.name, run: () => change({ kind: "addToPlaylist", id: p.id, songIds: ids }) })),
  ]);
}

/** A song's menu: play next or later, add to a playlist, heart; `extra` for the page's own. */
export function songMenu(s: Song & { starred?: boolean }, extra: Item[] = []): void {
  sheet(s.title, [
    { label: W.playNext, icon: "queue", run: () => enqueue([s], true) },
    { label: W.playLater, icon: "queue", run: () => enqueue([s], false) },
    { label: W.addToPlaylist, icon: "playlist", run: () => addToPlaylist([s]) },
    { label: s.starred ? W.unfavorite : W.favorite, icon: "heart", run: () => change({ kind: "star", what: "song", id: s.id, on: !s.starred }) },
    ...extra,
  ]);
}
