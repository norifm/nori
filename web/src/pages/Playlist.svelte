<script lang="ts">
  // A playlist: its head ("8 songs · 36:28", what the core offers for its songs, its menu: rename, delete),
  // and its songs, each movable and removable; the menu and the edits only where the account may change
  // it (the core's `playlist_editable`). Deleting asks first. Its head shows at once from what the card that
  // opened it knew (`known`).
  import { ask, read } from "../rpc";
  import { known } from "../state/known";
  import { library, reading } from "../state/library.svelte";
  import { deletePlaylist, itemMark } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { go } from "../state/route.svelte";
  import { ui, type Item } from "../state/ui.svelte";
  import type { Playlist, Song } from "../state/types";
  import { W } from "../words";
  import Hero from "../ui/Hero.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import Reads from "../ui/Reads.svelte";
  import Seal from "../ui/Seal.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id, source, mixed = false }: { id: string; source?: string; mixed?: boolean } = $props();
  const page = reading("playlist", async () => {
    const d = await read<{ playlist: Playlist; songs: Song[]; seconds: number }>({ kind: "playlist", id, source, mixed });
    return { ...d, tools: JSON.parse(await ask<string>("page_actions", "Playlist", d.songs.length)) as string[] };
  });
  // The answer while it is this page's: the page stays mounted when another playlist opens over it.
  const data = $derived(page.data?.playlist.id === id ? page.data : undefined);
  // Until the playlist is read, what the card that opened it knew.
  const pl = $derived(data?.playlist ?? known("playlist", id));
  const songs = $derived(data?.songs ?? []);
  // A long list's filter (offered by the core's `filter_offered`), narrowing it by the core's `filter_rows`.
  let filter = $state("");
  let offered = $state(false);
  let rows = $state.raw<number[] | undefined>(undefined);
  $effect(() => {
    void ask<boolean>("filter_offered", songs.length, filter.trim() !== "").then((o) => (offered = o));
  });
  $effect(() => {
    const all = songs;
    const f = filter;
    if (!f.trim()) return void (rows = undefined);
    let gone = false;
    void ask<string>("filter_rows", JSON.stringify(all.map((s) => [s.title, s.artist])), f).then((r) => !gone && (rows = JSON.parse(r) as number[]));
    return () => {
      gone = true;
    };
  });

  const moved = (i: number, to: number) => {
    const order = songs.map((s) => s.id);
    order.splice(to, 0, ...order.splice(i, 1));
    return library.change({ kind: "reorderPlaylist", id, songIds: order });
  };
  const edit: Record<string, (i: number) => Item> = {
    MoveUp: (i) => ({ label: W.moveUp, icon: "nav-arrow-up", run: () => moved(i, i - 1) }),
    MoveDown: (i) => ({ label: W.moveDown, icon: "nav-arrow-down", run: () => moved(i, i + 1) }),
    Remove: (i) => ({ label: W.removeFromPlaylist, icon: "xmark", run: () => library.change({ kind: "removeFromPlaylist", id, index: i }) }),
  };
  const edits = async (_: Song, i: number): Promise<Item[]> =>
    (JSON.parse(await ask<string>("playlist_edits", i, songs.length, !!pl?.editable)) as string[]).map((e) => edit[e](i));
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
          if (await deletePlaylist(id, pl!.name)) go("playlists");
        },
      },
    ]);
</script>

<Page back sketch="Playlist" ready={!!pl} error={page.error}>
  {#if pl}
    <Hero
      cover={pl.cover}
      title={pl.name}
      fallback="playlist"
      item={itemMark("playlist", id, { title: pl.name, line: pl.owner ?? "", cover: pl.cover }, !!pl.editable)}
      label={data ? (songs.length ? W.listCaption(songs.length, data.seconds) : "") : pl.songCount ? (pl.duration ? W.listCaption(pl.songCount, pl.duration) : W.songCount(pl.songCount)) : ""}
      playable={!!data && songs.length > 0}
      onplay={(s) => void player.play(songs, 0, s, "Playlist", id)}
    >
      {#snippet by()}{#if pl.comment}<span class="comment">{pl.comment}</span>{/if}{/snippet}
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="playlist" id={pl.id} on={!!pl.pinned} />{/if}
        {#if data}<PageTools tools={data.tools} {songs} />{/if}
        {#if data && pl.editable}<button class="glyph" aria-label={W.more} title={W.more} onclick={more}><Icon name="more-horiz" /></button>{/if}
      {/snippet}
    </Hero>
    {#if offered}<label class="field search-field filter"><Icon name="search" /><input type="search" placeholder={W.filter} aria-label={W.filter} bind:value={filter} /></label>{/if}
    {#if rows?.length === 0}<p class="empty">{W.nothingMatches(filter)}</p>{/if}
    <Reads ready={!!data || !!page.error} reading="Playlist" known />
    {#if data}
      {#if songs.length}<SongList {songs} {rows} kind="Playlist" {id} extra={edits} moved={pl.editable && !rows ? moved : undefined} />
      {:else}<p class="empty"><Icon name="music-note" />{W.noSongs}</p>{/if}
    {/if}
  {/if}
</Page>

<style>
  .filter {
    width: min(320px, 100%);
    margin: 0 0 14px;
  }
  .comment {
    font-size: 15px;
    font-weight: 400;
  }
</style>
