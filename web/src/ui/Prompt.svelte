<script lang="ts">
  // Asks for a line of text (a playlist's name): the shell's dialog card, its buttons words in ink.
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
    class="body"
    onsubmit={(e) => {
      e.preventDefault();
      p.done(value);
    }}
  >
    <h2>{p.title}</h2>
    <input class="field" bind:this={field} bind:value aria-label={p.title} />
    <div class="end">
      <button type="button" class="text-action" onclick={() => p.done(null)}>{W.cancel}</button>
      <button class="text-action" disabled={!value.trim()}>{W.done}</button>
    </div>
  </form>
</dialog>
