<script lang="ts">
  // An artist: its head ("14 releases · 162 songs"; Play and Shuffle play every album of theirs), the songs
  // its provider lists, and its releases by kind as the core grouped them (`release_groups`).
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Hero from "../ui/Hero.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  type Group = { kind: string; tag: string; albums: number[] };
  const page = reading("artist", async () => {
    const d = await read<{ artist: Artist; albums: Album[]; songs: Song[]; few: number; groups: Group[] }>({ kind: "artist", id });
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
    <Hero cover={a.cover} title={a.name} round fallback="user" {label} onplay={(s) => void player.playArtist(id, s)}>
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="artist" id={a.id} on={!!a.starred} />{/if}
        <PageTools tools={page.data?.tools ?? []} artist={a.id} />
      {/snippet}
    </Hero>
    {#if songs.length}<h2 class="section">{W.songs}</h2><SongList {songs} kind="Artist" id={a.id} shown={page.data?.few} />{/if}
    {#each shelves as s, n (n)}
      <h2 class="section">{s.name}</h2>
      <Cards albums={s.albums} />
    {/each}
  {/if}
</Page>
