<script lang="ts">
  // Asks for a line of text (a playlist's name): a card on the surface with a hairline edge, its buttons
  // words in ink.
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

<dialog class="menu card" bind:this={dialog} onclose={() => ui.prompt && p.done(null)}>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      p.done(value);
    }}
  >
    <h2>{p.title}</h2>
    <input class="field" bind:this={field} bind:value aria-label={p.title} />
    <div class="row">
      <button type="button" class="text-action" onclick={() => p.done(null)}>{W.cancel}</button>
      <button class="text-action" disabled={!value.trim()}>{W.done}</button>
    </div>
  </form>
</dialog>

<style>
  .card {
    padding: 24px;
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet);
    background: var(--surface);
    color: var(--ink);
    width: min(400px, calc(100vw - 32px));
  }
  .card::backdrop {
    background: var(--scrim);
  }
  h2 {
    margin-bottom: 16px;
    font-size: 20px;
    line-height: 26px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 24px;
  }
</style>
