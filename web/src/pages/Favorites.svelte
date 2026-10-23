<script lang="ts">
  // What carries the seal: the albums as a shelf, the artists as rows, then the songs.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Cover from "../ui/Cover.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  const page = reading("favorites", () => read<{ artists: Artist[]; albums: Album[]; songs: Song[] }>({ kind: "starred" }));
  const d = $derived(page.data);
</script>

<Page title={W.favorites} back ready={!!d} error={page.error}>
  {#if d}
    {#if d.albums?.length}<div class="albums"><Cards albums={d.albums} row /></div>{/if}
    {#if d.artists?.length}
      <ul class="rows">
        {#each d.artists as a (a.id)}
          <li><a class="nav-row pictured" href={href("artist", a.id)}><span class="picture"><Cover src={a.cover} round fallback="user" /></span><span class="words">{a.name}</span><Icon name="nav-arrow-right" /></a></li>
        {/each}
      </ul>
    {/if}
    {#if d.songs?.length}<SongList songs={d.songs} />{/if}
    {#if !d.artists?.length && !d.albums?.length && !d.songs?.length}<p class="empty"><Icon name="seal" />{W.noFavoriteSongs}</p>{/if}
  {/if}
</Page>

<style>
  .albums {
    margin: 4px 0 12px;
  }
</style>
