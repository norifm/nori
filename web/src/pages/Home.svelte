<script lang="ts" module>
  /** One of Home's rows as the core lays it out (`home_rows`): its albums, or its playlists. */
  export type HomeRow = { row: string; kind: "albums"; sort: string; size: number } | { row: string; kind: "playlists"; take: number };
</script>

<script lang="ts">
  // Home: the queue another device left, offered to carry on here; then "For you", the mixes made from
  // what plays here and the favourites (as the desktop's and Android's Home: at once as last drawn, then
  // brought up to date); then the rows kept in their order (the core's, from the Home rows setting), each
  // a shelf of prints whose title opens the whole row. A row with nothing in it is not drawn.
  import { ask, facts, read } from "../rpc";
  import { library, reading } from "../state/library.svelte";
  import { looks, today } from "../state/theme.svelte";
  import { href, route } from "../state/route.svelte";
  import type { Album, MixTile, Playlist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Continue from "../ui/Continue.svelte";
  import MixCard from "../ui/MixCard.svelte";
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

  let mixes = $state.raw<MixTile[]>([]);
  $effect(() => {
    void route.visit;
    void library.rev;
    void looks.rev;
    let gone = false;
    const read = async (warm: boolean) => {
      const tiles = JSON.parse(await ask<string>("mixes", warm, ...today())) as MixTile[];
      if (!gone) mixes = tiles;
    };
    void read(false).then(() => (gone ? undefined : read(true)));
    return () => {
      gone = true;
    };
  });
</script>

<Page title={W.listenNow} ready={!!page.data} error={page.error}>
  {#key route.visit}<Continue />{/key}
  {#if mixes.length}
    <section class="shelf" data-row="ForYou">
      <SectionTitle title={W.forYou} />
      <div class="mixes">
        {#each mixes as m (m.id)}<MixCard mix={m} />{/each}
      </div>
    </section>
  {/if}
  {#each page.data ?? [] as s (s.row)}
    {#if s.albums.length || s.playlists.length}
      <section class="shelf" data-row={s.row}>
        <SectionTitle title={W.homeRows[s.row] ?? s.row} href={href("row", s.row)} />
        <Cards albums={s.albums} playlists={s.playlists} row />
      </section>
    {/if}
  {/each}
</Page>

<style>
  /* One row of mix cards that scrolls sideways, as the shelves of prints. */
  .mixes {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 168px;
    gap: var(--grid-gap);
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter) 6px;
    scroll-padding: 0 var(--gutter);
    scrollbar-width: none;
  }
  .mixes::-webkit-scrollbar {
    display: none;
  }
  .mixes > :global(*) {
    scroll-snap-align: start;
  }
  @media (max-width: 719px) {
    .mixes {
      grid-auto-columns: 148px;
    }
  }
</style>
