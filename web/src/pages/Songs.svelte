<script lang="ts">
  // The library's songs in the order picked, under a bar as the Albums page's: the order and which way it
  // runs, which opens the others (the core's `song_sorts`; picking the one in use turns it around, kept by
  // the core), and Favorites apart from them. Play plays them, Shuffle the core's random songs.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  type Sort = { name: string; reversed: boolean; way: string };
  let sorts = $state<string[]>([]);
  let sort = $state<Sort>();
  let favorites = $state(false);
  void Promise.all([ask<string>("song_sorts"), ask<string>("song_sort")]).then(([all, kept]) => {
    sorts = JSON.parse(all);
    sort = JSON.parse(kept);
  });
  // Read once the order is known, and again whenever it or the filter changes; not shown before.
  const page = reading("songs", () => {
    const [by, starred] = [sort, favorites];
    if (!by) return new Promise<Song[]>(() => {});
    return read<{ songs: Song[] }>({ kind: "songs", sort: by.name, reversed: by.reversed, favorites: starred }).then((p) => p.songs ?? []);
  });

  async function choose(name: string) {
    sort = JSON.parse(await ask<string>("pick_song_sort", name));
  }
</script>

<Page title={W.songs} back={!ui.wide} places ready={!!page.data} error={page.error}>
  {#snippet head()}
    <div class="actions">
      <button class="pill small" onclick={() => void player.play(page.data ?? [], 0, false, "Songs", "")}><Icon name="play-solid" />{W.play}</button>
      <button class="pill small quiet" onclick={() => void player.shuffleLibrary(false)}><Icon name="shuffle" />{W.shuffle}</button>
    </div>
  {/snippet}
  {#if sort}
    <div class="bar">
      <button
        class="chip on"
        aria-label="{W.sortBy} {W.songSorts[sort.name] ?? sort.name}"
        onclick={(e) => ui.drop(e, sorts.map((s) => ({ label: W.songSorts[s] ?? s, chosen: s === sort?.name, run: () => void choose(s) })))}
      >
        {W.songSorts[sort.name] ?? sort.name}
        {#if sort.way === "up"}<Icon name="arrow-up" />{:else if sort.way === "down"}<Icon name="arrow-down" />{/if}
        <Icon name="nav-arrow-down" />
      </button>
      <button class="chip filter" class:on={favorites} aria-pressed={favorites} onclick={() => (favorites = !favorites)}>
        <Icon name={favorites ? "sealed" : "seal"} />{W.favorites}
      </button>
    </div>
  {/if}
  {#if page.data?.length}<SongList songs={page.data} kind="Songs" />{:else}<p class="empty">{favorites ? W.noFavoriteSongs : W.noLibrarySongs}</p>{/if}
</Page>

<style>
  .actions {
    display: flex;
    gap: 8px;
  }
  /* As the Albums page's bar: the order raised, the filter quiet until it is on. */
  .bar {
    display: flex;
    gap: 8px;
    margin: 0 0 18px;
  }
  .bar .chip {
    gap: 6px;
  }
  .bar .chip :global(svg) {
    width: 15px;
    height: 15px;
  }
  .filter {
    border: var(--hairline) solid var(--hair);
  }
  .filter.on :global(svg) {
    color: var(--accent);
  }
</style>
