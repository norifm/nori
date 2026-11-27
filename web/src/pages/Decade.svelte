<script lang="ts">
  // A decade's songs, by year (the core's `decade_songs`).
  import { read } from "../rpc";
  import { addToPlaylist } from "../state/menus";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  let { start }: { start: number } = $props();
  const page = reading("decade", () => read<{ songs: Song[] }>({ kind: "decade", start }).then((p) => p.songs ?? []));
</script>

<Page title={W.decade(start)} back sketch="Songs" ready={!!page.data} error={page.error}>
  {#snippet head()}
    <div class="actions">
      <button class="pill small" onclick={() => void player.play(page.data ?? [], 0, false, "Songs", "")}><Icon name="play-solid" />{W.play}</button>
      <button class="pill small quiet" onclick={() => void player.play(page.data ?? [], 0, true, "Songs", "")}><Icon name="shuffle" />{W.shuffle}</button>
      <span class="grow"></span>
      <button class="glyph" aria-label={W.more} title={W.more} onclick={() => void addToPlaylist(page.data ?? [])}><Icon name="more-horiz" /></button>
    </div>
  {/snippet}
  <SongList songs={page.data ?? []} kind="Songs" id="" />
</Page>

<style>
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
</style>
