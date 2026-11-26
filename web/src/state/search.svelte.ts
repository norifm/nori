// The search, one for the top bar's field and the search page: the core's search session in the worker
// (`search_typed`, `search_ask`, `search_pick`, `search_narrow`). Every key is answered from the offline
// index at once, the server once the typing pauses; which answer still stands, its layout and its chips
// are the core's.
import { ask } from "../rpc";
import type { Album, Artist, Playlist, Song } from "./types";

export type Kind = "songs" | "albums" | "artists" | "playlists";
export type Scope = "everything" | "library" | "providers";
export type Item = { kind: Kind; at: number };
export type Line = { line: "top" } | { line: "head"; kind: Kind } | { line: "artist" | "album" | "playlist" | "song"; at: number } | { line: "all" };
export type View = {
  state: "blank" | "asking" | "failed" | "found" | "nothing";
  query: string;
  artists: Artist[];
  albums: Album[];
  playlists: Playlist[];
  songs: Song[];
  preview: Line[];
  layout: { top: Item | null; sections: { kind: Kind; items: number[] }[] };
  kinds: Kind[];
  kind: Kind | null;
  scope: Scope;
  scopes: Scope[];
  fallback: string | null;
};

export const search = $state<{ view: View | null; text: string; recent: string[] }>({ view: null, text: "", recent: [] });
let timer: ReturnType<typeof setTimeout> | undefined;

/** A view from the worker; null (stale) changes nothing. */
function show(json: string | null) {
  const v: View | null = json ? JSON.parse(json) : null;
  if (v) search.view = v;
}

/** The field now holds `text`. The same text again asks nothing. */
export async function typed(text: string) {
  if (text === search.text && search.view) return;
  search.text = text;
  clearTimeout(timer);
  show(await ask<string>("search_typed", text).catch(() => "null"));
  const q = text.trim();
  if (!q) {
    search.recent = JSON.parse(await ask<string>("search_history").catch(() => "[]"));
    return;
  }
  const ms = await ask<number>("search_delay", q);
  if (search.text.trim() !== q) return;
  clearTimeout(timer);
  timer = setTimeout(async () => show(await ask<string>("search_ask", q).catch(() => "null")), ms);
}

/** Kind chip `kind` picked, null for All. */
export async function pick(kind: Kind | null) {
  show(await ask<string>("search_pick", kind).catch(() => "null"));
}

/** Scope chip `scope` tapped. */
export async function narrow(scope: Scope) {
  show(await ask<string>("search_narrow", scope).catch(() => "null"));
}

/** The query in the field is worth keeping in the recent searches: it was sent, or acted on. */
export function remember() {
  const q = search.text.trim();
  if (q) void ask("search_remember", q).catch(() => {});
}

export async function forgetSearches() {
  await ask("search_forget").catch(() => {});
  search.recent = [];
}
