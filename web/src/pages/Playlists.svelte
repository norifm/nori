<script lang="ts">
  // Every playlist, and a new one.
  import Icon from "../ui/Icon.svelte";
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { newPlaylist } from "../state/menus";
  import { href } from "../state/route.svelte";
  import type { Playlist } from "../state/types";
  import { W } from "../words";
  import Card from "../ui/Card.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("playlists", () => read<{ playlists: Playlist[] }>({ kind: "playlists" }).then((p) => p.playlists ?? []));
</script>

<Page title={W.playlists} back ready={!!page.data} error={page.error}>
  {#snippet head()}<button class="pill small" id="new-playlist" onclick={() => void newPlaylist()}>{W.newPlaylist}</button>{/snippet}
  {#if page.data?.length}
    <div class="grid">
      {#each page.data as p (p.id)}<Card href={href("playlist", p.id)} cover={p.cover} title={p.name} sub={W.songCount(p.songCount)} fallback="playlist" />{/each}
    </div>
  {:else}
    <p class="empty"><Icon name="playlist" />{W.empty}</p>
  {/if}
</Page>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
    gap: 22px 16px;
  }
  @media (min-width: 1600px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(208px, 1fr));
    }
  }
  @media (max-width: 719px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
      gap: 18px 14px;
    }
  }
</style>
