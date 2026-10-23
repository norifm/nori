<script lang="ts">
  // Every playlist, and a new one.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { newPlaylist } from "../state/menus";
  import type { Playlist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("playlists", () => read<{ playlists: Playlist[] }>({ kind: "playlists" }).then((p) => p.playlists ?? []));
</script>

<Page title={W.playlists} back ready={!!page.data} error={page.error}>
  {#snippet head()}<button class="pill small quiet" id="new-playlist" onclick={() => void newPlaylist()}><Icon name="plus" />{W.newPlaylist}</button>{/snippet}
  {#if page.data?.length}<Cards playlists={page.data} />{:else}<p class="empty"><Icon name="playlist" />{W.noPlaylists}</p>{/if}
</Page>
