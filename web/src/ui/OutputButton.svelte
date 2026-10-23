<script lang="ts" module>
  /** A device's glyph by its kind, as Android draws them. */
  export const kindGlyph: Record<string, string> = { Phone: "smartphone-device", Desktop: "computer", Terminal: "terminal", Guest: "user" };
</script>

<script lang="ts">
  // Where the music plays: this browser, or another device of the account's, its glyph lit; a click opens
  // the devices. `named`: the device's name beside the glyph, both one button (the player's output line).
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { named = false }: { named?: boolean } = $props();
  const there = $derived(player.now?.deviceKind ?? null);
  const name = $derived(there ? (player.now?.device ?? "") : W.thisBrowser);
</script>

<button
  class="output"
  class:named
  class:lit={!!there}
  aria-label={W.playingOn(name)}
  title={W.playingOn(name)}
  data-pane="devices"
  onclick={(e) => {
    e.stopPropagation();
    ui.devices = true;
  }}><Icon name={there ? (kindGlyph[there] ?? "computer") : "computer"} />{#if named}<span class="clip">{name}</span>{/if}</button
>

<style>
  .output {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    min-width: 48px;
    height: 48px;
    border-radius: 999px;
    color: var(--ink2);
    flex: none;
  }
  .output.lit {
    color: var(--ink);
  }
  .output :global(.icon) {
    width: 22px;
    height: 22px;
  }
  .named {
    padding: 0 14px;
    max-width: 100%;
    font-size: 15px;
  }
  .output:hover {
    background: var(--hover);
  }
</style>
