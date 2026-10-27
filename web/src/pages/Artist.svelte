<script lang="ts">
  // An artist, as the desktop's page: the picture across the head with the name, Play and Shuffle (every
  // album of theirs) over its foot; the list's head ("14 releases · 162 songs" and the page's glyphs); its
  // releases by kind as the core grouped them (`release_groups`), a shelf each; then the songs its provider
  // lists.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import ArtistHead from "../ui/ArtistHead.svelte";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  type Group = { kind: string; tag: string; albums: number[] };
  const page = reading("artist", async () => {
    const d = await read<{ artist: Artist; picture?: string; albums: Album[]; songs: Song[]; few: number; groups: Group[] }>({ kind: "artist", id });
    return { ...d, tools: JSON.parse(await ask<string>("page_actions", "Artist", d.songs.length)) as string[] };
  });
  const a = $derived(page.data?.artist);
  const albums = $derived(page.data?.albums ?? []);
  const songs = $derived(page.data?.songs ?? []);
  const shelves = $derived((page.data?.groups ?? []).map((g) => ({ name: W.shelf(g.kind, g.tag), albums: g.albums.map((i) => albums[i]) })));
  const label = $derived.by(() => {
    if (!albums.length) return songs.length ? W.songCount(songs.length) : "";
    const n = albums.reduce((s, x) => s + (x.songCount || 0), 0);
    return [W.releases(albums.length), n ? W.songCount(n) : ""].filter(Boolean).join(" · ");
  });
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
    <ArtistHead name={a.name} cover={a.cover} picture={page.data?.picture} onplay={(s) => void player.playWhole("artist", id, s)}>
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="artist" id={a.id} on={!!a.starred} />{/if}
      {/snippet}
    </ArtistHead>
    <div class="list-head">
      <span class="caption">{label}</span>
      <PageTools tools={page.data?.tools ?? []} artist={a.id} />
    </div>
    {#each shelves as s, n (n)}
      <h2 class="section">{s.name}</h2>
      <Cards albums={s.albums} row />
    {/each}
    {#if songs.length}<h2 class="section">{W.songs}</h2><SongList {songs} kind="Artist" id={a.id} shown={page.data?.few} />{/if}
  {/if}
</Page>

<style>
  /* What the artist has and the page's glyphs, a hairline under, as the desktop's list head. */
  .list-head {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 48px;
    margin-top: 18px;
    border-bottom: 1px solid var(--line);
  }
  .caption {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--ink2);
  }
</style>
