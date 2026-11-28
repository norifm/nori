<script lang="ts">
  // Albums, artists or playlists as cards: a grid filling the page, or a shelf of one row that scrolls
  // sideways (`Shelf`). An album's line is the core's (`subtitle`: its artist and year). A card opened is
  // kept for its page's head (`known`).
  import { know } from "../state/known";
  import { itemHref } from "../state/route.svelte";
  import { W } from "../words";
  import type { Album, Artist, Playlist } from "../state/types";
  import Card from "./Card.svelte";
  import Icon from "./Icon.svelte";
  import Shelf from "./Shelf.svelte";
  import { come, settle } from "./motion";

  let { albums = [], artists = [], playlists = [], row = false, sub }: { albums?: Album[]; artists?: Artist[]; playlists?: Playlist[]; row?: boolean; sub?: (a: Album) => string } = $props();
  const key = (id: string, source?: string | null) => (source ? `${id}\x1f${source}` : id);
</script>

{#if albums.length === 0 && artists.length === 0 && playlists.length === 0}
  <p class="empty"><Icon name="music-note" />{W.empty}</p>
{:else if row}
  <Shelf count={albums.length + artists.length + playlists.length}>{@render cells()}</Shelf>
{:else}
  <div class="grid">{@render cells()}</div>
{/if}

{#snippet cells()}
  {#each artists as a (key(a.id, a.source))}
    <!-- How many albums: two artists of one name are told apart. -->
    <div class="cell" animate:settle transition:come><Card href={itemHref("artist", a.id, a.source)} kind="artist" id={a.id} cover={a.cover} title={a.name} sub={a.albumCount ? W.albumCount(a.albumCount) : ""} round fallback="user" remote={a.isExternal} onopen={() => know("artist", a)} /></div>
  {/each}
  {#each albums as a (key(a.id, a.source))}
    <div class="cell" animate:settle transition:come><Card href={itemHref("album", a.id, a.source)} kind="album" id={a.id} cover={a.cover} title={a.name} sub={sub ? sub(a) : (a.subtitle ?? a.artist)} remote={a.isExternal} onopen={() => know("album", a)} /></div>
  {/each}
  {#each playlists as p (key(p.id, p.source))}
    <div class="cell" animate:settle transition:come><Card href={itemHref("playlist", p.id, p.source, !!p.mixed)} kind="playlist" id={p.id} cover={p.cover} title={p.name} sub={p.songCount ? W.songCount(p.songCount) : (p.comment ?? "")} fallback="playlist" editable={!!p.editable} remote={p.remote} onopen={() => know("playlist", p)} /></div>
  {/each}
{/snippet}

<style>
  .cell {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  /* The desktop's grid: as many columns as fit at 150 px at least, grown to fill the row. */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 24px var(--grid-gap);
  }
  @media (max-width: 719px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
      gap: 18px var(--grid-gap);
    }
  }
</style>
