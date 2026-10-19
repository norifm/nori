<script lang="ts">
  // What has a seal: artists, albums and songs.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  const page = reading("favorites", () => read<{ artists: Artist[]; albums: Album[]; songs: Song[] }>({ kind: "starred" }));
  const d = $derived(page.data);
</script>

<Page title={W.favorites} back ready={!!d} error={page.error}>
  {#if d}
    {#if d.artists?.length}<h2 class="section">{W.artists}</h2><Cards artists={d.artists} row />{/if}
    {#if d.albums?.length}<h2 class="section">{W.albums}</h2><Cards albums={d.albums} />{/if}
    {#if d.songs?.length}<h2 class="section">{W.songs}</h2><SongList songs={d.songs} />{/if}
    {#if !d.artists?.length && !d.albums?.length && !d.songs?.length}<p class="empty">{W.empty}</p>{/if}
  {/if}
</Page>
