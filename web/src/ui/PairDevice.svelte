<script lang="ts">
  // Pair a device, as Android's and the desktop's: in no group, this browser's code shows at once (the QR
  // code, its three words, how long it works, Copy link, a new one once it ran out); with profiles to give,
  // it takes another device's pasted link. The other device's yes comes here as a question: who and how many
  // sign-ins, the three words, the group it leaves and, at a first pairing, the encrypted copy. A browser has
  // no local network, so no device waiting nearby is listed. Closing stops a pairing that did not finish.
  import { ask } from "../rpc";
  import { sync } from "../state/sync.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const v = $derived(sync.view);
  let side = $state<"Show" | "Take">("Show");
  let link = $state("");
  let keep = $state(true);
  let copied = $state(false);
  /** The code shown last: it stays while the new device waits on after a key that did not fit. */
  let shown = $state<{ code: string; words: string[] } | null>(null);
  let qr = $state<{ size: number; dark: string } | null>(null);

  /** Opens on the side the core says, taking up a pairing under way. */
  async function open() {
    const now = await sync.read();
    side = now.opens;
    if (now.pairing === "Code" || now.pairing === "Confirm" || now.pairing === "Starting") return;
    await sync.act(side === "Show" ? "sync_code" : "sync_cancel");
  }
  void open();

  async function choose(to: "Show" | "Take") {
    if (to === side) return;
    side = to;
    shown = null;
    link = "";
    await sync.act(to === "Show" ? "sync_code" : "sync_cancel");
  }

  async function take(e: SubmitEvent) {
    e.preventDefault();
    if (!link.trim()) return;
    keep = true;
    await sync.act("sync_take", link.trim());
  }

  function close() {
    sync.pairing = false;
    void sync.act("sync_cancel");
  }

  $effect(() => {
    if (v?.pairing === "Code" && v.code && v.words) shown = { code: v.code, words: v.words };
  });
  $effect(() => {
    const code = shown?.code;
    if (code) void ask<string>("qr", code).then((q) => (qr = JSON.parse(q)));
  });
  // Over: the new device says it is paired, the other hears the group's notice.
  $effect(() => {
    if (v?.pairing === "Paired") ui.say(W.syncPaired);
    if (v?.pairing === "Paired" || v?.pairing === "Sent") sync.pairing = false;
  });

  /** A refusal the new device goes on waiting after. */
  const waitsOn = $derived(side === "Show" && v?.pairing === "Refused" && (v.reason === "Tampered" || v.reason === "Spent") && !!shown);
  const refused = $derived(v?.pairing === "Refused" ? W.pairingRefused(v.reason ?? "", side === "Show", v.serverRefused) : "");
  /** One square per dark module, as one path. */
  const path = (q: { size: number; dark: string }) => {
    let d = "";
    for (let i = 0; i < q.dark.length; i++) if (q.dark[i] === "1") d += `M${i % q.size} ${Math.floor(i / q.size)}h1v1h-1z`;
    return d;
  };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet tinted" id="pair-device" role="dialog" aria-modal="true" aria-label={W.syncPair}>
    <div class="body">
      <h2>{W.syncPair}</h2>
      {#if v?.pairing === "Confirm" && v.confirm}
        {@const c = v.confirm}
        <h3 id="pair-confirm-title">{W.syncConfirmTitle(c.name, c.profiles)}</h3>
        {#if c.words}
          <p class="words" id="pair-words">{c.words.join("  ")}</p>
          <p class="sub">{W.syncConfirmWords}</p>
        {/if}
        {#if c.leaves !== null}<p>{W.syncConfirmLeaves(c.name, c.leaves)}</p>{/if}
        {#if c.asksKeepCopy}
          <label class="keep" id="keep-copy">
            <span class="words-col"><span>{W.syncKeepCopyAsk}</span><span class="sub">{W.syncKeepCopyAskDetail}</span></span>
            <input type="checkbox" class="switch" aria-label={W.syncKeepCopyAsk} bind:checked={keep} />
          </label>
        {/if}
        <div class="buttons">
          <button type="button" class="pill small quiet" onclick={close}>{W.cancel}</button>
          <button type="button" class="pill small" id="pair-confirm" onclick={() => void sync.act("sync_confirm", keep)}>{W.syncPairAction}</button>
        </div>
      {:else}
        {#if v?.gives}
          <div class="sides" role="tablist">
            <button role="tab" class="chip" class:on={side === "Show"} aria-selected={side === "Show"} id="show-code" onclick={() => void choose("Show")}>{W.syncShowCode}</button>
            <button role="tab" class="chip" class:on={side === "Take"} aria-selected={side === "Take"} id="paste-link" onclick={() => void choose("Take")}>{W.syncPaste}</button>
          </div>
        {/if}
        {#if side === "Show"}
          {#if (v?.pairing === "Code" || waitsOn) && shown}
            {#if qr}
              <svg class="qr" id="pair-code" viewBox="-2 -2 {qr.size + 4} {qr.size + 4}" role="img" aria-label={W.syncLink}><rect x="-2" y="-2" width={qr.size + 4} height={qr.size + 4} fill="white" /><path d={path(qr)} fill="black" /></svg>
            {/if}
            <p class="words" id="pair-words">{shown.words.join("  ")}</p>
            <p class="sub">{W.syncCodeWords}</p>
            {#if waitsOn}<p class="error">{refused}</p>{/if}
            <p class="sub">{W.syncCodeLasts}</p>
            <button class="pill" id="copy-link" onclick={() => void navigator.clipboard?.writeText(shown?.code ?? "").then(() => (copied = true))}>{copied ? W.copied : W.copyLink}</button>
          {:else if v?.pairing === "Refused"}
            <p class="error" id="pair-refused">{refused}</p>
            <button class="pill" id="new-code" onclick={() => ((shown = null), void sync.act("sync_code"))}><Icon name="refresh" />{W.syncNewCode}</button>
          {:else}
            <p class="sub">{W.syncStarting}</p>
          {/if}
        {:else}
          <form class="form" onsubmit={take}>
            <label class="field">
              <Icon name="smartphone-device" />
              <!-- svelte-ignore a11y_autofocus -->
              <input id="pair-link" bind:value={link} placeholder={W.syncLink} aria-label={W.syncLink} autocomplete="off" autofocus />
            </label>
            {#if v?.pairing === "Starting"}<p class="sub">{W.syncStarting}</p>{/if}
            {#if refused}<p class="error" id="pair-refused">{refused}</p>{/if}
            <div class="buttons">
              <button type="button" class="pill small quiet" onclick={close}>{W.cancel}</button>
              <button class="pill small" id="pair-take" disabled={!link.trim()}>{W.syncPairAction}</button>
            </div>
          </form>
        {/if}
      {/if}
    </div>
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
    padding: 64px 16px 0;
    overflow-y: auto;
  }
  .sheet {
    width: min(420px, 100%);
  }
  .body {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
    padding: 20px 16px 16px;
    border-radius: 30px;
    background: var(--page);
    border: 1px solid var(--hair);
    color: var(--ink);
  }
  h2 {
    font: 700 17px/22px var(--font);
    text-align: center;
  }
  h3 {
    font: 600 15px/20px var(--font);
  }
  p {
    margin: 0;
  }
  .sub {
    font-size: 13px;
    color: var(--ink2);
    text-align: center;
  }
  .words {
    font: 600 20px/26px var(--font);
    text-align: center;
  }
  .sides {
    display: flex;
    justify-content: center;
    gap: 8px;
  }
  .qr {
    align-self: center;
    width: min(220px, 70vw);
    height: auto;
    border-radius: 8px;
  }
  .keep {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .words-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 14px;
  }
  .words-col .sub {
    text-align: left;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
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
  input:not(.switch) {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }
  .error {
    text-align: center;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
