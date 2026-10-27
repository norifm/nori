<script lang="ts">
  // The sheet beside the pages on a wide screen, as the desktop's: the devices, the lyrics or the queue
  // under their tabs, floating over the page (which moves for nothing). Its tab pressed again closes it.
  import { cubicOut } from "svelte/easing";
  import { fly } from "svelte/transition";
  import { ui, type Pane } from "../state/ui.svelte";
  import PaneView from "./PaneView.svelte";
  import PanelTabs from "./PanelTabs.svelte";

  let { pane }: { pane: Pane } = $props();
  const still = () => document.documentElement.classList.contains("still");
</script>

<aside class="side-sheet tinted" id="side" transition:fly={{ x: 24, duration: still() ? 0 : 240, easing: cubicOut }}>
  <div class="tabs"><PanelTabs current={pane} pick={(p) => ui.show(p)} /></div>
  <div class="body {pane}"><PaneView {pane} look="side" /></div>
</aside>

<style>
  .side-sheet {
    position: fixed;
    right: 16px;
    top: 68px;
    bottom: 84px;
    width: 320px;
    z-index: 25;
    display: flex;
    flex-direction: column;
    border-radius: 22px;
    border: 1px solid var(--outline);
    background: var(--page);
    overflow: hidden;
  }
  .tabs {
    padding: 10px 10px 0;
    height: 56px;
    flex: none;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .body.queue,
  .body.devices {
    padding: 0 12px;
  }
</style>
