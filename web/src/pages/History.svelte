<script lang="ts">
  // What this browser played, newest first, a page at a time as the list nears its end; and a way to
  // forget it (the core's `history_page`, `history_clear`).
  import { ask, read } from "../rpc";
  import { library, reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  type Place = { startedMs: number; row: number };
  let more = $state<Song[]>([]);
  let next = $state<Place | null>(null);
  let asking = false;
  const page = reading("history", async () => {
    const p = await read<{ songs: Song[]; next: Place | null }>({ kind: "history", after: null });
    more = [];
    next = p.next;
    return p.songs ?? [];
  });
  const songs = $derived([...(page.data ?? []), ...more]);

  async function readOn() {
    if (!next || asking) return;
    asking = true;
    const p = await read<{ songs: Song[]; next: Place | null }>({ kind: "history", after: next });
    more = [...more, ...(p.songs ?? [])];
    next = p.next;
    asking = false;
  }

  let end = $state<HTMLElement>();
  $effect(() => {
    if (!end) return;
    const seen = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && void readOn(), { rootMargin: "800px" });
    seen.observe(end);
    return () => seen.disconnect();
  });

  async function clear() {
    await ask("history_clear");
    library.changed();
  }
</script>

<Page title={W.history} back ready={!!page.data} error={page.error}>
  {#snippet head()}
    <span class="actions">
      <a class="text-action" href={href("stats")}>{W.listeningStats}</a>
      {#if songs.length}<button class="text-action" onclick={() => void clear()}>{W.clearHistory}</button>{/if}
    </span>
  {/snippet}
  {#if songs.length}<SongList {songs} />{:else}<p class="empty">{W.nothingPlayed}</p>{/if}
  <div bind:this={end}></div>
</Page>

<style>
  .actions {
    display: flex;
    gap: 4px;
  }
  .actions a {
    display: inline-flex;
    align-items: center;
    color: var(--ink);
    text-decoration: none;
  }
  .actions a:hover {
    background: var(--hover);
  }
</style>
