<script lang="ts">
  // The Library's songs: its places, then the server's songs in its own order (a browser keeps no index
  // to sort them by), the first pages of them.
  import type { Snippet } from "svelte";
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import SongList from "../ui/SongList.svelte";

  let { places }: { places: Snippet } = $props();
  const page = reading("library", () => read<{ songs: Song[] }>({ kind: "songs", offset: 0, count: 300 }).then((p) => p.songs ?? []));
</script>

{@render places()}
{#if page.error}<p class="empty">{W.failed(page.error)}</p>
{:else if page.data && !page.data.length}<p class="empty"><Icon name="music-note" />{W.noSongs}</p>
{:else if page.data}<SongList songs={page.data} kind="Songs" />{/if}
