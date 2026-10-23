<script lang="ts">
  // The library's songs, the first pages of them; Play plays them, Shuffle the core's random songs.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  const page = reading("songs", () => read<{ songs: Song[] }>({ kind: "songs", offset: 0, count: 300 }).then((p) => p.songs ?? []));
</script>

<Page title={W.songs} back ready={!!page.data} error={page.error}>
  {#snippet head()}
    <div class="actions">
      <button class="pill small" onclick={() => void player.play(page.data ?? [], 0, false, "Songs", "")}><Icon name="play-solid" />{W.play}</button>
      <button class="pill small quiet" onclick={() => void player.shuffleLibrary(false)}><Icon name="shuffle" />{W.shuffle}</button>
    </div>
  {/snippet}
  <SongList songs={page.data ?? []} kind="Songs" />
</Page>

<style>
  .actions {
    display: flex;
    gap: 8px;
  }
</style>
