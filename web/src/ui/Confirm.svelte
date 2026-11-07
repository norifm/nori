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
  <div class="end">
    <button type="button" class="text-action" onclick={() => c.done(false)}>{W.cancel}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button type="button" class="text-action" id="confirm-yes" autofocus onclick={() => c.done(true)}>{c.action}</button>
  </div>
</dialog>

<style>
  .line {
    color: var(--ink2);
    white-space: pre-line;
  }
</style>
