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
  {#snippet head()}
    <select class="field sort" aria-label={W.sortBy} onchange={(e) => go("albums", e.currentTarget.value)}>
      {#each Object.entries(W.sorts) as [k, v] (k)}<option value={k} selected={k === sort}>{v}</option>{/each}
    </select>
  {/snippet}
  <Cards albums={page.data ?? []} />
</Page>

<style>
  .sort {
    width: auto;
    height: 32px;
    font-size: 13px;
  }
</style>
