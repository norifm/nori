<script lang="ts">
  // A genre's songs.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  let { name }: { name: string } = $props();
  const page = reading("genre", () => read<{ songs: Song[] }>({ kind: "genre", name }).then((p) => p.songs ?? []));
</script>

<Page title={name} back ready={!!page.data} error={page.error}>
  {#snippet head()}
    <div class="actions">
      <button class="pill small" onclick={() => void player.play(page.data ?? [], 0, false, "Genre", name)}><Icon name="play-solid" />{W.play}</button>
      <button class="pill small quiet" onclick={() => void player.play(page.data ?? [], 0, true, "Genre", name)}><Icon name="shuffle" />{W.shuffle}</button>
    </div>
  {/snippet}
  <SongList songs={page.data ?? []} kind="Genre" id={name} />
</Page>

<style>
  .actions {
    display: flex;
    gap: 8px;
  }
</style>
