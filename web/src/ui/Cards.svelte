<script lang="ts">
  // Albums or artists as cards: a grid filling the page, or a shelf of one row that scrolls sideways.
  import { href } from "../state/route.svelte";
  import { W } from "../words";
  import type { Album, Artist } from "../state/types";
  import Card from "./Card.svelte";

  let { albums = [], artists = [], row = false, sub }: { albums?: Album[]; artists?: Artist[]; row?: boolean; sub?: (a: Album) => string } = $props();
</script>

{#if albums.length === 0 && artists.length === 0}
  <p class="empty">{W.empty}</p>
{:else}
  <div class={row ? "row" : "grid"}>
    {#each artists as a (a.id)}
      <Card href={href("artist", a.id)} cover={a.cover} title={a.name} round fallback="artist" />
    {/each}
    {#each albums as a (a.id)}
      <Card href={href("album", a.id)} cover={a.cover} title={a.name} sub={sub ? sub(a) : a.artist} />
    {/each}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
    gap: 22px 16px;
  }
  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 176px;
    gap: 16px;
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    margin: 0 calc(-1 * var(--page-pad));
    padding: 0 var(--page-pad) 6px;
    scroll-padding: 0 var(--page-pad);
    scrollbar-width: none;
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
      gap: 18px 14px;
    }
    .row {
      grid-auto-columns: 148px;
      gap: 14px;
    }
  }
</style>
