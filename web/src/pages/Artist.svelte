<script lang="ts">
  // An artist: its hero ("14 releases · 162 songs", Add to queue), its releases a shelf per kind as the
  // core grouped them (`release_groups`), each title opening that shelf as a grid, then the songs its
  // provider lists. `#/artist/<id>/<n>` is shelf n in full.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { href, route } from "../state/route.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Hero from "../ui/Hero.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import Seal from "../ui/Seal.svelte";
  import SectionTitle from "../ui/SectionTitle.svelte";
  import SongList from "../ui/SongList.svelte";

  type Group = { kind: string; tag: string; albums: number[] };
  const page = reading("artist", async () => {
    const d = await read<{ artist: Artist; albums: Album[]; songs: Song[]; few: number; groups: Group[] }>({ kind: "artist", id: route.arg });
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
  const shelf = $derived(route.more === "" ? null : shelves[Number(route.more)]);
</script>

{#if shelf}
  <Page title={shelf.name} back><Cards albums={shelf.albums} /></Page>
{:else}
  <Hero cover={a?.cover} title={a?.name ?? ""} big fallback="user" ready={!!a} error={page.error} {label} onplay={(shuffle) => void player.playArtist(route.arg, shuffle)}>
    {#snippet actions()}{#if a && !remote.jam?.guest}<Seal what="artist" id={a.id} on={!!a.starred} />{/if}{/snippet}
    {#snippet tools()}<PageTools tools={page.data?.tools ?? []} artist={a?.id} />{/snippet}
    {#each shelves as s, n (n)}
      <SectionTitle title={s.name} href={href("artist", route.arg, String(n))} />
      <Cards albums={s.albums} row />
    {/each}
    {#if songs.length}
      <SectionTitle title={W.songs} />
      <SongList {songs} kind="Artist" id={a?.id} shown={page.data?.few} />
    {/if}
  </Hero>
{/if}
