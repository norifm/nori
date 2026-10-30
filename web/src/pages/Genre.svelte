<script lang="ts">
  // A genre's songs.
  import { read } from "../rpc";
  import { addToPlaylist } from "../state/menus";
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
      <span class="grow"></span>
      <!-- Its songs into a playlist, a new one or another, as an album's ⋯ offers. -->
      <button class="glyph" aria-label={W.more} title={W.more} onclick={() => void addToPlaylist(page.data ?? [])}><Icon name="more-horiz" /></button>
    </div>
  {/snippet}
  <SongList songs={page.data ?? []} kind="Genre" id={name} />
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
