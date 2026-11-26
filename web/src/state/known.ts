// What the card that opened a page knew of it: the page draws its head from it (title, cover, colors)
// while its own read is out, as Android's pages do. The last few of each kind are kept, so going back
// and forward between pages finds them.
import type { Album, Artist, Playlist } from "./types";

const KEPT = 8;

type Kinds = { album: Album; artist: Artist; playlist: Playlist };

const kept: { [K in keyof Kinds]: Map<string, Kinds[K]> } = { album: new Map(), artist: new Map(), playlist: new Map() };

/** Keeps `item`, the card of `kind` just opened. */
export function know<K extends keyof Kinds>(kind: K, item: Kinds[K]) {
  const m = kept[kind] as Map<string, Kinds[K]>;
  m.delete(item.id);
  m.set(item.id, item);
  if (m.size > KEPT) m.delete(m.keys().next().value!);
}

/** What the card that opened page `id` of `kind` knew, if one did. */
export function known<K extends keyof Kinds>(kind: K, id: string): Kinds[K] | undefined {
  return (kept[kind] as Map<string, Kinds[K]>).get(id);
}
