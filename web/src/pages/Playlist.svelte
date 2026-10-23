<script lang="ts">
  // A playlist: its head ("8 songs · 36:28", what the core offers for its songs, its menu: rename, delete),
  // and its songs, each movable and removable.
  import { ask, read } from "../rpc";
  import { library, reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { go } from "../state/route.svelte";
  import { ui, type Item } from "../state/ui.svelte";
  import type { Playlist, Song } from "../state/types";
  import { W } from "../words";
  import Hero from "../ui/Hero.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  const page = reading("playlist", async () => {
    const d = await read<{ playlist: Playlist; songs: Song[]; seconds: number }>({ kind: "playlist", id });
    return { ...d, tools: JSON.parse(await ask<string>("page_actions", "Playlist", d.songs.length)) as string[] };
  });
  const pl = $derived(page.data?.playlist);
  const songs = $derived(page.data?.songs ?? []);

  const moved = (i: number, to: number) => {
    const order = songs.map((s) => s.id);
    order.splice(to, 0, ...order.splice(i, 1));
    return library.change({ kind: "reorderPlaylist", id, songIds: order });
  };
  const edits = (_: Song, i: number): Item[] => [
    ...(i > 0 ? [{ label: W.moveUp, icon: "nav-arrow-up", run: () => moved(i, i - 1) }] : []),
    ...(i < songs.length - 1 ? [{ label: W.moveDown, icon: "nav-arrow-down", run: () => moved(i, i + 1) }] : []),
    { label: W.removeFromPlaylist, icon: "xmark", run: () => library.change({ kind: "removeFromPlaylist", id, index: i }) },
  ];
  const more = (e: MouseEvent) =>
    ui.drop(e, [
      {
        label: W.rename,
        run: async () => {
          const name = await ui.ask(W.playlistName, pl!.name);
          if (name) await library.change({ kind: "renamePlaylist", id, name });
        },
      },
      {
        label: W.deletePlaylist,
        run: async () => {
          if (!confirm(W.deleteConfirm(pl!.name))) return;
          await library.change({ kind: "deletePlaylist", id });
          go("playlists");
        },
      },
    ]);
</script>

<Page back ready={!!pl} error={page.error}>
  {#if pl}
    <Hero cover={pl.cover} title={pl.name} fallback="playlist" label={songs.length ? W.listCaption(songs.length, page.data?.seconds ?? 0) : ""} onplay={(s) => void player.play(songs, 0, s, "Playlist", id)}>
      {#snippet by()}{#if pl.comment}<span class="comment">{pl.comment}</span>{/if}{/snippet}
      {#snippet actions()}
        <PageTools tools={page.data?.tools ?? []} {songs} />
        <button class="glyph" aria-label={W.more} title={W.more} onclick={more}><Icon name="more-horiz" /></button>
      {/snippet}
    </Hero>
    <SongList {songs} kind="Playlist" {id} extra={edits} {moved} />
  {/if}
</Page>

<style>
  .comment {
    font-size: 15px;
    font-weight: 400;
  }
</style>
