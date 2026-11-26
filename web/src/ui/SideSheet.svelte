<script lang="ts">
  // The sheet beside the pages on a wide screen, as the desktop's: the devices, the lyrics or the queue
  // under their tabs. Docked, it is a third outlined pill beside the page (which makes room for it), from the
  // top bar's top to the player bar's foot and as far in from the window's edge, the page blurred inside; on
  // a narrower window it floats between the bars over the page. Its close button, which stays put as the
  // bars slide, or its tab pressed again closes it.
  import { cubicOut } from "svelte/easing";
  import { fly } from "svelte/transition";
  import { SIDE_MS, ui, type Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import PanelTabs from "./PanelTabs.svelte";

  let { pane }: { pane: Pane } = $props();
  // Apart from the first paint; the shell fetches it with the full player, once idle.
  const paneView = () => import("./PaneView.svelte");
  const still = () => document.documentElement.classList.contains("still");
</script>

<!-- Docked, it slides in from the window's edge as the page makes room; floating, it glides in and fades. -->
<aside class="side-sheet tinted" class:docked={ui.docks} class:chrome={ui.docks} id="side" transition:fly={{ x: ui.docks ? 336 : 24, opacity: ui.docks ? 1 : 0, duration: still() ? 0 : SIDE_MS, easing: cubicOut }}>
  <div class="tabs">
    <PanelTabs current={pane} pick={(p) => ui.show(p)} />
    <button class="glyph quiet close" id="side-close" title={W.close} aria-label={W.close} onclick={() => ui.show(pane)}><Icon name="xmark" /></button>
  </div>
  <div class="body {pane}">{#await paneView() then m}<m.default {pane} look="side" />{/await}</div>
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
    /* The tabs' corner and their inset, as the top bar's. */
    border-radius: 24px;
    border: 1px solid var(--outline);
    overflow: hidden;
  }
  .side-sheet:not(.docked) {
    background: var(--page);
  }
  /* The top bar's paper and outline (`chrome`), level with its top. */
  .side-sheet.docked {
    top: 12px;
    bottom: 12px;
  }
  .tabs {
    display: flex;
    align-items: flex-start;
    padding: 6px 8px 0;
    height: 52px;
    flex: none;
  }
  /* Less room on each tab's sides, for the close button's corner. */
  .tabs :global(.tab) {
    padding: 0 10px;
  }
  /* Inset as the tabs are, so its circle is concentric with the corner. */
  .close {
    width: 32px;
    height: 32px;
    margin: 2px 0 0 auto;
  }
  .close :global(.icon) {
    width: 20px;
    height: 20px;
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
