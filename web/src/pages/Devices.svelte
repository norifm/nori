<script lang="ts">
  // Your devices, as Android's: this browser and its group's others with when each was last seen, Pair a
  // device, the sync server's switch and address, the kept copy, and Reset pairing, asked first. Not paired,
  // one line says so over Pair a device. A browser runs no plugins, so it lists none that profiles need.
  import { ask } from "../rpc";
  import { sync } from "../state/sync.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SyncServer from "../ui/SyncServer.svelte";

  void sync.read();
  const v = $derived(sync.view);
  const now = $derived.by(() => (void v, Date.now()));

  /** Reset pairing, asked first, naming the profiles whose sign-ins the other devices hold. */
  async function askReset() {
    const n = await ask<string | null>("sync_reset_notice");
    if (!n) return;
    const { signedIn } = JSON.parse(n) as { others: number; signedIn: string[] };
    if (await ui.confirm(W.syncResetTitle, W.syncResetText(signedIn), W.syncResetConfirm)) await sync.act("sync_reset");
  }
</script>

<Page title={W.yourDevices} back narrow ready={!!v}>
  {#if v}
    {#if !v.paired}
      <p class="alone" id="not-paired">{W.syncNotPaired}</p>
      <button class="pill" id="pair-device" disabled={!v.on} onclick={() => (sync.pairing = true)}><Icon name="plus" />{W.syncPair}</button>
    {:else}
      <div class="group-label">{W.syncSectionDevices}</div>
      <div class="group" id="your-devices">
        <div class="group-row"><Icon name="computer" /><span class="words"><span>{W.syncThisDevice}</span><span class="sub">{W.syncState(v.state, v.refused, v.retryMs)}</span></span></div>
        {#each v.devices as d (d.name + d.lastMs)}
          <div class="group-row"><Icon name="smartphone-device" /><span class="words"><span>{d.name}</span><span class="sub">{W.syncSeen(d.lastMs, d.online, now)}</span></span></div>
        {/each}
        <button class="group-row" id="pair-device" disabled={!v.on} onclick={() => (sync.pairing = true)}><Icon name="plus" /><span class="words">{W.syncPair}</span></button>
      </div>
    {/if}
    <div class="group-label">{W.syncSectionServer}</div>
    <SyncServer />
    {#if v.paired}
      <div class="group reset">
        <button class="group-row" id="reset-pairing" onclick={() => void askReset()}><span class="words">{W.syncReset}</span></button>
      </div>
    {/if}
  {/if}
</Page>

<style>
  .alone {
    margin: 8px 0 0;
    color: var(--ink2);
  }
  .pill {
    width: 100%;
    margin: 12px 0;
  }
  .reset {
    margin-top: 8px;
  }
</style>
