<script lang="ts">
  // Every artist, narrowed by name in the field over them (what it keeps is the core's, `filter_rows`).
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import type { Artist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("artists", () => read<{ artists: Artist[] }>({ kind: "artists" }).then((p) => p.artists ?? []));
  let filter = $state("");
  let shown = $state.raw<Artist[]>([]);
  $effect(() => {
    const all = page.data ?? [];
    const f = filter;
    if (!f.trim()) return void (shown = all);
    let gone = false;
    void ask<string>("filter_rows", JSON.stringify(all.map((a) => [a.name])), f).then((r) => !gone && (shown = (JSON.parse(r) as number[]).map((i) => all[i])));
    return () => {
      gone = true;
    };
  });
</script>

<Page title={W.artists} back ready={!!page.data} error={page.error}>
  {#snippet head()}<label class="field search-field filter"><Icon name="search" /><input type="search" placeholder={W.filterArtists} aria-label={W.filterArtists} bind:value={filter} /></label>{/snippet}
  <Cards artists={shown} />
</Page>

<style>
  .filter {
    width: min(320px, 100%);
  }
</style>
