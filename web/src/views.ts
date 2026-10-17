// The pages: what each route shows, read from the core through the worker.

import { ask, facts, read } from "./rpc.js";
import { cover, h, icon, iconButton } from "./dom.js";
import { addToPlaylist, askText, change, heart, sheet, songMenu } from "./menu.js";
import { play, Song } from "./player.js";
import { panel } from "./jam.js";
import { W } from "./words.js";

type Album = { id: string; name: string; artist: string; artistId?: string; year: number; cover?: string; songCount: number; duration: number };
type Artist = { id: string; name: string; cover?: string; albumCount: number };
type Playlist = { id: string; name: string; songCount: number; duration: number; cover?: string; comment?: string; owner?: string };
/** As the server names it: the name is `value`. */
type Genre = { value: string; songCount: number; albumCount: number };

/** A page's big title, with a back link where it is not a top page. */
function title(text: string, back = false, extra?: HTMLElement): HTMLElement {
  return h("header", { class: "page-head" }, back ? h("button", { class: "back", "aria-label": "Back", on: { click: () => history.back() } }, icon("back")) : null, h("h1", null, text), extra ?? null);
}

function albumCard(a: Album, sub = a.artist): HTMLElement {
  return h("a", { class: "card", href: `#/album/${encodeURIComponent(a.id)}` }, cover(a.cover), h("span", { class: "title" }, a.name), h("span", { class: "sub" }, sub));
}

function artistCard(a: Artist): HTMLElement {
  return h("a", { class: "card artist", href: `#/artist/${encodeURIComponent(a.id)}` }, cover(a.cover, "cover round"), h("span", { class: "title" }, a.name));
}

function shelf(name: string, href: string, items: HTMLElement[]): HTMLElement | null {
  if (items.length === 0) return null;
  return h("section", { class: "shelf" }, h("div", { class: "shelf-head" }, h("h2", null, name), h("a", { href }, W.seeAll)), h("div", { class: "row" }, ...items));
}

function grid(items: HTMLElement[]): HTMLElement {
  return items.length ? h("div", { class: "grid" }, ...items) : h("p", { class: "empty" }, W.empty);
}

type Starred = { starred?: boolean };
type Extra = (s: Song, i: number) => { label: string; icon?: string; run: () => unknown; danger?: boolean }[];

/** A list of songs; a tap plays them from there, as the page's list; each has its menu (`extra`: the
 * page's own entries). */
function songList(songs: Song[], kind: string, id: string, numbered = false, extra: Extra = () => []): HTMLElement {
  return h(
    "ol",
    { class: numbered ? "songs numbered" : "songs" },
    ...songs.map((s, i) =>
      h(
        "li",
        { on: { click: () => void play(songs, i, false, kind, id) } },
        numbered ? h("span", { class: "n" }, String((s as Song & { track?: number }).track || i + 1)) : cover(s.cover, "cover tiny"),
        h("div", { class: "row-text" }, h("span", { class: "title" }, s.title), h("span", { class: "sub" }, numbered ? s.artist : `${s.artist} · ${s.album}`)),
        (s as Song & Starred).starred ? h("span", { class: "starred", "aria-label": W.favorite }, icon("heart")) : "",
        h("span", { class: "len" }, W.time(s.duration * 1000)),
        iconButton("more", W.more, () => songMenu(s, extra(s, i)), "plain small"),
      ),
    ),
  );
}

function playButtons(songs: Song[], kind: string, id: string): HTMLElement {
  return h(
    "div",
    { class: "actions" },
    h("button", { class: "pill", on: { click: () => void play(songs, 0, false, kind, id) } }, icon("play"), W.play),
    h("button", { class: "pill", on: { click: () => void play(songs, 0, true, kind, id) } }, icon("shuffle"), W.shuffle),
  );
}

export async function home(): Promise<HTMLElement> {
  const list = (sort: string, size = 12) => read<{ albums: Album[] }>({ kind: "albums", sort, size, offset: 0 }).then((p) => p.albums ?? [], () => []);
  const [newest, recent, frequent, random] = await Promise.all([list("newest"), list("recent"), list("frequent"), list("random", 8)]);
  facts.albums = newest.length;
  return h(
    "div",
    { class: "page" },
    title(W.home),
    shelf(W.recentlyAdded, "#/albums/newest", newest.map((a) => albumCard(a))),
    shelf(W.recentlyPlayed, "#/albums/recent", recent.map((a) => albumCard(a))),
    shelf(W.mostPlayed, "#/albums/frequent", frequent.map((a) => albumCard(a))),
    shelf(W.discover, "#/albums/random", random.map((a) => albumCard(a))),
  );
}

