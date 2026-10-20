<script lang="ts">
  // Search over the server: artists, albums and songs, read a moment after the typing stops.
  import { facts, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route, searchFor } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  let { query }: { query: string } = $props();
  type Found = { artists: Artist[]; albums: Album[]; songs: Song[]; query?: string };
  let field = $state<HTMLInputElement>();

  const page = reading("search", async (): Promise<Found | null> => {
    const q = query.trim();
    if (!q) return null;
    // A moment after the typing stops; a later key replaces this read.
    await new Promise((r) => setTimeout(r, 200));
    if (route.arg.trim() !== q) return new Promise<never>(() => {});
    const p = await read<Found>({ kind: "search", query: q });
    facts.searched = { query: q, artists: p.artists?.length ?? 0, albums: p.albums?.length ?? 0, songs: p.songs?.length ?? 0 };
    return { ...p, query: q };
  });
  const d = $derived(page.data);
  // The results on show are for another query: the server is still answering.
  const busy = $derived(!!query?.trim() && d?.query !== query.trim() && !page.error);
  $effect(() => {
    if (!ui.wide) field?.focus();
  });
</script>

<Page title={W.search} error={page.error}>
  {#if !ui.wide}
    <input bind:this={field} type="search" class="field big" placeholder={W.searchHint} aria-label={W.search} value={query} oninput={(e) => searchFor(e.currentTarget.value)} />
  {/if}
  <div class="busy" class:on={busy} role="progressbar" aria-hidden={!busy} aria-label={W.loading}></div>
  {#if d}
    {#if !d.artists?.length && !d.albums?.length && !d.songs?.length}<p class="empty"><Icon name="search" />{W.noResults}</p>{/if}
    {#if d.artists?.length}<h2 class="section">{W.artists}</h2><Cards artists={d.artists} row />{/if}
    {#if d.albums?.length}<h2 class="section">{W.albums}</h2><Cards albums={d.albums} row />{/if}
    {#if d.songs?.length}<h2 class="section">{W.songs}</h2><SongList songs={d.songs} kind="Search" id={query} />{/if}
  {:else if !query.trim()}
    <p class="empty">{W.searchHint}</p>
  {/if}
</Page>

<style>
  /* Always there, so the results never move as it shows; a sweep while the server answers. */
  .busy.on {
    background: rgb(from var(--accent) r g b / 0.18);
  }
  .busy {
    height: 3px;
    border-radius: 2px;
    overflow: hidden;
    position: relative;
    margin-bottom: 6px;
  }
  .busy.on::after {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 30%;
    border-radius: 2px;
    background: var(--accent);
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    from { transform: translateX(-100%); }
    to { transform: translateX(340%); }
  }
  .big {
    height: 42px;
    font-size: 16px;
    margin-bottom: 8px;
  }
</style>
