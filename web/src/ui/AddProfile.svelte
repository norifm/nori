<script lang="ts">
  // Adding a profile, on the shell's dialog card: first the provider, as Android's choice lists them (a
  // provider plugin can't run here); a server's form asks its address, user and password, signs in and
  // opens it (`login`); local files ask for a folder, add it and read it (state/folder).
  import { ask } from "../rpc";
  import { folder, pick } from "../state/folder.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { close, added, forProfile }: { close: () => void; added: () => void; forProfile?: string } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => dialog?.showModal());
  /** The provider chosen; null on the choice. */
  let kind = $state<"subsonic" | null>(null);
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
      const addr = url.trim().replace(/\/$/, "");
      if (forProfile) await ask("login_source", forProfile, addr, user.trim(), password);
      else await ask("login", addr, user.trim(), password);
      added();
    } catch (err) {
      failed = W.signInFailed(String((err as Error).message ?? err));
    }
    busy = false;
  }

  async function local() {
    const picked = await pick();
    if (!picked) return;
    busy = true;
    try {
      await folder.add(picked, forProfile);
      added();
    } catch (err) {
      failed = String((err as Error).message ?? err);
    }
    busy = false;
  }
</script>

<dialog class="menu card" id="add-profile" bind:this={dialog} onclose={close}>
  {#if kind === null}
    <div class="body">
      <h2>{W.providerChoose}</h2>
      <div class="group">
        <button class="group-row" data-kind="subsonic" onclick={() => (kind = "subsonic")}>
          <Icon name="server" /><span class="words"><span>{W.subsonic}</span><span class="sub">{W.subsonicAbout}</span></span><Icon name="nav-arrow-right" />
        </button>
        <button class="group-row" data-kind="local" disabled={busy} onclick={() => void local()}>
          <Icon name="folder" /><span class="words"><span>{W.local}</span><span class="sub">{busy ? W.readingFolder : W.localAbout}</span></span><Icon name="nav-arrow-right" />
        </button>
      </div>
      {#if failed}<p class="error">{failed}</p>{/if}
      <div class="end"><button type="button" class="text-action" onclick={close}>{W.cancel}</button></div>
    </div>
  {:else}
    <form class="body" onsubmit={add}>
      <h2>{W.subsonic}</h2>
      <input class="field" type="url" bind:value={url} aria-label={W.server} placeholder={W.server} autocomplete="url" data-field="url" />
      <input class="field" bind:value={user} aria-label={W.user} placeholder={W.user} autocomplete="username" data-field="user" />
      <input class="field" type="password" bind:value={password} aria-label={W.password} placeholder={W.password} autocomplete="current-password" data-field="password" />
      {#if failed}<p class="error">{failed}</p>{/if}
      <div class="end">
        <button type="button" class="text-action" onclick={() => ((kind = null), (failed = ""))}>{W.back}</button>
        <button class="text-action" id="add-profile-add" disabled={busy || !ready}>{busy ? W.connecting : W.add}</button>
      </div>
    </form>
  {/if}
</dialog>
