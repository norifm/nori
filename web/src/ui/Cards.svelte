<script lang="ts">
  // Albums, artists or playlists as cards: a shelf of one row of prints that runs out past the gutters and
  // scrolls sideways (`row`), or a grid, two across on a phone and as many as fit on a wide screen. Home's
  // rows, their full views and an artist's shelves share the card's size.
  import { href } from "../state/route.svelte";
  import { W } from "../words";
  import type { Album, Artist, Playlist } from "../state/types";
  import Card from "./Card.svelte";

  let { albums = [], artists = [], playlists = [], row = false, sub }: { albums?: Album[]; artists?: Artist[]; playlists?: Playlist[]; row?: boolean; sub?: (a: Album) => string } = $props();
</script>

<div class={row ? "shelf" : "grid"}>
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

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--grid-gap);
  }
  .shelf {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: var(--print);
    gap: var(--grid-gap);
    overflow-x: auto;
    scroll-snap-type: x proximity;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter);
    scroll-padding: 0 var(--gutter);
    scrollbar-width: none;
  }
  .shelf::-webkit-scrollbar {
    display: none;
  }
  .shelf > :global(*) {
    scroll-snap-align: start;
  }
  @media (min-width: 720px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(var(--grid-min), 1fr));
    }
  }
</style>
