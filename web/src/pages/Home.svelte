<script lang="ts" module>
  /** One of Home's rows as the core lays it out (`home_rows`): its albums, or its playlists. */
  export type HomeRow = { row: string; kind: "albums"; sort: string; size: number } | { row: string; kind: "playlists"; take: number };
</script>

<script lang="ts">
  // Home: the queue another device left, offered to carry on here; then the rows kept in their order (the
  // core's, from the Home rows setting), each a shelf of prints whose title opens the whole row. A row with
  // nothing in it is not drawn.
  import { ask, facts, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href, route } from "../state/route.svelte";
  import type { Album, Playlist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Continue from "../ui/Continue.svelte";
  import Page from "../ui/Page.svelte";
  import SectionTitle from "../ui/SectionTitle.svelte";

  type Shelf = { row: string; albums: Album[]; playlists: Playlist[] };
  const page = reading("home", async () => {
    const rows = JSON.parse(await ask<string>("home_rows")) as HomeRow[];
    const shelves = await Promise.all(
      rows.map(async (r): Promise<Shelf> => {
        if (r.kind === "playlists") {
          const p = await read<{ playlists?: Playlist[] }>({ kind: "playlists" }).catch(() => ({ playlists: [] }));
          return { row: r.row, albums: [], playlists: (p.playlists ?? []).slice(0, r.take) };
        }
        const a = await read<{ albums?: Album[] }>({ kind: "albums", sort: r.sort, size: r.size, offset: 0 }).catch(() => ({ albums: [] }));
        return { row: r.row, albums: a.albums ?? [], playlists: [] };
      }),
    );
    facts.albums = shelves.reduce((n, s) => n + s.albums.length, 0);
    return shelves;
  });
</script>

<Page title={W.listenNow} ready={!!page.data} error={page.error}>
  {#key route.visit}<Continue />{/key}
  {#each page.data ?? [] as s (s.row)}
    {#if s.albums.length || s.playlists.length}
      <section class="shelf" data-row={s.row}>
        <SectionTitle title={W.homeRows[s.row] ?? s.row} href={href("row", s.row)} />
        <Cards albums={s.albums} playlists={s.playlists} row />
      </section>
    {/if}
  {/each}
</Page>
