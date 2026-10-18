<script lang="ts">
  // The panel beside the page on a wide screen, as the desktop app's.
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import PaneView from "./PaneView.svelte";

  const titles = { lyrics: W.lyrics, queue: W.queue, devices: W.playOn, jam: W.jam };
</script>

<aside class="panel" id="panel" aria-label={titles[ui.panel!]}>
  <header>
    <h2>{titles[ui.panel!]}</h2>
    <button class="round" aria-label={W.close} title={W.close} onclick={() => (ui.panel = null)}><Icon name="close" /></button>
  </header>
  <div class="body"><PaneView pane={ui.panel!} /></div>
</aside>

<style>
  .panel {
    background: var(--panel);
    border-left: 1px solid var(--hair);
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 14px 14px 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 8px;
  }
  h2 {
    font-size: 17px;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
