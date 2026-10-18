<script lang="ts">
  // Asks for a line of text (a playlist's name).
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";

  let dialog = $state<HTMLDialogElement>();
  let field = $state<HTMLInputElement>();
  const p = $derived(ui.prompt!);
  let value = $state(ui.prompt?.value ?? "");
  $effect(() => {
    dialog?.showModal();
    field?.select();
  });
</script>

<dialog class="menu" bind:this={dialog} onclose={() => ui.prompt && p.done(null)}>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      p.done(value);
    }}
  >
    <p class="menu-title">{p.title}</p>
    <input class="field" bind:this={field} bind:value aria-label={p.title} />
    <div class="row">
      <button type="button" class="pill" onclick={() => p.done(null)}>{W.cancel}</button>
      <button class="pill solid">{W.done}</button>
    </div>
  </form>
</dialog>

<style>
  .menu {
    border: 0;
    padding: 16px;
    border-radius: 14px;
    background: var(--raised);
    color: var(--text);
    box-shadow: var(--shadow);
    width: min(360px, calc(100vw - 32px));
  }
  .menu::backdrop {
    background: var(--scrim);
  }
  .menu-title {
    margin: 0 0 10px;
    font-weight: 600;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }
</style>
