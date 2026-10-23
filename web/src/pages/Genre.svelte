<script lang="ts">
  // A genre's songs, with Shuffle and Play over them.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  const name = $derived(route.arg);
  const page = reading("genre", () => read<{ songs: Song[] }>({ kind: "genre", name: route.arg }).then((p) => p.songs ?? []));
  const songs = $derived(page.data ?? []);
</script>

<Page title={name} back ready={!!page.data} error={page.error}>
  <div class="buttons">
    <button class="glyph" aria-label={W.shuffle} title={W.shuffle} onclick={() => void player.play(songs, 0, true, "Genre", name)}><Icon name="shuffle" /></button>
    <button class="pill" onclick={() => void player.play(songs, 0, false, "Genre", name)}><Icon name="play-solid" />{W.play}</button>
  </div>
  <SongList {songs} kind="Genre" id={name} />
</Page>

<style>
  .buttons {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 4px calc(var(--gap-tight) - var(--gutter)) 10px;
  }
  .pill {
    flex: 1;
  }
</style>
