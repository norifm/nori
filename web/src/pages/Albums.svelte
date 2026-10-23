<script lang="ts">
  // The Library's albums: its places, the orders as chips (the core's `album_sorts`, the one left last
  // lit and kept by the core), then the albums in that order.
  import type { Snippet } from "svelte";
  import { ask } from "../rpc";
  import { W } from "../words";
  import AlbumGrid from "../ui/AlbumGrid.svelte";

  let { places }: { places: Snippet } = $props();
  let sorts = $state<string[]>([]);
  let sort = $state("");
  void Promise.all([ask<string>("album_sorts"), ask<string>("album_sort")]).then(([all, kept]) => {
    sorts = JSON.parse(all);
    sort = kept;
  });

  function choose(s: string) {
    sort = s;
    void ask("keep_album_sort", s);
  }
</script>

{#if sort}
  <AlbumGrid {sort} top="library">
    {#snippet head()}
      {@render places()}
      <div class="chips" role="radiogroup" aria-label={W.sortBy}>
        {#each sorts as s (s)}
          <button class="chip" class:on={s === sort} role="radio" aria-checked={s === sort} onclick={() => choose(s)}>{W.albumSorts[s] ?? s}</button>
        {/each}
      </div>
    {/snippet}
  </AlbumGrid>
{/if}
