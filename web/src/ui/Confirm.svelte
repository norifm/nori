<script lang="ts">
  // A question before something that cannot be undone: what goes, and the button that does it.
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";

  let dialog = $state<HTMLDialogElement>();
  const c = $derived(ui.confirming!);
  $effect(() => dialog?.showModal());
</script>

<dialog class="card" id="confirm" bind:this={dialog} onclose={() => ui.confirming && c.done(false)}>
  <h2>{c.title}</h2>
  <p class="line">{c.line}</p>
  <div class="row">
    <button type="button" class="pill small quiet" onclick={() => c.done(false)}>{W.cancel}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button type="button" class="pill small" id="confirm-yes" autofocus onclick={() => c.done(true)}>{c.action}</button>
  </div>
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
    font-size: 20px;
    line-height: 26px;
  }
  .line {
    margin: 8px 0 0;
    color: var(--ink2);
    white-space: pre-line;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 24px;
  }
</style>
