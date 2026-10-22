<script lang="ts">
  // An action sheet over the page (a song's menu, a playlist's), as a dialog.
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const sheet = $derived(ui.sheet!);
  $effect(() => dialog?.showModal());
  const close = () => (ui.sheet = null);
  /** A click on the backdrop, not on the sheet's own padding (both target the dialog). */
  function backdrop(e: MouseEvent) {
    const r = dialog!.getBoundingClientRect();
    if (e.target === dialog && (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom)) close();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="menu" bind:this={dialog} onclose={close} onclick={backdrop}>
  <p class="menu-title">{sheet.title}</p>
  {#each sheet.items as it, i (i)}
    <button
      class="menu-item"
      class:danger={it.danger}
      disabled={it.disabled}
      onclick={() => {
        close();
        void it.run();
      }}>{#if it.icon}<Icon name={it.icon} />{/if}{it.label}</button
    >
  {/each}
  <button class="menu-item cancel" onclick={close}>{W.cancel}</button>
</dialog>

<style>
  .menu {
    padding: 8px;
    border-radius: 22px;
    background: var(--surface);
    color: var(--ink);
    border: 1px solid var(--hair);
    box-shadow: var(--shadow);
    width: min(320px, calc(100vw - 32px));
    max-height: 80vh;
  }
  .menu::backdrop {
    background: var(--scrim);
  }
  .menu-title {
    margin: 6px 12px 8px;
    font-size: 13px;
    color: var(--ink2);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    border-radius: 12px;
    text-align: left;
    font-size: 15px;
  }
  .menu-item:hover:enabled {
    background: var(--hover);
  }
  .menu-item:disabled {
    opacity: 0.38;
    cursor: default;
  }
  .menu-item :global(.icon) {
    color: var(--ink2);
    width: 20px;
    height: 20px;
  }
  .danger {
    font-weight: 600;
  }
  .cancel {
    justify-content: center;
    color: var(--ink2);
    border-top: 1px solid var(--hair);
    border-radius: 0 0 12px 12px;
    margin-top: 4px;
  }
  @media (max-width: 719px) {
    .menu {
      margin: auto auto 12px;
      width: calc(100vw - 16px);
    }
  }
</style>
