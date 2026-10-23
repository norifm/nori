<script lang="ts">
  // Every album, in the order picked: the core's orders as chips (`album_sorts`; the one left last when the
  // address names none, kept by the core), Shuffle (random albums, each whole), then the albums: a grid, or
  // a shelf per artist in the Artist order.
  import { ask } from "../rpc";
  import { player } from "../state/player.svelte";
  import { go } from "../state/route.svelte";
  import { W } from "../words";
  import AlbumGrid from "../ui/AlbumGrid.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  let { sort: named }: { sort: string } = $props();
  let sorts = $state<string[]>([]);
  let kept = $state("");
  void Promise.all([ask<string>("album_sorts"), ask<string>("album_sort")]).then(([all, k]) => {
    sorts = JSON.parse(all);
    kept = k;
  });
  const sort = $derived(named || kept);

  function choose(s: string) {
    void ask("keep_album_sort", s);
    go("albums", s);
  }
</script>

<Page title={W.albums} back>
  {#snippet head()}<button class="pill quiet small" onclick={() => void player.shuffleLibrary(true)}><Icon name="shuffle" />{W.shuffle}</button>{/snippet}
  {#if sort}
    <AlbumGrid {sort} top="albums">
      {#snippet head()}
        <div class="chips" role="radiogroup" aria-label={W.sortBy}>
          {#each sorts as s (s)}
            <button class="chip" class:on={s === sort} role="radio" aria-checked={s === sort} onclick={() => choose(s)}>{W.albumSorts[s] ?? s}</button>
          {/each}
        </div>
      {/snippet}
    </AlbumGrid>
  {/if}
</Page>

<style>
  .chips {
    flex-wrap: wrap;
    margin: 0 0 18px;
    padding: 0;
  }
</style>