export async function library(): Promise<HTMLElement> {
  const recent = await read<{ albums: Album[] }>({ kind: "albums", sort: "newest", size: 12, offset: 0 }).then((p) => p.albums ?? [], () => []);
  const entry = (name: string, href: string, i: string) => h("li", null, h("a", { href }, icon(i), h("span", null, name), icon("chevron")));
  return h(
    "div",
    { class: "page" },
    title(W.library),
    h("ul", { class: "sections" }, entry(W.playlists, "#/playlists", "playlist"), entry(W.artists, "#/artists", "artist"), entry(W.albums, "#/albums/alphabeticalByName", "album"), entry(W.songs, "#/songs", "note"), entry(W.genres, "#/genres", "genre"), entry(W.favorites, "#/favorites", "heart")),
    h("h2", { class: "section-title" }, W.recentlyAdded),
    grid(recent.map((a) => albumCard(a))),
  );
}

export async function albums(sort: string): Promise<HTMLElement> {
  const page = await read<{ albums: Album[] }>({ kind: "albums", sort, size: 200, offset: 0 });
  const select = h(
    "select",
    { class: "sort", "aria-label": W.sortBy, on: { change: (e) => (location.hash = `#/albums/${(e.target as HTMLSelectElement).value}`) } },
    ...Object.entries(W.sorts).map(([k, v]) => h("option", { value: k, selected: k === sort }, v)),
  );
  return h("div", { class: "page" }, title(W.albums, true, select), grid((page.albums ?? []).map((a) => albumCard(a))));
}

export async function album(id: string): Promise<HTMLElement> {
  const p = await read<{ album: Album; songs: Song[]; seconds: number }>({ kind: "album", id });
  const a = p.album;
  const songs = p.songs ?? [];
  return h(
    "div",
    { class: "page detail" },
    h("button", { class: "back", "aria-label": "Back", on: { click: () => history.back() } }, icon("back")),
    h(
      "div",
      { class: "hero" },
      cover(a.cover, "cover hero-cover"),
      h(
        "div",
        { class: "hero-text" },
        h("h1", null, a.name),
        h("a", { class: "artist-link", href: a.artistId ? `#/artist/${encodeURIComponent(a.artistId)}` : "#" }, a.artist),
        h("span", { class: "sub" }, [a.year || "", W.songCount(songs.length), W.minutes(p.seconds ?? 0)].filter(Boolean).join(" · ")),
        h("div", { class: "hero-actions" }, playButtons(songs, "Album", a.id), heart("album", a.id, !!(a as Album & Starred).starred), iconButton("more", W.more, () => void addToPlaylist(songs), "plain")),
      ),
    ),
    songList(songs, "Album", a.id, true),
  );
}

export async function artists(): Promise<HTMLElement> {
  const p = await read<{ artists: Artist[] }>({ kind: "artists" });
  return h("div", { class: "page" }, title(W.artists, true), grid((p.artists ?? []).map(artistCard)));
}

export async function artist(id: string): Promise<HTMLElement> {
  const p = await read<{ artist: Artist; albums: Album[] }>({ kind: "artist", id });
  const albumsOf = p.albums ?? [];
  const playAll = async (shuffle: boolean) => {
    const all: Song[] = [];
    for (const a of albumsOf) all.push(...((await read<{ songs: Song[] }>({ kind: "album", id: a.id })).songs ?? []));
    await play(all, 0, shuffle, "Artist", id);
  };
  return h(
    "div",
    { class: "page detail" },
    h("button", { class: "back", "aria-label": "Back", on: { click: () => history.back() } }, icon("back")),
    h(
      "div",
      { class: "hero artist-hero" },
      cover(p.artist.cover, "cover round hero-cover"),
      h(
        "div",
        { class: "hero-text" },
        h("h1", null, p.artist.name),
        h("div", { class: "actions" }, h("button", { class: "pill", on: { click: () => void playAll(false) } }, icon("play"), W.play), h("button", { class: "pill", on: { click: () => void playAll(true) } }, icon("shuffle"), W.shuffle), heart("artist", p.artist.id, !!(p.artist as Artist & Starred).starred)),
      ),
    ),
    h("h2", { class: "section-title" }, W.albums),
    grid(albumsOf.map((a) => albumCard(a, String(a.year || "")))),
  );
}

