<script lang="ts">
  // Use a sync server and its address, kept on Enter and when the field is left; paired, the kept copy's
  // switch. Your devices has it, and the first run's last page.
  import { sync } from "../state/sync.svelte";
  import { W } from "../words";

  const v = $derived(sync.view);
  let address = $state("");
  $effect(() => {
    address = v?.server ?? "";
  });
  async function keep() {
    if (address.trim() !== (v?.server ?? "")) await sync.server(address.trim());
  }
</script>

{#if v}
  <div class="group" id="sync-server">
    <label class="group-row" data-setting="sync">
      <span class="words"><span>{W.syncOn}</span>{#if v.paired && v.on}<span class="sub">{W.syncState(v.state, v.refused, v.retryMs)}</span>{/if}</span>
      <input type="checkbox" class="switch" aria-label={W.syncOn} checked={v.on} onchange={(e) => void sync.on(e.currentTarget.checked)} />
    </label>
    <label class="group-row stacked" class:off={!v.on}>
      <span class="name">{W.syncServer}</span>
      <input class="plain" type="url" id="sync-address" placeholder={W.syncServerOwn} disabled={!v.on} bind:value={address} onchange={() => void keep()} onkeydown={(e) => e.key === "Enter" && void keep()} />
    </label>
    {#if v.paired}
      <label class="group-row">
        <span class="words"><span>{W.syncKeepCopy}</span><span class="sub">{W.syncKeepCopyDetail}</span></span>
        <input type="checkbox" class="switch" id="keep-copy" aria-label={W.syncKeepCopy} checked={v.keepsCopy} onchange={(e) => void sync.act("sync_keep_copy", e.currentTarget.checked)} />
      </label>
    {/if}
  </div>
{/if}

<style>
  .stacked {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
  }
  .name {
    font-size: 13px;
    color: var(--ink2);
  }
  .off {
    opacity: 0.5;
  }
  input:not(.switch) {
    all: unset;
    font-size: 15px;
  }
</style>
