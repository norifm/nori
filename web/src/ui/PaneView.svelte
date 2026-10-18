<script lang="ts">
  // What a pane shows: the lyrics, the queue or other devices.
  import type { Pane } from "../state/ui.svelte";
  import Lyrics from "./Lyrics.svelte";
  import Queue from "./Queue.svelte";
  import Devices from "./Devices.svelte";

  let { pane }: { pane: Pane } = $props();
</script>

{#if pane === "lyrics"}<Lyrics />
{:else if pane === "queue"}<Queue />
{:else}<Devices />{/if}
