<script lang="ts">
  // The bare glyphs at the end of a page's list head, as the core offers them (`page_actions`): a browser
  // keeps no downloads, so what is left is Add to queue. `artist`: an artist's page queues everything theirs.
  import { player } from "../state/player.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { tools, songs = [], artist }: { tools: string[]; songs?: Song[]; artist?: string } = $props();
</script>

{#each tools as t (t)}
  {#if t === "AddToQueue"}
    <button class="glyph quiet" aria-label={W.addToQueue} title={W.addToQueue} onclick={() => void (artist ? player.queueWhole("artist", artist, false) : player.enqueue(songs, false))}><Icon name="playlist" /></button>
  {/if}
{/each}