export async function playlists(): Promise<HTMLElement> {
  const p = await read<{ playlists: Playlist[] }>({ kind: "playlists" });
  const card = (pl: Playlist) => h("a", { class: "card", href: `#/playlist/${encodeURIComponent(pl.id)}` }, cover(pl.cover), h("span", { class: "title" }, pl.name), h("span", { class: "sub" }, W.songCount(pl.songCount)));
  const create = h("button", { class: "pill small", on: { click: () => void newPlaylist() } }, W.newPlaylist);
  return h("div", { class: "page" }, title(W.playlists, true, create), grid((p.playlists ?? []).map(card)));
}

export async function playlist(id: string): Promise<HTMLElement> {
  const p = await read<{ playlist: Playlist; songs: Song[]; seconds: number }>({ kind: "playlist", id });
  const songs = p.songs ?? [];
  const ids = songs.map((s) => s.id);
  const moved = (i: number, to: number) => {
    const order = [...ids];
    order.splice(to, 0, ...order.splice(i, 1));
    return change({ kind: "reorderPlaylist", id, songIds: order });
  };
  const edits: Extra = (_, i) => [
    ...(i > 0 ? [{ label: W.moveUp, icon: "up", run: () => moved(i, i - 1) }] : []),
    ...(i < songs.length - 1 ? [{ label: W.moveDown, icon: "down", run: () => moved(i, i + 1) }] : []),
    { label: W.removeFromPlaylist, icon: "close", danger: true, run: () => change({ kind: "removeFromPlaylist", id, index: i }) },
  ];
  const more = () =>
    sheet(p.playlist.name, [
      {
        label: W.rename,
        run: async () => {
          const name = await askText(W.playlistName, p.playlist.name);
          if (name) await change({ kind: "renamePlaylist", id, name });
        },
      },
      {
        label: W.deletePlaylist,
        danger: true,
        run: async () => {
          if (!confirm(W.deleteConfirm(p.playlist.name))) return;
          await ask("change", JSON.stringify({ kind: "deletePlaylist", id }));
          location.hash = "#/playlists";
        },
      },
    ]);
  return h(
    "div",
    { class: "page detail" },
    h("button", { class: "back", "aria-label": "Back", on: { click: () => history.back() } }, icon("back")),
    h("div", { class: "hero" }, cover(p.playlist.cover, "cover hero-cover"), h("div", { class: "hero-text" }, h("h1", null, p.playlist.name), h("span", { class: "sub" }, `${W.songCount(songs.length)} · ${W.minutes(p.seconds ?? 0)}`), h("div", { class: "hero-actions" }, playButtons(songs, "Playlist", id), iconButton("more", W.more, more, "plain")))),
    songList(songs, "Playlist", id, false, edits),
  );
}

async function newPlaylist() {
  const name = await askText(W.playlistName);
  if (name) await change({ kind: "createPlaylist", name, songIds: [] });
}

export async function genres(): Promise<HTMLElement> {
  const p = await read<{ genres: Genre[] }>({ kind: "genres" });
  const list = (p.genres ?? []).filter((g) => g.songCount > 0).sort((a, b) => a.value.localeCompare(b.value));
  return h(
    "div",
    { class: "page" },
    title(W.genres, true),
    h("ul", { class: "genres" }, ...list.map((g) => h("li", null, h("a", { href: `#/genre/${encodeURIComponent(g.value)}` }, h("span", { class: "title" }, g.value), h("span", { class: "sub" }, W.songCount(g.songCount)))))),
  );
}

export async function genre(name: string): Promise<HTMLElement> {
  const p = await read<{ songs: Song[] }>({ kind: "genre", name });
  const songs = p.songs ?? [];
  return h("div", { class: "page" }, title(name, true), playButtons(songs, "Genre", name), songList(songs, "Genre", name));
}

export async function songs(): Promise<HTMLElement> {
  const p = await read<{ songs: Song[] }>({ kind: "songs", offset: 0, count: 300 });
  const list = p.songs ?? [];
  return h("div", { class: "page" }, title(W.songs, true), playButtons(list, "Songs", ""), songList(list, "Songs", ""));
}

export async function favorites(): Promise<HTMLElement> {
  const p = await read<{ artists: Artist[]; albums: Album[]; songs: Song[] }>({ kind: "starred" });
  const songs = p.songs ?? [];
  return h(
    "div",
    { class: "page" },
    title(W.favorites, true),
    (p.artists ?? []).length ? h("h2", { class: "section-title" }, W.artists) : null,
    (p.artists ?? []).length ? h("div", { class: "row" }, ...p.artists.map(artistCard)) : null,
    (p.albums ?? []).length ? h("h2", { class: "section-title" }, W.albums) : null,
    (p.albums ?? []).length ? grid(p.albums.map((a) => albumCard(a))) : null,
    songs.length ? h("h2", { class: "section-title" }, W.songs) : null,
    songs.length ? songList(songs, "", "") : h("p", { class: "empty" }, W.empty),
  );
}

