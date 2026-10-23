<script lang="ts">
  // One of Home's rows in full: its albums, every page of them, or its playlists, as a grid of the shelf's cards.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route } from "../state/route.svelte";
  import type { Playlist } from "../state/types";
  import { W } from "../words";
  import AlbumGrid from "../ui/AlbumGrid.svelte";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";
  import type { HomeRow } from "./Home.svelte";

  const name = $derived(route.arg);
  const row = reading("row", async () => {
    const r = (JSON.parse(await ask<string>("home_rows")) as HomeRow[]).find((x) => x.row === route.arg);
    if (r?.kind !== "playlists") return { row: r, playlists: [] as Playlist[] };
    return { row: r, playlists: (await read<{ playlists?: Playlist[] }>({ kind: "playlists" })).playlists ?? [] };
  });
  const r = $derived(row.data?.row);
</script>

<Page title={W.homeRows[name] ?? name} back ready={!!row.data} error={row.error}>
  {#if r?.kind === "albums"}<AlbumGrid sort={r.sort} top="row" />
  {:else if r?.kind === "playlists"}<Cards playlists={row.data?.playlists} />{/if}
</Page>
