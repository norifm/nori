<script lang="ts">
  // Every album, in the order picked.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { go } from "../state/route.svelte";
  import type { Album } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";

  let { sort }: { sort: string } = $props();
  const page = reading("albums", () => read<{ albums: Album[] }>({ kind: "albums", sort, size: 500, offset: 0 }).then((p) => p.albums ?? []));
</script>

<Page title={W.albums} back ready={!!page.data} error={page.error}>
  <div class="chips" role="radiogroup" aria-label={W.sortBy}>
    {#each Object.entries(W.sorts) as [k, v] (k)}
      <button class="chip" class:on={k === sort} role="radio" aria-checked={k === sort} onclick={() => go("albums", k)}>{v}</button>
    {/each}
  </div>
  <Cards albums={page.data ?? []} />
</Page>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 18px;
  }
  .chip {
    border: 1px solid var(--outline);
    border-radius: 999px;
    padding: 5px 13px;
    color: var(--ink2);
  }
  .chip.on {
    background: var(--ink);
    border-color: var(--ink);
    color: var(--page);
  }
</style>
