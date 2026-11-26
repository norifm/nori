<script lang="ts">
  // An album: its head (what the list holds and what the core offers for its songs, `page_actions`), and its
  // songs by number, a disc at a time as the core laid them out. Its head shows at once from what the card that
  // opened it knew (`known`).
  import { ask, read } from "../rpc";
  import { known } from "../state/known";
  import { reading } from "../state/library.svelte";
  import { addToPlaylist, itemMark } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { href } from "../state/route.svelte";
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
  // The answer while it is this page's: the page stays mounted when another album opens over it.
  const data = $derived(page.data?.album.id === id ? page.data : undefined);
  // Until the album is read, what the card that opened it knew.
  const a = $derived(data?.album ?? known("album", id));
  const songs = $derived(data?.songs ?? []);
  const label = $derived(
    data
      ? [data.album.year || "", W.songCount(songs.length), W.time(data.seconds * 1000), data.quality ?? ""].filter(Boolean).join(" · ")
      : a
        ? [a.year || "", a.songCount ? W.songCount(a.songCount) : "", a.duration ? W.time(a.duration * 1000) : ""].filter(Boolean).join(" · ")
        : "",
  );
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
  <!-- A list of a title and a time keeps a reading width, its head with it, where a wide window would set
       them a screen apart. -->
  <div class="reading">
    <Hero cover={a.cover} title={a.name} {label} item={itemMark("album", a.id, { title: a.name, line: a.artist, cover: a.cover })} playable={!!data} onplay={(shuffle) => void player.play(songs, 0, shuffle, "Album", a.id)}>
      {#snippet by()}{#if a.artistId}<a href={href("artist", a.artistId)}>{a.artist}</a>{:else}{a.artist}{/if}{/snippet}
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="album" id={a.id} on={!!a.starred} />{/if}
        {#if data}
          <PageTools tools={data.tools} {songs} />
          <button class="glyph" aria-label={W.more} title={W.more} onclick={() => void addToPlaylist(songs)}><Icon name="more-horiz" /></button>
        {/if}
      {/snippet}
    </Hero>
    {#each data?.discs ?? [] as disc (disc.disc)}
      {#if disc.headed}<h2 class="section">{disc.title ? W.discTitled(disc.disc, disc.title) : W.disc(disc.disc)}</h2>{/if}
      <SongList {songs} kind="Album" id={a.id} numbered rows={disc.songs} />
    {/each}
  </div>
  {/if}
</Page>

<style>
  .reading {
    max-width: 960px;
  }
</style>
