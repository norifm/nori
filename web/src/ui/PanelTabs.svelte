<script lang="ts">
  // The panel's tabs, as the desktop's: the devices (while remote control is on), the lyrics, the queue;
  // the one showing in the accent. Each keeps the room its bold name takes, so nothing moves as it lights.
  import { remote } from "../state/remote.svelte";
  import type { Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { current, pick }: { current: Pane | null; pick: (p: Pane) => void } = $props();
  const tabs = $derived(
    ([["devices", "computer", W.devices], ["lyrics", "quote-message", W.lyrics], ["queue", "playlist", W.queue]] as [Pane, string, string][]).filter(([p]) => p !== "devices" || remote.devices.on),
  );
</script>

<nav class="panel-tabs" aria-label={W.more}>
  {#each tabs as [p, icon, label] (p)}
    <button class="tab" class:on={current === p} aria-pressed={current === p} data-pane={p} onclick={() => pick(p)}>
      <Icon name={icon} /><span class="name" data-name={label}>{label}</span>
    </button>
  {/each}
</nav>

<style>
  .panel-tabs {
    display: flex;
    gap: 2px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 36px;
    padding: 0 12px;
    border-radius: 18px;
    color: var(--ink2);
    font-size: 14px;
    font-weight: 400;
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .tab.on {
    color: var(--accent);
    font-weight: 700;
  }
  /* The bold name's room, held by an unseen copy. */
  .name {
    display: inline-grid;
  }
  .name::after {
    content: attr(data-name);
    font-weight: 700;
    height: 0;
    visibility: hidden;
  }
</style>
