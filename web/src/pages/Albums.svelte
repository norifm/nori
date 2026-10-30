<script lang="ts">
  // Every album, in the order picked, under a bar: the order and which way it runs, which drops the core's
  // others (`album_sorts`; the one left last when the address names none, kept by the core), and Favorites
  // apart from them (only the starred, in that order); Shuffle (random albums, each whole) heads the page.
  // Then the albums: a grid, or a shelf per artist in the Artist order.
  import { ask } from "../rpc";
  import { player } from "../state/player.svelte";
  import { go } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import AlbumGrid from "../ui/AlbumGrid.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  let { sort: named }: { sort: string } = $props();
  let sorts = $state<{ name: string; way: string }[]>([]);
  let kept = $state("");
  let favorites = $state(false);
  void Promise.all([ask<string>("album_sorts"), ask<string>("album_sort")]).then(([all, k]) => {
    sorts = JSON.parse(all);
    kept = k;
  });
  const sort = $derived(named || kept);
  const way = $derived(sorts.find((s) => s.name === sort)?.way ?? "");

  function choose(s: string) {
    void ask("keep_album_sort", s);
    go("albums", s);
  }
</script>

<Page title={W.albums} back>
  {#snippet head()}<button class="pill quiet small" onclick={() => void player.shuffleLibrary(true)}><Icon name="shuffle" />{W.shuffle}</button>{/snippet}
  {#if sort}
    <AlbumGrid {sort} {favorites} top="albums">
      {#snippet head()}
        <div class="bar">
          <button
            class="chip on"
            aria-label="{W.sortBy} {W.albumSorts[sort] ?? sort}"
            onclick={(e) => ui.drop(e, sorts.map((s) => ({ label: W.albumSorts[s.name] ?? s.name, chosen: s.name === sort, run: () => choose(s.name) })))}
          >
            {W.albumSorts[sort] ?? sort}
            {#if way === "up"}<Icon name="arrow-up" />{:else if way === "down"}<Icon name="arrow-down" />{/if}
            <Icon name="nav-arrow-down" />
          </button>
          <button class="chip filter" class:on={favorites} aria-pressed={favorites} onclick={() => (favorites = !favorites)}>
            <Icon name={favorites ? "sealed" : "seal"} />{W.favorites}
          </button>
        </div>
      {/snippet}
    </AlbumGrid>
  {/if}
</Page>

<style>
  /* The order raised, the filter quiet until it is on; both edged, so the filter reads as a control. */
  .bar {
    display: flex;
    gap: 8px;
    margin: 0 0 18px;
  }
  .bar .chip {
    gap: 6px;
  }
  .bar .chip :global(svg) {
    width: 15px;
    height: 15px;
  }
  .filter {
    border: var(--hairline) solid var(--hair);
  }
  .filter.on :global(svg) {
    color: var(--accent);
  }
</style>
