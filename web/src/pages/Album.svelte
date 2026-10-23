<script lang="ts">
  // An album: its head, and its songs by number.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { addToPlaylist } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { href } from "../state/route.svelte";
  import type { Album, Song } from "../state/types";
  import { W } from "../words";
  import Hero from "../ui/Hero.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  const page = reading("album", () => read<{ album: Album; songs: Song[]; seconds: number }>({ kind: "album", id }));
  const a = $derived(page.data?.album);
  const songs = $derived(page.data?.songs ?? []);
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
    <Hero cover={a.cover} title={a.name} facts={[a.genre, a.year || "", W.songCount(songs.length), W.minutes(page.data?.seconds ?? 0)].filter(Boolean).join(" · ")} onplay={(shuffle) => void player.play(songs, 0, shuffle, "Album", a.id)}>
      {#snippet by()}{#if a.artistId}<a href={href("artist", a.artistId)}>{a.artist}</a>{:else}{a.artist}{/if}{/snippet}
      {#snippet actions()}
        <Seal what="album" id={a.id} on={!!a.starred} />
        <button class="round" aria-label={W.more} title={W.more} onclick={() => void addToPlaylist(songs)}><Icon name="more-horiz" /></button>
      {/snippet}
    </Hero>
    <SongList {songs} kind="Album" id={a.id} numbered />
  {/if}
</Page>
