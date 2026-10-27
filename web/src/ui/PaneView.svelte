<script lang="ts">
  // What a pane shows: the lyrics, the queue or other devices; `look` where (the lyrics' sizes).
  import type { Pane } from "../state/ui.svelte";
  import Lyrics from "./Lyrics.svelte";
  import Queue from "./Queue.svelte";
  import Devices from "./Devices.svelte";

  let { pane, look = "phone" }: { pane: Pane; look?: "full" | "side" | "phone" } = $props();
</script>

{#if pane === "lyrics"}<Lyrics {look} />
{:else if pane === "queue"}<Queue />
{:else}<Devices />{/if}