let searchTimer: number | undefined;

export async function search(query: string): Promise<HTMLElement> {
  const results = h("div", { class: "results" });
  const field = h("input", { type: "search", class: "search-field", placeholder: W.searchHint, value: query, "aria-label": W.search, autofocus: true }) as HTMLInputElement;
  const run = async (q: string) => {
    if (!q.trim()) {
      results.replaceChildren();
      return;
    }
    const p = await read<{ artists: Artist[]; albums: Album[]; songs: Song[] }>({ kind: "search", query: q });
    facts.searched = { query: q, artists: p.artists?.length ?? 0, albums: p.albums?.length ?? 0, songs: p.songs?.length ?? 0 };
    if (field.value !== q) return;
    const none = !p.artists?.length && !p.albums?.length && !p.songs?.length;
    results.replaceChildren(
      none ? h("p", { class: "empty" }, W.noResults) : "",
      p.artists?.length ? h("section", null, h("h2", { class: "section-title" }, W.artists), h("div", { class: "row" }, ...p.artists.map(artistCard))) : "",
      p.albums?.length ? h("section", null, h("h2", { class: "section-title" }, W.albums), h("div", { class: "row" }, ...p.albums.map((a) => albumCard(a)))) : "",
      p.songs?.length ? h("section", null, h("h2", { class: "section-title" }, W.songs), songList(p.songs, "Search", q)) : "",
    );
  };
  field.oninput = () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      history.replaceState(null, "", `#/search/${encodeURIComponent(field.value)}`);
      void run(field.value);
    }, 250) as unknown as number;
  };
  void run(query);
  setTimeout(() => field.focus(), 0);
  return h("div", { class: "page" }, title(W.search), field, results);
}

export async function settings(signOut: () => void): Promise<HTMLElement> {
  const names = "autoMix,crossfadeSec,speed,pitch,autoMixKeepPitch";
  let v: Record<string, string> = JSON.parse(await ask<string>("settings", names));
  const profile = JSON.parse(await ask<string>("profile")) as { url: string; user: string } | null;
  const body = h("div", { class: "settings" });
  const jamBox = h("div", { class: "group jam-group" });
  const drawJam = () => void panel(drawJam).then((p) => jamBox.replaceChildren(p));
  drawJam();
  const set = async (name: string, value: string) => {
    await ask("setting", name, value);
    v = JSON.parse(await ask<string>("settings", names));
    draw();
  };
  const toggle = (name: string, label: string, lead?: string) =>
    h(
      "label",
      { class: "setting" },
      h("div", null, h("span", { class: "title" }, label), lead ? h("span", { class: "sub" }, lead) : null),
      h("input", { type: "checkbox", class: "switch", checked: v[name] === "true", on: { change: (e) => void set(name, (e.target as HTMLInputElement).checked ? "true" : "false") } }),
    );
  const choice = (name: string, label: string, values: string[], word: (x: string) => string) =>
    h(
      "label",
      { class: "setting" },
      h("span", { class: "title" }, label),
      h("select", { on: { change: (e) => void set(name, (e.target as HTMLSelectElement).value) } }, ...values.map((x) => h("option", { value: x, selected: Number(x) === Number(v[name]) }, word(x)))),
    );
  const draw = () =>
    body.replaceChildren(
      h("h2", { class: "section-title" }, W.sound),
      h(
        "div",
        { class: "group" },
        toggle("autoMix", W.autoMix, W.autoMixLead),
        choice("crossfadeSec", W.crossfade, ["0", "2", "4", "6", "8", "12"], (x) => (x === "0" ? W.crossfadeOff : W.seconds(Number(x)))),
        choice("speed", W.speed, ["0.75", "0.9", "1", "1.1", "1.25", "1.5", "2"], (x) => `${x}×`),
        choice("pitch", W.pitch, ["0.9", "0.95", "1", "1.05", "1.1"], (x) => `${x}×`),
        toggle("autoMixKeepPitch", W.keepPitch),
      ),
      h("h2", { class: "section-title" }, W.jam),
      jamBox,
      h("h2", { class: "section-title" }, W.account),
      h("div", { class: "group" }, h("div", { class: "setting" }, h("span", { class: "sub" }, profile ? W.signedInAs(profile.user, profile.url) : ""), h("button", { class: "pill", on: { click: signOut } }, W.signOut))),
      h("p", { class: "about" }, W.aboutLead),
    );
  draw();
  return h("div", { class: "page" }, title(W.settings), body);
}
