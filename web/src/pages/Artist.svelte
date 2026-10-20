<script lang="ts">
  // An artist: its head (Play and Shuffle play every album of theirs), their top songs, and their albums.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Hero from "../ui/Hero.svelte";
  import Page from "../ui/Page.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  const page = reading("artist", () => read<{ artist: Artist; albums: Album[]; songs: Song[]; few: number }>({ kind: "artist", id }));
  const a = $derived(page.data?.artist);
  const albums = $derived(page.data?.albums ?? []);
  const songs = $derived(page.data?.songs ?? []);

  async function playAll(shuffle: boolean) {
    const all: Song[] = albums.length ? [] : [...songs];
    for (const al of albums) all.push(...((await read<{ songs: Song[] }>({ kind: "album", id: al.id })).songs ?? []));
    await player.play(all, 0, shuffle, "Artist", id);
  }
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
    <Hero cover={a.cover} title={a.name} round fallback="artist" facts={!albums.length && songs.length ? W.songCount(songs.length) : W.albumCount(albums.length)} onplay={(s) => void playAll(s)}>
      {#snippet actions()}<Seal what="artist" id={a.id} on={!!a.starred} />{/snippet}
    </Hero>
    {#if songs.length}<h2 class="section">{W.songs}</h2><SongList {songs} kind="Artist" id={a.id} shown={page.data?.few} />{/if}
    {#if albums.length}<h2 class="section">{W.albums}</h2><Cards {albums} sub={(al) => String(al.year || "")} />{/if}
  {/if}
</Page>
