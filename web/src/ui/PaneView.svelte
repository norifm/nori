<script lang="ts">
  // What a pane shows: the lyrics, the queue, other devices or the jam.
  import type { Pane } from "../state/ui.svelte";
  import Lyrics from "./Lyrics.svelte";
  import Queue from "./Queue.svelte";
  import Devices from "./Devices.svelte";
  import JamPanel from "./JamPanel.svelte";

  let { pane }: { pane: Pane } = $props();
</script>

{#if pane === "lyrics"}<Lyrics />
{:else if pane === "queue"}<Queue />
{:else if pane === "devices"}<Devices />
{:else}<div class="jam-pane"><JamPanel /></div>{/if}

<style>
  .jam-pane {
    overflow-y: auto;
    min-height: 0;
    padding: 4px 2px 16px;
  }
</style>
