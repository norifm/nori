<script lang="ts">
  // The Library's artists: its places, a field that narrows the list by name, then every artist with their
  // picture and how many albums. What the filter keeps is the core's (`filter_rows`).
  import type { Snippet } from "svelte";
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Artist } from "../state/types";
  import { W } from "../words";
  import Cover from "../ui/Cover.svelte";
  import Icon from "../ui/Icon.svelte";

  let { places }: { places: Snippet } = $props();
  const page = reading("library", () => read<{ artists: Artist[] }>({ kind: "artists" }).then((p) => p.artists ?? []));
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

{@render places()}
{#if page.error}<p class="empty">{W.failed(page.error)}</p>
{:else if page.data && !page.data.length}<p class="empty"><Icon name="user" />{W.noArtists}</p>
{:else if page.data}
  <label class="field search-field filter"><Icon name="search" /><input type="search" placeholder={W.filterArtists} aria-label={W.filterArtists} bind:value={filter} /></label>
  <ul class="rows artists">
    {#each shown as a (a.id)}
      <li>
        <a class="nav-row pictured" href={href("artist", a.id)}>
          <span class="picture"><Cover src={a.cover} round fallback="user" /></span>
          <span class="words"><span>{a.name}</span>{#if a.albumCount}<span class="sub">{W.albumCount(a.albumCount)}</span>{/if}</span>
        </a>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .filter {
    margin-bottom: 10px;
  }
  .artists .nav-row {
    padding-top: 7px;
    padding-bottom: 7px;
  }
</style>
