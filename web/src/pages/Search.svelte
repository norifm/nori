<script lang="ts">
  // Search over the server, read a moment after the typing stops: artists and albums as shelves, songs as
  // rows, where a click plays that one song (a search's results are not an album).
  import { facts, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route, searchFor } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SectionTitle from "../ui/SectionTitle.svelte";
  import SongList from "../ui/SongList.svelte";

  type Found = { artists: Artist[]; albums: Album[]; songs: Song[]; few: number; query?: string };
  let field = $state<HTMLInputElement>();
  const query = $derived(route.arg);

  const page = reading("search", async (): Promise<Found | null> => {
    const q = route.arg.trim();
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
  const busy = $derived(!!query.trim() && d?.query !== query.trim() && !page.error);
  $effect(() => {
    if (!ui.wide) field?.focus();
  });
</script>

<Page title={W.search} error={page.error}>
  {#if !ui.wide}
    <label class="field search-field box"><Icon name="search" /><input bind:this={field} type="search" placeholder={W.searchHint} aria-label={W.search} value={query} oninput={(e) => searchFor(e.currentTarget.value)} /></label>
  {/if}
  <div class="busy" class:on={busy} role="progressbar" aria-hidden={!busy} aria-label={W.loading}></div>
  {#if d}
    {#if d.artists?.length}<SectionTitle title={W.artists} /><div class="artists"><Cards artists={d.artists} row /></div>{/if}
    {#if d.albums?.length}<SectionTitle title={W.albums} /><div class="albums"><Cards albums={d.albums} row /></div>{/if}
    {#if d.songs?.length}<SectionTitle title={W.songs} />{#key d.query}<SongList songs={d.songs} kind="Search" id={query} shown={d.few} one />{/key}{/if}
    {#if !d.artists?.length && !d.albums?.length && !d.songs?.length}<p class="empty"><Icon name="search" />{W.noResults}</p>{/if}
  {/if}
</Page>

<style>
  .box {
    margin: 8px 0;
  }
  /* Always there, so the results never move as it shows; a sweep while the server answers. */
  .busy {
    height: 2px;
    overflow: hidden;
    position: relative;
  }
  .busy.on {
    background: var(--hair);
  }
  .busy.on::after {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 30%;
    background: var(--ink);
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  .artists :global(.shelf) {
    grid-auto-columns: 96px;
  }
  .albums :global(.shelf) {
    grid-auto-columns: 120px;
  }
</style>
