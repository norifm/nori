<script lang="ts">
  // Every album in the order `sort` (the server's name for it), read a page after another once the first
  // is in: a grid of cards or, in the Artist order (`artistRows`), a shelf per artist under their name and
  // how many, the name a way to the artist. `head` goes above (a page's places and orders).
  import type { Snippet } from "svelte";
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Album } from "../state/types";
  import { W } from "../words";
  import Cards from "./Cards.svelte";
  import Icon from "./Icon.svelte";

  let { sort, top, head }: { sort: string; top: string; head?: Snippet } = $props();
  const page = (offset: number) => read<{ albums?: Album[]; next: number | null }>({ kind: "albums", sort, size: 500, offset });
  const first = reading(top, () => page(0));
  /** The pages after the first. */
  let rest = $state.raw<Album[]>([]);
  $effect(() => {
    const p = first.data;
    rest = [];
    let gone = false;
    void (async () => {
      // An album the server moved between two pages comes once.
      const seen = new Set(p?.albums?.map((a) => a.id));
      for (let next = p?.next; next != null && !gone; ) {
        const more = await page(next);
        if (gone) return;
        rest = [...rest, ...(more.albums ?? []).filter((a) => !seen.has(a.id) && !!seen.add(a.id))];
        next = more.next;
      }
    })();
    return () => {
      gone = true;
    };
  });
  const albums = $derived([...(first.data?.albums ?? []), ...rest]);
  /** In the Artist order: where each artist's shelf starts (the core's `artist_rows`). */
  let starts = $state.raw<number[]>([]);
  $effect(() => {
    const all = albums;
    if (sort !== "artistRows" || !all.length) return void (starts = []);
    let gone = false;
    void ask<string>("artist_rows", JSON.stringify(all.map((a) => ({ id: a.id, artist: a.artist, artistId: a.artistId })))).then((s) => !gone && (starts = JSON.parse(s)));
    return () => {
      gone = true;
    };
  });
  const shelves = $derived(starts.map((s, i) => albums.slice(s, starts[i + 1] ?? albums.length)));
</script>

{@render head?.()}
{#if first.error}<p class="empty">{W.failed(first.error)}</p>
{:else if first.data && !albums.length}<p class="empty"><Icon name="compact-disc" />{W.noAlbums}</p>
{:else if sort === "artistRows"}
  {#each shelves as row (row[0].id)}
    <section class="artist">
      {#if row[0].artistId}
        <a class="name" href={href("artist", row[0].artistId)}><span class="clip">{row[0].artist}</span><span class="count">{W.albumCount(row.length)}</span><Icon name="nav-arrow-right" /></a>
      {:else}
        <span class="name"><span class="clip">{row[0].artist}</span><span class="count">{W.albumCount(row.length)}</span></span>
      {/if}
      <Cards albums={row} row sub={(a) => (a.year ? String(a.year) : "")} />
    </section>
  {/each}
{:else}
  <Cards {albums} />
{/if}

<style>
  .artist {
    margin-bottom: 22px;
  }
  .artist :global(.shelf) {
    grid-auto-columns: 112px;
  }
  .name {
    display: flex;
    align-items: center;
    padding-bottom: 12px;
    font-size: 17px;
    line-height: 22px;
    font-weight: 600;
  }
  .count {
    flex: 1;
    padding-left: 8px;
    font-size: 12.5px;
    font-weight: 400;
    color: var(--ink2);
    white-space: nowrap;
  }
  .name :global(.icon) {
    width: 20px;
    height: 20px;
    color: var(--ink2);
  }
</style>
