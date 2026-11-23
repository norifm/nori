<script lang="ts">
  // Adding a profile, on the shell's dialog card: a browser plays from OpenSubsonic servers only (a provider
  // plugin can't run here), so it asks the server's address, user and password, signs in and opens it
  // (`login`).
  import { ask } from "../rpc";
  import { W } from "../words";

  let { close, added }: { close: () => void; added: () => void } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => dialog?.showModal());
  let url = $state("");
  let user = $state("");
  let password = $state("");
  let busy = $state(false);
  let failed = $state("");
  const ready = $derived(!!url.trim() && !!user.trim());

  async function add(e: SubmitEvent) {
    e.preventDefault();
    if (!ready || busy) return;
    busy = true;
    failed = "";
    try {
      await ask("login", url.trim().replace(/\/$/, ""), user.trim(), password);
      added();
    } catch (err) {
      failed = W.signInFailed(String((err as Error).message ?? err));
    }
    busy = false;
  }
</script>

<dialog class="menu card" id="add-profile" bind:this={dialog} onclose={close}>
  <form class="body" onsubmit={add}>
    <h2>{W.subsonic}</h2>
    <input class="field" type="url" bind:value={url} aria-label={W.server} placeholder={W.server} autocomplete="url" data-field="url" />
    <input class="field" bind:value={user} aria-label={W.user} placeholder={W.user} autocomplete="username" data-field="user" />
    <input class="field" type="password" bind:value={password} aria-label={W.password} placeholder={W.password} autocomplete="current-password" data-field="password" />
    {#if failed}<p class="error">{failed}</p>{/if}
    <div class="end">
      <button type="button" class="text-action" onclick={close}>{W.cancel}</button>
      <button class="text-action" id="add-profile-add" disabled={busy || !ready}>{busy ? W.connecting : W.add}</button>
    </div>
  </form>
</dialog>
