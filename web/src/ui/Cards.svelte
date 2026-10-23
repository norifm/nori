<script lang="ts">
  // Albums, artists or playlists as cards: a grid filling the page, or a shelf of one row that scrolls
  // sideways. An album's line is the core's (`subtitle`: its artist and year).
  import { href } from "../state/route.svelte";
  import { W } from "../words";
  import type { Album, Artist, Playlist } from "../state/types";
  import Card from "./Card.svelte";
  import Icon from "./Icon.svelte";

  let { albums = [], artists = [], playlists = [], row = false, sub }: { albums?: Album[]; artists?: Artist[]; playlists?: Playlist[]; row?: boolean; sub?: (a: Album) => string } = $props();
</script>

{#if albums.length === 0 && artists.length === 0 && playlists.length === 0}
  <p class="empty"><Icon name="music-note" />{W.empty}</p>
{:else}
  <div class={row ? "row" : "grid"}>
    {#each artists as a (a.id)}
      <Card href={href("artist", a.id)} cover={a.cover} title={a.name} round fallback="user" />
    {/each}
    {#each albums as a (a.id)}
      <Card href={href("album", a.id)} cover={a.cover} title={a.name} sub={sub ? sub(a) : (a.subtitle ?? a.artist)} />
    {/each}
    {#each playlists as p (p.id)}
      <Card href={href("playlist", p.id)} cover={p.cover} title={p.name} sub={p.songCount ? W.songCount(p.songCount) : (p.comment ?? "")} fallback="playlist" />
    {/each}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
    gap: 22px var(--grid-gap);
  }
  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 176px;
    gap: var(--grid-gap);
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter) 6px;
    scroll-padding: 0 var(--gutter);
    scrollbar-width: none;
  }
  .row::-webkit-scrollbar {
    display: none;
  }
  .row > :global(*) {
    scroll-snap-align: start;
  }
  @media (min-width: 1600px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(208px, 1fr));
    }
  }
  @media (max-width: 719px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
      gap: 18px var(--grid-gap);
    }
    .row {
      grid-auto-columns: 148px;
    }
  }
</style>
