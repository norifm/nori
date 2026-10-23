<script lang="ts">
  // An album: its head (what the list holds and what the core offers for its songs, `page_actions`), and its
  // songs by number, a disc at a time as the core laid them out.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { addToPlaylist } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { href } from "../state/route.svelte";
  import { beside } from "../state/ui.svelte";
  import type { Album, Song } from "../state/types";
  import { W } from "../words";
  import Hero from "../ui/Hero.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  type Disc = { disc: number; headed: boolean; title: string; songs: number[]; lines: string[] };
  const page = reading("album", async () => {
    const d = await read<{ album: Album; songs: Song[]; seconds: number; discs: Disc[]; quality: string | null }>({ kind: "album", id });
    return { ...d, tools: JSON.parse(await ask<string>("page_actions", "Album", d.songs.length)) as string[] };
  });
  const a = $derived(page.data?.album);
  const songs = $derived(page.data?.songs ?? []);
  const label = $derived(a ? [a.year || "", W.songsAlways(songs.length), W.time((page.data?.seconds ?? 0) * 1000), page.data?.quality ?? ""].filter(Boolean).join(" · ") : "");
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
    <Hero cover={a.cover} title={a.name} {label} onplay={(shuffle) => void player.play(songs, 0, shuffle, "Album", a.id)}>
      {#snippet by()}{#if a.artistId}<a href={href("artist", a.artistId)}>{a.artist}</a>{:else}{a.artist}{/if}{/snippet}
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="album" id={a.id} on={!!a.starred} />{/if}
        <PageTools tools={page.data?.tools ?? []} {songs} />
        <button class="glyph" aria-label={W.more} title={W.more} onclick={(e) => void addToPlaylist(songs, beside(e))}><Icon name="more-horiz" /></button>
      {/snippet}
    </Hero>
    {#each page.data?.discs ?? [] as disc (disc.disc)}
      {#if disc.headed}<h2 class="section">{disc.title ? W.discTitled(disc.disc, disc.title) : W.disc(disc.disc)}</h2>{/if}
      <SongList {songs} kind="Album" id={a.id} numbered rows={disc.songs} />
    {/each}
  {/if}
</Page>
