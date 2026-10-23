<script lang="ts">
  // The devices and the jam as a sheet over whatever is open, as Android's: opaque paper on the surface
  // with a hairline edge, from the bottom on a phone, in the middle on a wide screen.
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Devices from "./Devices.svelte";

  let dialog = $state<HTMLDialogElement>();
  $effect(() => dialog?.showModal());
  const close = () => (ui.devices = false);
  function backdrop(e: MouseEvent) {
    const r = dialog!.getBoundingClientRect();
    if (e.target === dialog && (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom)) close();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="sheet" class:wide={ui.wide} bind:this={dialog} onclose={close} onclick={backdrop} aria-label={W.playOn}>
  {#if !ui.wide}<span class="handle"></span>{/if}
  <h2>{W.playOn}</h2>
  <Devices />
</dialog>

<style>
  .sheet {
    margin: auto auto 0;
    width: 100%;
    max-width: 560px;
    max-height: 86vh;
    padding: 8px 16px calc(16px + env(safe-area-inset-bottom));
    border: 0;
    border-top: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet) var(--r-sheet) 0 0;
    background: var(--surface);
    color: var(--ink);
  }
  .sheet.wide {
    margin: auto;
    width: min(440px, calc(100vw - 32px));
    padding: 16px;
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet);
  }
  .sheet::backdrop {
    background: var(--scrim);
  }
  .handle {
    display: block;
    width: 36px;
    height: 4px;
    border-radius: 2px;
    margin: 0 auto 12px;
    background: color-mix(in srgb, var(--ink) 20%, transparent);
  }
  h2 {
    padding: 4px 4px 12px;
    font-size: 19px;
    line-height: 24px;
  }
</style>
