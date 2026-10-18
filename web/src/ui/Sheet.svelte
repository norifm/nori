<script lang="ts">
  // An action sheet over the page (a song's menu, a playlist's), as a dialog.
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const sheet = $derived(ui.sheet!);
  $effect(() => dialog?.showModal());
  const close = () => (ui.sheet = null);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="menu" bind:this={dialog} onclose={close} onclick={(e) => e.target === dialog && close()}>
  <p class="menu-title">{sheet.title}</p>
  {#each sheet.items as it, i (i)}
    <button
      class="menu-item"
      class:danger={it.danger}
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
    border: 0;
    padding: 6px;
    border-radius: 14px;
    background: var(--raised);
    color: var(--text);
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
    color: var(--dim);
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
    border-radius: 8px;
    text-align: left;
    font-size: 15px;
  }
  .menu-item:hover {
    background: var(--hover);
  }
  .menu-item :global(.icon) {
    color: var(--dim);
    width: 20px;
    height: 20px;
  }
  .danger {
    color: var(--accent);
  }
  .cancel {
    justify-content: center;
    color: var(--dim);
    border-top: 1px solid var(--hair);
    border-radius: 0 0 8px 8px;
    margin-top: 4px;
  }
  @media (max-width: 899px) {
    .menu {
      margin: auto auto 12px;
      width: calc(100vw - 16px);
    }
  }
</style>
