<script lang="ts">
  // Joining someone's jam, as the desktop's sheet: the link they sent in one field; Enter or Join.
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { close }: { close: () => void } = $props();
  let link = $state("");
  let busy = $state(false);
  let said = $state("");

  async function join(e: SubmitEvent) {
    e.preventDefault();
    if (!link.trim() || busy) return;
    busy = true;
    said = "";
    try {
      const r = await remote.join(link, "");
      if (r === "joined") close();
      else said = r === "own" ? W.jamOwn : W.jamEnded;
    } catch (err) {
      said = W.failed(String((err as Error).message ?? err));
    }
    busy = false;
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet tinted" role="dialog" aria-modal="true" aria-label={W.jamJoinTitle}>
  <form class="form" id="jam-join" onsubmit={join}>
    <Icon name="community" />
    <h2>{W.jamJoinTitle}</h2>
    <p class="lead">{W.jamJoinLead}</p>
    <label class="field">
      <Icon name="community" />
      <!-- svelte-ignore a11y_autofocus -->
      <input id="jam-link" bind:value={link} placeholder={W.jamJoinField} aria-label={W.jamLink} autocomplete="off" autofocus />
    </label>
    {#if said}<p class="error">{said}</p>{/if}
    <div class="buttons">
      <button type="button" class="pill small quiet" onclick={close}>{W.cancel}</button>
      <button class="pill small" disabled={busy || !link.trim()}>{busy ? W.joining : W.join}</button>
    </div>
  </form>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 70;
    background: color-mix(in srgb, var(--page) 60%, transparent);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 80px 16px 0;
  }
  .sheet {
    width: min(420px, 100%);
  }
  .form {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
    padding: 16px;
    border-radius: 30px;
    background: var(--page);
    border: 1px solid var(--hair);
    color: var(--ink);
  }
  .form > :global(.icon) {
    width: 30px;
    height: 30px;
    align-self: center;
  }
  h2 {
    font: 700 17px/22px var(--font);
    text-align: center;
  }
  .lead {
    margin: 0;
    font-size: 12px;
    color: var(--ink2);
    text-align: center;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 10px;
    border-radius: 8px;
    border: 1px solid var(--hair);
    background: var(--surface);
  }
  .field:focus-within {
    border-color: var(--accent);
  }
  .field :global(.icon) {
    width: 18px;
    height: 18px;
    color: var(--ink2);
  }
  input {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }
  .error {
    margin: 0;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
