<script lang="ts">
  // Adding a scrobbling account to the profile signed in, on the shell's dialog card: the services to
  // pick from, then the picked one's fields (`scrobbler_kinds`), its address folded under Advanced. A
  // service signing in on its page opens it for the user to allow nori, then Done signs in; the core
  // does both (`Client::scrobbler_approval`, `Client::sign_in_scrobbler`).
  import { ask } from "../rpc";
  import type { ScrobblerField, ScrobblerKindForm } from "../state/types";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { kinds, close, added }: { kinds: ScrobblerKindForm[]; close: () => void; added: () => void } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => dialog?.showModal());
  let asks = $state<ScrobblerKindForm | null>(null);
  let typed = $state<Partial<Record<ScrobblerField, string>>>({});
  let advanced = $state(false);
  /** The token allowed on the service's page, once it was opened. */
  let approved = $state("");
  let busy = $state(false);
  let said = $state("");
  const ready = $derived(!!asks && asks.fields.every((f) => !f.required || !!typed[f.id]?.trim()));
  const browser = $derived(asks?.signIn === "browser");
  const hidden: ScrobblerField[] = ["token", "secret", "password"];
  const autocomplete: Partial<Record<ScrobblerField, AutoFill>> = { user: "username", password: "current-password" };

  function pick(k: ScrobblerKindForm) {
    asks = k;
    typed = {};
    advanced = false;
    approved = "";
    said = "";
  }

  async function add(e: SubmitEvent) {
    e.preventDefault();
    if (!asks || !ready || busy) return;
    busy = true;
    said = "";
    const fields = JSON.stringify(typed);
    // Opened while the click still counts as the user's, or the browser blocks it; pointed at the page once known.
    const tab = browser && !approved ? window.open("", "_blank") : null;
    try {
      if (browser && !approved) {
        const a = JSON.parse(await ask<string>("scrobbler_approval", asks.kind, fields)) as { token: string; page: string };
        approved = a.token;
        if (tab) tab.location.href = a.page;
        else window.open(a.page, "_blank");
      } else {
        await ask("scrobbler_add", asks.kind, fields, approved);
        added();
      }
    } catch (err) {
      tab?.close();
      said = W.failed(String((err as Error).message ?? err));
    }
    busy = false;
  }
</script>

{#snippet field(f: ScrobblerKindForm["fields"][number])}
  <input
    class="field"
    type={hidden.includes(f.id) ? "password" : f.id === "address" ? "url" : "text"}
    bind:value={typed[f.id]}
    placeholder={W.scrobblerField[f.id]}
    autocomplete={autocomplete[f.id] ?? "off"}
  />
{/snippet}

<dialog class="menu card" id="add-scrobbler" bind:this={dialog} onclose={close}>
  {#if !asks}
    <form class="body" onsubmit={(e) => e.preventDefault()}>
      <h2>{W.addScrobblerTitle}</h2>
      <div class="group">
        {#each kinds as k (k.kind)}
          <button type="button" class="group-row" data-kind={k.kind} onclick={() => pick(k)}>
            <span class="words"><span>{W.scrobbler(k.kind)}</span>{#if W.scrobblerCost(k.kind)}<span class="sub">{W.scrobblerCost(k.kind)}</span>{/if}</span>
            <Icon name="nav-arrow-right" />
          </button>
        {/each}
      </div>
      <div class="end"><button type="button" class="text-action" onclick={close}>{W.cancel}</button></div>
    </form>
  {:else}
    <form class="body" onsubmit={add}>
      <h2>{W.addScrobbler(asks.kind)}</h2>
      {#each asks.fields.filter((f) => !f.advanced) as f (f.id)}{@render field(f)}{/each}
      {#if asks.fields.some((f) => f.id === "password")}<p class="sub">{W.scrobblerPasswordNote}</p>{/if}
      {#if asks.help || asks.fields.some((f) => f.advanced)}
        <div class="group">
          {#if asks.help}
            <a class="group-row" href={asks.help} target="_blank" rel="noreferrer"><span class="words">{W.scrobblerHelp(asks.kind)}</span><Icon name="arrow-up" /></a>
          {/if}
          {#if asks.fields.some((f) => f.advanced)}
            <button type="button" class="group-row" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>
              <span class="words">{W.advanced}</span><Icon name={advanced ? "nav-arrow-up" : "nav-arrow-down"} />
            </button>
          {/if}
        </div>
      {/if}
      {#if advanced}
        {#each asks.fields.filter((f) => f.advanced) as f (f.id)}{@render field(f)}{/each}
        <p class="sub">{W.scrobblerAddressNote(asks.kind)}</p>
      {/if}
      {#if browser && approved}<p class="sub" id="scrobbler-allow">{W.scrobblerAllow(asks.kind)}</p>{/if}
      {#if said}<p class="error">{said}</p>{/if}
      <div class="end">
        <button type="button" class="text-action" onclick={() => (asks = null)}>{W.back}</button>
        <button class="text-action" disabled={busy || !ready}>
          {busy ? W.scrobblerAdding : !browser ? W.scrobblerAdd : approved ? W.done : W.scrobblerContinue(asks.kind)}
        </button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  /* The page opens elsewhere: the arrow leans out. */
  a.group-row > :global(.icon) {
    transform: rotate(45deg);
  }
</style>
